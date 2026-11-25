use mach::{core::Http, http, prelude::*};
use std::sync::Arc;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub name: String,
}
impl Validate for Payload {
    fn validate(&self) -> mach::db::Result<()> {
        if (2..=100).contains(&self.name.chars().count()) {
            Ok(())
        } else {
            Err(mach::db::Error::Invalid)
        }
    }
}
pub fn grant(s: &AppState, h: &HeaderMap, action: &str, id: Option<i64>) -> Result<()> {
    s.require(
        h,
        Resource {
            table: "records",
            action,
            id,
        },
    )
}
pub async fn start(
    routes: Router<AppState>,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let db = mach::db::connect(&std::env::var("DATABASE_URL")?, 20).await?;
    let http = Http {
        bind: std::env::var("BENCH_BIND")?.parse()?,
        ..Default::default()
    };
    let state = AppState {
        db,
        access: Arc::new(mach::http::BearerAccess::from_env()),
    };
    http::serve(http::application(routes, state, &http)?, http.bind).await?;
    Ok(())
}
