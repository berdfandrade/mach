use mach::{core::Config, db, http, prelude::*};
use std::sync::Arc;

pub async fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let config = Config::load("mach.toml")?;
    let url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required")?;
    let db = db::connect(&url, config.database.max_connections).await?;
    let state = AppState { db, access: Arc::new(http::BearerAccess::from_env()) };
    let app = http::application(crate::modules::routes(), state, &config.http)?;
    Ok(http::serve(app, config.http.bind).await?)
}
