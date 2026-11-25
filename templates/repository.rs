use super::model::ENTITY;
use mach::repository;

#[repository(ENTITY)]
pub trait ENTITYRepository {
    async fn find_by_email(email: &str) -> Result<Option<ENTITY>>;
}
