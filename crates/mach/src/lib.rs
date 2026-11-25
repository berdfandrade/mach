pub use mach_macros::*;
pub use {axum, mach_core as core, mach_db as db, mach_http as http, serde, sqlx, tokio};
pub mod prelude {
    pub use axum::{
        Json, Router,
        extract::{Path, Query, State},
        http::{HeaderMap, StatusCode},
    };
    pub use mach_db::{Db, Pagination, Validate};
    pub use mach_http::{AppState, Input, Resource, Result};
    pub use mach_macros::*;
}

#[macro_export]
macro_rules! modules {
    (declared: [$($module:ident),*], routed: [$($route:ident),*])=>{
        $(pub mod $module;)*
        pub fn routes()->$crate::axum::Router<$crate::http::AppState>{
            $crate::axum::Router::new()$(.merge($route::controller::Controller::routes()))*
        }
    };
}
