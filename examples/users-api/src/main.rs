mod app;
use users_api::modules;

#[mach::tokio::main(crate = "mach::tokio")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run().await
}
