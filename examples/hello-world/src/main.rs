use mach::{
    axum::{Router, routing::get},
    core::Http,
    http, tokio,
};

#[tokio::main(crate = "mach::tokio")]
async fn main() -> std::io::Result<()> {
    let config = Http::default();
    http::serve(
        http::bounded(
            Router::new().route("/", get(|| async { "Hello, MACH!" })),
            &config,
        )?,
        config.bind,
    )
    .await
}
