use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, FromRequest, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use mach_core::Http;
use mach_db::{Db, Validate};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use subtle::ConstantTimeEq;
use tower::limit::GlobalConcurrencyLimitLayer;
use tower_http::{
    cors::CorsLayer, limit::RequestBodyLimitLayer, timeout::TimeoutLayer, trace::TraceLayer,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Db(#[from] mach_db::Error),
    #[error("authentication required")]
    Unauthorized,
    #[error("access denied")]
    Forbidden,
    #[error("invalid request")]
    Request(StatusCode),
}
pub type Result<T> = std::result::Result<T, Error>;
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        use mach_db::Error as D;
        let status = match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::Request(s) => s,
            Self::Db(D::NotFound) => StatusCode::NOT_FOUND,
            Self::Db(D::Conflict) => StatusCode::CONFLICT,
            Self::Db(D::Invalid) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Db(D::Unavailable) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Db(D::Database) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(serde_json::json!({"error":self.to_string()}))).into_response()
    }
}

pub struct Input<T>(pub T);
impl<S: Send + Sync, T: DeserializeOwned + Validate> FromRequest<S> for Input<T> {
    type Rejection = Error;
    async fn from_request(req: Request, state: &S) -> Result<Self> {
        let Json(value) = Json::<T>::from_request(req, state)
            .await
            .map_err(|e| Error::Request(e.status()))?;
        value.validate()?;
        Ok(Self(value))
    }
}

pub struct Resource<'a> {
    pub table: &'a str,
    pub action: &'a str,
    pub id: Option<i64>,
}
pub trait Access: Send + Sync {
    fn authenticate(&self, headers: &HeaderMap) -> Option<String>;
    fn authorize(&self, principal: &str, resource: Resource<'_>) -> bool;
}
#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub access: Arc<dyn Access>,
}
impl AppState {
    pub fn require(&self, headers: &HeaderMap, resource: Resource<'_>) -> Result<()> {
        let principal = self
            .access
            .authenticate(headers)
            .ok_or(Error::Unauthorized)?;
        self.access
            .authorize(&principal, resource)
            .then_some(())
            .ok_or(Error::Forbidden)
    }
}
pub struct BearerAccess {
    digest: Option<[u8; 32]>,
    grants: Vec<String>,
}
impl BearerAccess {
    pub fn new(token: Option<&str>, grants: &str) -> Self {
        Self {
            digest: token
                .filter(|t| t.len() >= 32)
                .map(|t| Sha256::digest(t).into()),
            grants: grants.split(',').map(|s| s.trim().to_owned()).collect(),
        }
    }
    pub fn from_env() -> Self {
        Self::new(
            std::env::var("MACH_TOKEN").ok().as_deref(),
            &std::env::var("MACH_GRANTS").unwrap_or_default(),
        )
    }
}
impl Access for BearerAccess {
    fn authenticate(&self, headers: &HeaderMap) -> Option<String> {
        let token = headers
            .get("authorization")?
            .to_str()
            .ok()?
            .strip_prefix("Bearer ")?;
        bool::from(self.digest?.ct_eq(&Sha256::digest(token))).then(|| "operator".to_owned())
    }
    fn authorize(&self, principal: &str, r: Resource<'_>) -> bool {
        let scope = format!(
            "{}:{}:{}",
            r.table,
            r.action,
            r.id.map(|v| v.to_string())
                .unwrap_or_else(|| "collection".into())
        );
        principal == "operator" && self.grants.contains(&scope)
    }
}
pub fn bounded(router: Router, h: &Http) -> std::io::Result<Router> {
    let mut cors = CorsLayer::new();
    if let Some(origin) = &h.cors_origin {
        if origin == "*" {
            return Err(std::io::Error::other("CORS requires a single origin"));
        }
        cors = cors
            .allow_origin(
                origin
                    .parse::<HeaderValue>()
                    .map_err(|_| std::io::Error::other("invalid CORS origin"))?,
            )
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
            ])
            .allow_headers([
                axum::http::header::AUTHORIZATION,
                axum::http::header::CONTENT_TYPE,
            ]);
    }
    let admission = tower::ServiceBuilder::new()
        .layer(axum::error_handling::HandleErrorLayer::new(
            |_: axum::BoxError| async { StatusCode::SERVICE_UNAVAILABLE },
        ))
        .load_shed()
        .layer(GlobalConcurrencyLimitLayer::new(h.max_concurrent_requests));
    let trace = TraceLayer::new_for_http().make_span_with(
        |req: &axum::http::Request<_>| tracing::info_span!("request", method = %req.method()),
    );
    Ok(router
        .layer(DefaultBodyLimit::max(h.max_body_bytes))
        .layer(RequestBodyLimitLayer::new(h.max_body_bytes))
        .layer(admission)
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_millis(h.request_timeout_ms),
        ))
        .layer(axum::middleware::map_response(structured_error))
        .layer(cors)
        .layer(trace))
}
async fn structured_error(mut response: Response) -> Response {
    use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE};
    let status = response.status();
    if (status.is_client_error() || status.is_server_error())
        && response
            .headers()
            .get(CONTENT_TYPE)
            .is_none_or(|h| h != "application/json")
    {
        let body = Json(
            serde_json::json!({"error":status.canonical_reason().unwrap_or("request failed")}),
        );
        *response.body_mut() = body.into_response().into_body();
        response.headers_mut().remove(CONTENT_LENGTH);
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }
    response
}
pub fn application(router: Router<AppState>, state: AppState, h: &Http) -> std::io::Result<Router> {
    bounded(
        router
            .route("/health", get(|| async { "ok" }))
            .route(
                "/ready",
                get(|State(s): State<AppState>| async move { sqlx_ping(&s.db).await }),
            )
            .with_state(state),
        h,
    )
}
async fn sqlx_ping(db: &Db) -> StatusCode {
    if db.acquire().await.is_ok() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
pub async fn serve(router: Router, bind: std::net::SocketAddr) -> std::io::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            #[cfg(unix)]
            if let Ok(mut term) =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            {
                tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}};
                return;
            }
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request, routing::post};
    use tower::ServiceExt;
    #[tokio::test]
    async fn limits_timeout_and_structured_rejections() {
        let http = Http {
            max_body_bytes: 3,
            request_timeout_ms: 10,
            ..Default::default()
        };
        let app = bounded(
            Router::new()
                .route("/body", post(|_: String| async {}))
                .route(
                    "/slow",
                    get(|| async { tokio::time::sleep(Duration::from_secs(1)).await }),
                ),
            &http,
        )
        .unwrap();
        for (req, status) in [
            (
                Request::post("/body")
                    .header("content-length", 4)
                    .body(Body::from("abcd"))
                    .unwrap(),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
            (
                Request::get("/slow").body(Body::empty()).unwrap(),
                StatusCode::REQUEST_TIMEOUT,
            ),
        ] {
            let r = app.clone().oneshot(req).await.unwrap();
            assert_eq!(r.status(), status);
            assert_eq!(r.headers()["content-type"], "application/json")
        }
    }
    #[tokio::test]
    async fn excess_concurrency_is_rejected_across_routes() {
        let started = Arc::new(tokio::sync::Notify::new());
        let entered = started.clone();
        let routes = Router::new()
            .route(
                "/hold",
                get(move || async move {
                    entered.notify_one();
                    std::future::pending::<()>().await
                }),
            )
            .route("/quick", get(|| async { "ok" }));
        let app = bounded(
            routes,
            &Http {
                max_concurrent_requests: 1,
                ..Default::default()
            },
        )
        .unwrap();
        let hold = tokio::spawn(
            app.clone()
                .oneshot(Request::get("/hold").body(Body::empty()).unwrap()),
        );
        started.notified().await;
        let r = app
            .oneshot(Request::get("/quick").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        hold.abort();
    }
}
