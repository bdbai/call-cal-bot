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
    pub qq_uin: u32,
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
    // 180 days in seconds = 180 * 24 * 3600 = 15,552,000
    let claims = Claims {
        sub: member_id,
        exp: now + 15_552_000usize,
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
        .route("/", get(index_handler))
        .route("/static/{*file}", get(static_handler))
        .route("/daka/records", get(daka_records_handler))
        .route("/daka/gu", get(daka_gu_handler))
        .route("/daka/daka", post(daka_create_handler))
        .route("/daka/daka", delete(daka_delete_handler))
        .with_state(svc)
}

#[derive(Deserialize)]
struct DakaPayload {}

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

use axum::extract::Query;
use std::collections::HashMap;

async fn daka_records_handler(
    State(svc): State<Service>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let token = match extract_token_from_cookies(&headers) {
        Ok(t) => t,
        Err(e) => return e.into_response(),
    };
    let data = match verify_jwt(&token) {
        Ok(d) => d,
        Err(_) => return (StatusCode::UNAUTHORIZED, "invalid token").into_response(),
    };
    let _member_id = data.claims.sub;
    let date = q.get("date").map(|s| s.as_str());
    match svc.query_records_for_date(date) {
        Ok(rows) => {
            // return array of { name, time } where time is null or "HH:MM"
            let arr: Vec<_> = rows
                .into_iter()
                .map(|(n, time)| serde_json::json!({"name": n, "time": time}))
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"records": arr}))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e, "message": "failed to get records"})),
        )
            .into_response(),
    }
}

async fn daka_gu_handler(State(svc): State<Service>, headers: HeaderMap) -> impl IntoResponse {
    let token = match extract_token_from_cookies(&headers) {
        Ok(t) => t,
        Err(e) => return e.into_response(),
    };
    if verify_jwt(&token).is_err() {
        return (StatusCode::UNAUTHORIZED, "invalid token").into_response();
    }

    match svc.query_missed_and_warning() {
        Ok((missed, warn)) => (
            StatusCode::OK,
            Json(serde_json::json!({"missed_10": missed, "warning_7": warn})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

// Serve SPA index.html
async fn index_handler() -> impl IntoResponse {
    match tokio::fs::read_to_string("web/index.html").await {
        Ok(s) => (
            StatusCode::OK,
            [("Content-Type", "text/html; charset=utf-8")],
            s,
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

// Serve static assets from web/static
async fn static_handler(
    axum::extract::Path(file): axum::extract::Path<String>,
) -> impl IntoResponse {
    // prevent path traversal
    if file.contains("..") {
        return (StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let path = format!("web/static/{}", file);
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let content_type = match std::path::Path::new(&path)
                .extension()
                .and_then(|s| s.to_str())
            {
                Some("js") => "application/javascript",
                Some("css") => "text/css",
                Some("png") => "image/png",
                Some("jpg") | Some("jpeg") => "image/jpeg",
                Some("svg") => "image/svg+xml",
                Some("html") => "text/html; charset=utf-8",
                _ => "application/octet-stream",
            };
            let body = bytes;
            (StatusCode::OK, [("Content-Type", content_type)], body).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

async fn daka_create_handler(
    State(svc): State<Service>,
    headers: HeaderMap,
    Json(_payload): Json<DakaPayload>,
) -> impl IntoResponse {
    let token = match extract_token_from_cookies(&headers) {
        Ok(t) => t,
        Err(e) => return e.into_response(),
    };
    let Ok(jwt) = verify_jwt(&token) else {
        return (StatusCode::UNAUTHORIZED, "invalid token").into_response();
    };
    let member_id = jwt.claims.sub as i64;
    let resp = svc.handle_打卡(member_id, "");
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok": resp.ok, "message": resp.message})),
    )
        .into_response()
}

async fn daka_delete_handler(
    State(svc): State<Service>,
    headers: HeaderMap,
    Json(_payload): Json<DakaPayload>,
) -> impl IntoResponse {
    let token = match extract_token_from_cookies(&headers) {
        Ok(t) => t,
        Err(e) => return e.into_response(),
    };
    let Ok(jwt) = verify_jwt(&token) else {
        return (StatusCode::UNAUTHORIZED, "invalid token").into_response();
    };
    let member_id = jwt.claims.sub as i64;
    let resp = svc.handle_我没打卡(member_id, "");
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok": resp.ok, "message": resp.message})),
    )
        .into_response()
}

async fn login_handler(
    State(svc): State<Service>,
    Json(payload): Json<LoginRequest>,
) -> impl IntoResponse {
    // find member by uin
    match svc.find_member_by_uin(payload.uin) {
        Some((member_id, pw_hash)) => {
            // if stored password is empty, instruct frontend to redirect to reset-password
            if pw_hash.trim().is_empty() {
                return (StatusCode::OK, Json(serde_json::json!({"ok": false, "need_reset": true, "message": "password not set"}))).into_response();
            }
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
                                // set HttpOnly cookie with Max-Age matching JWT expiry (180 days)
                                let cookie = format!(
                                    "auth_token={}; HttpOnly; Path=/; Max-Age=15552000; SameSite=Lax",
                                    token
                                );
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
    Json(req): Json<ResetPasswordRequest>,
) -> impl IntoResponse {
    // Anonymous reset: accepts qq_uin + new_password. Only allowed when stored password is empty.
    match svc.find_member_by_uin(req.qq_uin) {
        Some((member_id, pw_hash)) => {
            if !pw_hash.trim().is_empty() {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"ok": false, "message": "password already set"})),
                )
                    .into_response();
            }

            // hash new password
            let salt = SaltString::generate(&mut OsRng);
            let hasher = Argon2::default();
            match hasher.hash_password(req.new_password.as_bytes(), &salt) {
                Ok(ph) => {
                    let encoded = ph.to_string();
                    match svc.set_password_for_member_id(member_id, &encoded) {
                        Ok(_) => {
                            (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response()
                        }
                        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.message).into_response(),
                    }
                }
                Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "hash error").into_response(),
            }
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok": false, "message": "member not found"})),
        )
            .into_response(),
    }
}
