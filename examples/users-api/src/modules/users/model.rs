use mach::prelude::*;

#[entity(table = "users")]
pub struct User {
    #[id]
    #[generated]
    #[expose]
    pub id: i64,
    #[unique]
    #[writable]
    #[expose]
    #[length(min = 3, max = 254)]
    pub email: String,
    #[column_key("name")]
    #[writable]
    #[expose]
    #[length(min = 2, max = 100)]
    pub full_name: String,
    pub phone: Option<String>,
}
