use super::model::{User, UserInput};
use mach::prelude::*;

pub struct Controller;

#[controller("/users")]
impl Controller {
    #[get("")]
    async fn list(
        State(s): State<AppState>,
        h: HeaderMap,
        Query(page): Query<Pagination>,
    ) -> Result<Json<Vec<User>>> {
        s.require(
            &h,
            Resource {
                table: User::TABLE,
                action: "list",
                id: None,
            },
        )?;
        Ok(Json(User::list(&s.db, page).await?))
    }
    #[get("/{id}")]
    async fn find(
        State(s): State<AppState>,
        h: HeaderMap,
        Path(id): Path<i64>,
    ) -> Result<Json<User>> {
        s.require(
            &h,
            Resource {
                table: User::TABLE,
                action: "read",
                id: Some(id),
            },
        )?;
        Ok(Json(User::find_or_fail(&s.db, id).await?))
    }
    #[post("")]
    async fn create(
        State(s): State<AppState>,
        h: HeaderMap,
        Input(value): Input<UserInput>,
    ) -> Result<(StatusCode, Json<User>)> {
        s.require(
            &h,
            Resource {
                table: User::TABLE,
                action: "create",
                id: None,
            },
        )?;
        Ok((StatusCode::CREATED, Json(User::create(&s.db, value).await?)))
    }
    #[put("/{id}")]
    async fn update(
        State(s): State<AppState>,
        h: HeaderMap,
        Path(id): Path<i64>,
        Input(value): Input<UserInput>,
    ) -> Result<Json<User>> {
        s.require(
            &h,
            Resource {
                table: User::TABLE,
                action: "update",
                id: Some(id),
            },
        )?;
        Ok(Json(User::update(&s.db, id, value).await?))
    }
    #[delete("/{id}")]
    async fn delete(
        State(s): State<AppState>,
        h: HeaderMap,
        Path(id): Path<i64>,
    ) -> Result<StatusCode> {
        s.require(
            &h,
            Resource {
                table: User::TABLE,
                action: "delete",
                id: Some(id),
            },
        )?;
        User::delete(&s.db, id).await?;
        Ok(StatusCode::NO_CONTENT)
    }
}
