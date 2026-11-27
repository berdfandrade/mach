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
    #[writable]
    #[expose]
    #[length(min = 2, max = 100)]
    pub name: String,
}
