mod handler;
mod service;

refinery::embed_migrations!("migrations");

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let ctx = service::init_service();

    // spawn bot in background
    let bot = ctx.clone();
    tokio::spawn(async move { handler::qbot::run(bot).await });

    // build api app and serve via axum::serve
    let app = handler::api::routes(ctx.clone());
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
