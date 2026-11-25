use mach::{
    core::Config,
    db,
    http::{self, Access, BearerAccess},
    prelude::*,
};
use std::sync::Arc;

struct AccountAccess {
    credential: BearerAccess,
    account: i64,
}
impl Access for AccountAccess {
    fn authenticate(&self, headers: &HeaderMap) -> Option<String> {
        self.credential
            .authenticate(headers)
            .map(|_| self.account.to_string())
    }
    fn authorize(&self, subject: &str, resource: Resource<'_>) -> bool {
        resource.table == "users"
            && resource.action == "read"
            && resource.id.is_some_and(|id| subject.parse() == Ok(id))
    }
}
#[mach::tokio::main(crate = "mach::tokio")]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::load("mach.toml")?;
    let db = db::connect(
        &std::env::var("DATABASE_URL")?,
        cfg.database.max_connections,
    )
    .await?;
    let access = AccountAccess {
        credential: BearerAccess::from_env(),
        account: std::env::var("MACH_ACCOUNT_ID")?.parse()?,
    };
    let app = http::application(
        users_api::modules::routes(),
        AppState {
            db,
            access: Arc::new(access),
        },
        &cfg.http,
    )?;
    Ok(http::serve(app, cfg.http.bind).await?)
}
