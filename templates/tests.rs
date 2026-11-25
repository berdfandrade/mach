use super::controller::Controller;
use mach::{axum::{body::Body, http::Request}, http::BearerAccess, prelude::*, sqlx::postgres::PgPoolOptions};
use std::sync::Arc;
use tower::ServiceExt;

#[mach::tokio::test(crate="mach::tokio")]
async fn anonymous_reads_require_authentication() {
    let db=PgPoolOptions::new().connect_lazy("postgresql:///unused").unwrap();
    let state=AppState { db,access:Arc::new(BearerAccess::new(None,"")) };
    let response=Controller::routes().with_state(state).oneshot(Request::builder().uri("/MODULE/1").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::UNAUTHORIZED);
}
