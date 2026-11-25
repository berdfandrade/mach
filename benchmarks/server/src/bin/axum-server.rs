use mach::{
    axum::{
        extract::rejection::JsonRejection,
        routing::{get, post},
    },
    prelude::*,
};
use mach_bench::{Payload, grant};
#[derive(sqlx::FromRow, serde::Serialize)]
struct Record {
    id: i64,
    name: String,
}
fn payload(body: std::result::Result<Json<Payload>, JsonRejection>) -> Result<Payload> {
    let Json(v) = body.map_err(|e| mach::http::Error::Request(e.status()))?;
    v.validate()?;
    Ok(v)
}
async fn read(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<Record>> {
    grant(&s, &h, "read", Some(id))?;
    Ok(Json(
        sqlx::query_as("SELECT id, name FROM records WHERE id = $1")
            .bind(id)
            .fetch_one(&s.db)
            .await
            .map_err(mach::db::Error::from)?,
    ))
}
async fn list(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(page): Query<Pagination>,
) -> Result<Json<Vec<Record>>> {
    grant(&s, &h, "list", None)?;
    let p = page.validate()?;
    Ok(Json(
        sqlx::query_as("SELECT id, name FROM records ORDER BY id LIMIT $1 OFFSET $2")
            .bind(p.limit)
            .bind(p.offset)
            .fetch_all(&s.db)
            .await
            .map_err(mach::db::Error::from)?,
    ))
}
async fn write(
    State(s): State<AppState>,
    h: HeaderMap,
    body: std::result::Result<Json<Payload>, JsonRejection>,
) -> Result<Json<Record>> {
    let v = payload(body)?;
    grant(&s, &h, "create", None)?;
    Ok(Json(
        sqlx::query_as("INSERT INTO records (name) VALUES ($1) RETURNING id, name")
            .bind(v.name)
            .fetch_one(&s.db)
            .await
            .map_err(mach::db::Error::from)?,
    ))
}
#[mach::tokio::main(crate = "mach::tokio")]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let routes = Router::new()
        .route("/bench/plain", get(|| async { "ok" }))
        .route(
            "/bench/json",
            get(|| async {
                Json(Payload {
                    name: "example".into(),
                })
            }),
        )
        .route(
            "/bench/path/{id}",
            get(|Path(id): Path<i64>| async move { Json(id) }),
        )
        .route(
            "/bench/validate",
            post(|body| async move {
                payload(body)?;
                Ok::<_, mach::http::Error>(StatusCode::NO_CONTENT)
            }),
        )
        .route(
            "/bench/auth",
            get(|State(s): State<AppState>, h: HeaderMap| async move {
                grant(&s, &h, "read", Some(1))?;
                Ok::<_, mach::http::Error>(StatusCode::NO_CONTENT)
            }),
        )
        .route("/bench/records/{id}", get(read))
        .route("/bench/records", get(list).post(write));
    mach_bench::start(routes).await
}
