use axum::{
    Router,
    extract::{Json, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
// async_trait not required anymore
use std::time::{SystemTime, UNIX_EPOCH};

use crate::service::Service;

use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::http::HeaderMap;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, TokenData, Validation, decode, encode};
use rand::rngs::OsRng;

#[derive(Deserialize)]
pub struct LoginRequest {
    pub uin: u32,
    pub password: String,
}

#[derive(Deserialize)]
pub struct ResetPasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

#[derive(Serialize, Deserialize)]
struct Claims {
    sub: i64,
    exp: usize,
}

fn jwt_secret() -> String {
    std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev-secret".to_string())
}

fn issue_jwt(member_id: i64) -> Result<String, jsonwebtoken::errors::Error> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize;
    let claims = Claims {
        sub: member_id,
        exp: now + 3600,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(jwt_secret().as_bytes()),
    )
}

fn verify_jwt(token: &str) -> Result<TokenData<Claims>, jsonwebtoken::errors::Error> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(jwt_secret().as_bytes()),
        &Validation::default(),
    )
}

pub fn routes(svc: Service) -> Router {
    Router::new()
        .route("/login", post(login_handler))
        .route("/logout", post(logout_handler))
        .route("/reset_password", post(reset_password_handler))
        .route(
            "/daka/records",
            get(|| async { (StatusCode::OK, Json(serde_json::json!([]))) }),
        )
        .route("/daka/records", get(daka_records_handler))
        .route("/daka/daka", post(daka_create_handler))
        .route("/daka/daka", delete(daka_delete_handler))
        .with_state(svc)
}

#[derive(Deserialize)]
struct DakaPayload {
    qq_uin: u32,
}

// AuthUser unused (cookie-based auth)

fn extract_token_from_cookies(headers: &HeaderMap) -> Result<String, (StatusCode, &'static str)> {
    // look for cookie header and parse auth_token
    if let Some(cookie_hdr) = headers.get("cookie") {
        if let Ok(cookie_str) = cookie_hdr.to_str() {
            for pair in cookie_str.split(';') {
                let pair = pair.trim();
                if let Some(val) = pair.strip_prefix("auth_token=") {
                    return Ok(val.to_string());
                }
            }
        }
    }
    Err((StatusCode::UNAUTHORIZED, "missing token"))
}

async fn daka_records_handler(State(svc): State<Service>, headers: HeaderMap) -> impl IntoResponse {
    let token = match extract_token_from_cookies(&headers) {
        Ok(t) => t,
        Err(e) => return e.into_response(),
    };
    let data = match verify_jwt(&token) {
        Ok(d) => d,
        Err(_) => return (StatusCode::UNAUTHORIZED, "invalid token").into_response(),
    };
    let _member_id = data.claims.sub;
    let report = svc.build_daily_report();
    (StatusCode::OK, Json(serde_json::json!({"records": report}))).into_response()
}

async fn daka_create_handler(
    State(svc): State<Service>,
    headers: HeaderMap,
    Json(payload): Json<DakaPayload>,
) -> impl IntoResponse {
    let token = match extract_token_from_cookies(&headers) {
        Ok(t) => t,
        Err(e) => return e.into_response(),
    };
    if verify_jwt(&token).is_err() {
        return (StatusCode::UNAUTHORIZED, "invalid token").into_response();
    }
    // Do NOT upsert. Find existing member by qq_uin.
    match svc.find_member_by_uin(payload.qq_uin) {
        Some((member_id, _pw)) => {
            let resp = svc.handle_打卡(member_id, "");
            (
                StatusCode::OK,
                Json(serde_json::json!({"ok": resp.ok, "message": resp.message})),
            )
                .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok": false, "message": "member not found"})),
        )
            .into_response(),
    }
}

async fn daka_delete_handler(
    State(svc): State<Service>,
    headers: HeaderMap,
    Json(payload): Json<DakaPayload>,
) -> impl IntoResponse {
    let token = match extract_token_from_cookies(&headers) {
        Ok(t) => t,
        Err(e) => return e.into_response(),
    };
    if verify_jwt(&token).is_err() {
        return (StatusCode::UNAUTHORIZED, "invalid token").into_response();
    }
    match svc.find_member_by_uin(payload.qq_uin) {
        Some((member_id, _)) => {
            let resp = svc.handle_我没打卡(member_id, "");
            (
                StatusCode::OK,
                Json(serde_json::json!({"ok": resp.ok, "message": resp.message})),
            )
                .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok": false, "message": "member not found"})),
        )
            .into_response(),
    }
}

async fn login_handler(
    State(svc): State<Service>,
    Json(payload): Json<LoginRequest>,
) -> impl IntoResponse {
    // find member by uin
    match svc.find_member_by_uin(payload.uin) {
        Some((member_id, pw_hash)) => {
            // verify password
            let parsed = PasswordHash::new(&pw_hash);
            match parsed {
                Ok(ph) => {
                    let verifier = Argon2::default();
                    if verifier
                        .verify_password(payload.password.as_bytes(), &ph)
                        .is_ok()
                    {
                        match issue_jwt(member_id) {
                            Ok(token) => {
                                // set HttpOnly cookie
                                let cookie =
                                    format!("auth_token={}; HttpOnly; Path=/; SameSite=Lax", token);
                                let body = Json(serde_json::json!({"ok": true}));
                                return (StatusCode::OK, [("Set-Cookie", cookie)], body)
                                    .into_response();
                            }
                            Err(e) => {
                                return (
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    format!("token error: {:?}", e),
                                )
                                    .into_response();
                            }
                        }
                    }
                }
                Err(_) => {
                    // invalid stored hash
                }
            }
            (StatusCode::UNAUTHORIZED, "invalid credentials").into_response()
        }
        None => (StatusCode::UNAUTHORIZED, "invalid credentials").into_response(),
    }
}

async fn logout_handler() -> impl IntoResponse {
    // clear cookie by setting Max-Age=0
    let cookie = "auth_token=; HttpOnly; Path=/; Max-Age=0; SameSite=Lax".to_string();
    (
        StatusCode::OK,
        [("Set-Cookie", cookie)],
        Json(serde_json::json!({"ok": true})),
    )
        .into_response()
}

async fn reset_password_handler(
    State(svc): State<Service>,
    headers: HeaderMap,
    Json(req): Json<ResetPasswordRequest>,
) -> impl IntoResponse {
    let token = match extract_token_from_cookies(&headers) {
        Ok(t) => t,
        Err(e) => return e.into_response(),
    };
    let data = match verify_jwt(&token) {
        Ok(d) => d,
        Err(_) => return (StatusCode::UNAUTHORIZED, "invalid token").into_response(),
    };
    let member_id = data.claims.sub;

    // verify old password first
    // fetch current password hash
    // we need the uin -> but we have member_id; create query
    // for simplicity, query by id
    let pw_hash = match svc.get_password_by_id(member_id) {
        Some(h) => h,
        None => return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response(),
    };

    let parsed = PasswordHash::new(&pw_hash);
    match parsed {
        Ok(ph) => {
            let verifier = Argon2::default();
            if verifier
                .verify_password(req.old_password.as_bytes(), &ph)
                .is_err()
            {
                return (StatusCode::UNAUTHORIZED, "old password mismatch").into_response();
            }
        }
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "bad stored password").into_response();
        }
    }

    // hash new password
    let salt = SaltString::generate(&mut OsRng);
    let hasher = Argon2::default();
    match hasher.hash_password(req.new_password.as_bytes(), &salt) {
        Ok(ph) => {
            let encoded = ph.to_string();
            match svc.set_password_for_member_id(member_id, &encoded) {
                Ok(_) => (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.message).into_response(),
            }
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "hash error").into_response(),
    }
}
