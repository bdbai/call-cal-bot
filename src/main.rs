use std::env;

mod handler;
mod service;

refinery::embed_migrations!("migrations");

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let ctx = service::init_service();
    let run_mode = env::var("RUN_MODE").ok();

    if run_mode.as_deref() != Some("bot") {
        // build api app and serve via axum::serve
        let app = handler::api::routes(ctx.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:9004")
            .await
            .unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    }

    // spawn bot in background
    if run_mode.as_deref() != Some("web") {
        let bot = ctx.clone();
        tokio::spawn(async move { handler::qbot::run(bot).await });
    }
    tokio::signal::ctrl_c().await.unwrap();
}
