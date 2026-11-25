use mach::prelude::*;

#[entity(table = "MODULE")]
pub struct ENTITY {
    #[id]
    #[generated]
    #[expose]
    pub id: i64,
    #[writable]
    #[expose]
    #[length(min = 2, max = 100)]
    pub name: String,
}
