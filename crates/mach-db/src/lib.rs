use serde::Deserialize;
pub use sqlx::{PgExecutor, PgPool as Db, Postgres};
pub type Timestamp = chrono::DateTime<chrono::Utc>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("resource not found")]
    NotFound,
    #[error("invalid input")]
    Invalid,
    #[error("constraint conflict")]
    Conflict,
    #[error("database unavailable")]
    Unavailable,
    #[error("database operation failed")]
    Database,
}
impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => Self::NotFound,
            sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed | sqlx::Error::Io(_) => {
                Self::Unavailable
            }
            sqlx::Error::Database(e) if e.is_unique_violation() || e.is_foreign_key_violation() => {
                Self::Conflict
            }
            sqlx::Error::Database(e) if e.is_check_violation() => Self::Invalid,
            _ => Self::Database,
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub async fn connect(url: &str, max: u32) -> Result<Db> {
    Ok(sqlx::postgres::PgPoolOptions::new()
        .max_connections(max)
        .connect(url)
        .await?)
}
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Pagination {
    pub limit: i64,
    pub offset: i64,
    pub order: Order,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Order {
    #[default]
    Asc,
    Desc,
}
impl Default for Pagination {
    fn default() -> Self {
        Self {
            limit: 25,
            offset: 0,
            order: Order::Asc,
        }
    }
}
impl Pagination {
    pub fn validate(self) -> Result<Self> {
        if (1..=100).contains(&self.limit) && (0..=1_000_000).contains(&self.offset) {
            Ok(self)
        } else {
            Err(Error::Invalid)
        }
    }
}
pub trait Validate {
    fn validate(&self) -> Result<()>;
}
