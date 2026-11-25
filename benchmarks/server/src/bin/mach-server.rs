use mach::prelude::*;
use mach_bench::{Payload, grant};
#[entity(table = "records")]
struct Record {
    #[id]
    #[generated]
    #[expose]
    id: i64,
    #[writable]
    #[expose]
    #[length(min = 2, max = 100)]
    name: String,
}
struct Bench;
#[controller("/bench")]
impl Bench {
    #[get("/plain")]
    async fn plain() -> &'static str {
        "ok"
    }
    #[get("/json")]
    async fn json() -> Json<Payload> {
        Json(Payload {
            name: "example".into(),
        })
    }
    #[get("/path/{id}")]
    async fn path(Path(id): Path<i64>) -> Json<i64> {
        Json(id)
    }
    #[post("/validate")]
    async fn validate(Input(_): Input<Payload>) -> StatusCode {
        StatusCode::NO_CONTENT
    }
    #[get("/auth")]
    async fn auth(State(s): State<AppState>, h: HeaderMap) -> Result<StatusCode> {
        grant(&s, &h, "read", Some(1))?;
        Ok(StatusCode::NO_CONTENT)
    }
    #[get("/records/{id}")]
    async fn read(
        State(s): State<AppState>,
        h: HeaderMap,
        Path(id): Path<i64>,
    ) -> Result<Json<Record>> {
        grant(&s, &h, "read", Some(id))?;
        Ok(Json(Record::find_or_fail(&s.db, id).await?))
    }
    #[get("/records")]
    async fn list(
        State(s): State<AppState>,
        h: HeaderMap,
        Query(page): Query<Pagination>,
    ) -> Result<Json<Vec<Record>>> {
        grant(&s, &h, "list", None)?;
        Ok(Json(Record::list(&s.db, page).await?))
    }
    #[post("/records")]
    async fn write(
        State(s): State<AppState>,
        h: HeaderMap,
        Input(v): Input<RecordInput>,
    ) -> Result<Json<Record>> {
        grant(&s, &h, "create", None)?;
        Ok(Json(Record::create(&s.db, v).await?))
    }
}
#[mach::tokio::main(crate = "mach::tokio")]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    mach_bench::start(Bench::routes()).await
}
