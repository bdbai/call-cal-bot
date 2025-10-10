mod handler;
mod service;

refinery::embed_migrations!("migrations");

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let ctx = service::init_service();

    handler::qbot::run(ctx).await;

    tokio::signal::ctrl_c().await.unwrap();
}
