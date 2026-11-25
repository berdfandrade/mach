use super::model::User;
use mach::repository;

#[repository(User)]
pub trait UserRepository {
    async fn find_by_email(email: &str) -> Result<Option<User>>;
}
