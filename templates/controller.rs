use super::model::{ENTITY, ENTITYInput};
use mach::prelude::*;

pub struct Controller;

#[controller("/MODULE")]
impl Controller {
    #[get("")]
    async fn list(State(s): State<AppState>, h: HeaderMap, Query(page): Query<Pagination>) -> Result<Json<Vec<ENTITY>>> {
        s.require(&h, Resource { table: ENTITY::TABLE, action: "list", id: None })?;
        Ok(Json(ENTITY::list(&s.db, page).await?))
    }
    #[get("/{id}")]
    async fn find(State(s): State<AppState>, h: HeaderMap, Path(id): Path<i64>) -> Result<Json<ENTITY>> {
        s.require(&h, Resource { table: ENTITY::TABLE, action: "read", id: Some(id) })?;
        Ok(Json(ENTITY::find_or_fail(&s.db, id).await?))
    }
    #[post("")]
    async fn create(State(s): State<AppState>, h: HeaderMap, Input(value): Input<ENTITYInput>) -> Result<(StatusCode,Json<ENTITY>)> {
        s.require(&h, Resource { table: ENTITY::TABLE, action: "create", id: None })?;
        Ok((StatusCode::CREATED,Json(ENTITY::create(&s.db, value).await?)))
    }
    #[put("/{id}")]
    async fn update(State(s): State<AppState>, h: HeaderMap, Path(id): Path<i64>, Input(value): Input<ENTITYInput>) -> Result<Json<ENTITY>> {
        s.require(&h, Resource { table: ENTITY::TABLE, action: "update", id: Some(id) })?;
        Ok(Json(ENTITY::update(&s.db,id, value).await?))
    }
    #[delete("/{id}")]
    async fn delete(State(s): State<AppState>, h: HeaderMap, Path(id): Path<i64>) -> Result<StatusCode> {
        s.require(&h, Resource { table: ENTITY::TABLE, action: "delete", id: Some(id) })?;
        ENTITY::delete(&s.db,id).await?;
        Ok(StatusCode::NO_CONTENT)
    }
}
