//! HTTP surface, v0: health, search, read, manual submission.
//! The ARD-compliant read interface will be a sibling router that projects
//! the same listings into the standard's shapes.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::model::{SearchQuery, SubmitListing};
use crate::registrar::{self, RegistrarError};
use crate::store;

pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/", get(ui_index))
        .route("/ui/style.css", get(ui_css))
        .route("/ui/app.js", get(ui_js))
        .route("/healthz", get(healthz))
        .route("/api/stats", get(stats))
        .route("/api/listings", get(list).post(submit))
        .route("/api/listings/:id", get(get_one))
        .route("/api/resolve", get(resolve_handle))
        .route("/api/handles/start", post(handles_start))
        .route("/api/handles/verify", post(handles_verify))
        .route("/api/handles/claim", post(handles_claim))
        .route("/api/handles/bind", post(handles_bind))
        .route("/api/handles/release", post(handles_release))
        .route("/api/handles/mine", get(handles_mine))
        .route("/api/handles/log", get(handles_log))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(pool)
}

// The UI ships inside the binary: every instance — public shelf or
// self-hosted org catalog — serves the same interface with zero extra setup.
async fn ui_index() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("../ui/index.html"))
}
async fn ui_css() -> ([(axum::http::HeaderName, &'static str); 1], &'static str) {
    ([(axum::http::header::CONTENT_TYPE, "text/css")], include_str!("../ui/style.css"))
}
async fn ui_js() -> ([(axum::http::HeaderName, &'static str); 1], &'static str) {
    ([(axum::http::header::CONTENT_TYPE, "text/javascript")], include_str!("../ui/app.js"))
}

async fn stats(State(pool): State<PgPool>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (total, online) = store::stats(&pool).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "ok": true, "total": total, "online": online })))
}

async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

async fn list(
    State(pool): State<PgPool>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let listings = store::search(&pool, &q)
        .await
        .map_err(internal)?;
    Ok(Json(serde_json::json!({ "ok": true, "count": listings.len(), "listings": listings })))
}

async fn get_one(
    State(pool): State<PgPool>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match store::get(&pool, id).await.map_err(internal)? {
        Some(listing) => {
            let probes = store::probes_for(&pool, id).await.map_err(internal)?;
            Ok(Json(serde_json::json!({ "ok": true, "listing": listing, "probes": probes })))
        }
        None => Err((StatusCode::NOT_FOUND, "no such listing".to_string())),
    }
}

async fn submit(
    State(pool): State<PgPool>,
    Json(body): Json<SubmitListing>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if body.name.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name is required".to_string()));
    }
    let id = store::submit(&pool, &body).await.map_err(internal)?;
    log::info!("[catalog] manual submission upserted: {} ({})", body.name, id);
    Ok(Json(serde_json::json!({ "ok": true, "id": id })))
}

fn internal(e: sqlx::Error) -> (StatusCode, String) {
    log::error!("[catalog] db error: {e}");
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

// ── the registrar: email-anchored handles ──────────────────────────────────

fn reg_err(e: RegistrarError) -> (StatusCode, Json<serde_json::Value>) {
    let (code, msg) = match &e {
        RegistrarError::Invalid(m) => (StatusCode::BAD_REQUEST, m.clone()),
        RegistrarError::Taken(m) => (StatusCode::CONFLICT, m.clone()),
        RegistrarError::Unauthorized => {
            (StatusCode::UNAUTHORIZED, "session expired — verify your email again".to_string())
        }
        RegistrarError::Db(err) => {
            log::error!("[catalog:registrar] db error: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, "storage error".to_string())
        }
    };
    (code, Json(serde_json::json!({ "ok": false, "error": msg })))
}

async fn session_from(
    pool: &PgPool,
    headers: &HeaderMap,
) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|v| Uuid::parse_str(v.trim()).ok())
        .ok_or_else(|| reg_err(RegistrarError::Unauthorized))?;
    registrar::session_email(pool, token).await.map_err(reg_err)
}

/// Deliver a verification code. No email provider is wired yet, so dev mode
/// logs it to the server console; the response says which delivery happened.
fn deliver_code(email: &str, code: &str) -> &'static str {
    log::info!("[catalog:registrar] verification code for {email}: {code} (dev mode — no email provider configured)");
    "console"
}

#[derive(Deserialize)]
struct StartBody {
    email: String,
}

async fn handles_start(
    State(pool): State<PgPool>,
    Json(body): Json<StartBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let (email, code) = registrar::start_verification(&pool, &body.email).await.map_err(reg_err)?;
    let delivery = deliver_code(&email, &code);
    Ok(Json(serde_json::json!({ "ok": true, "email": email, "delivery": delivery })))
}

#[derive(Deserialize)]
struct VerifyBody {
    email: String,
    code: String,
}

async fn handles_verify(
    State(pool): State<PgPool>,
    Json(body): Json<VerifyBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let token = registrar::verify_code(&pool, &body.email, &body.code).await.map_err(reg_err)?;
    Ok(Json(serde_json::json!({ "ok": true, "token": token })))
}

#[derive(Deserialize)]
struct ClaimBody {
    name: String,
    #[serde(default)]
    listing_id: Option<Uuid>,
}

async fn handles_claim(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(body): Json<ClaimBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let email = session_from(&pool, &headers).await?;
    let handle =
        registrar::claim(&pool, &email, &body.name, body.listing_id).await.map_err(reg_err)?;
    Ok(Json(serde_json::json!({ "ok": true, "handle": handle })))
}

#[derive(Deserialize)]
struct BindBody {
    handle: String,
    #[serde(default)]
    listing_id: Option<Uuid>,
}

async fn handles_bind(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(body): Json<BindBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let email = session_from(&pool, &headers).await?;
    registrar::bind(&pool, &email, &body.handle, body.listing_id).await.map_err(reg_err)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ReleaseBody {
    handle: String,
}

async fn handles_release(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(body): Json<ReleaseBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let email = session_from(&pool, &headers).await?;
    registrar::release(&pool, &email, &body.handle).await.map_err(reg_err)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn handles_mine(
    State(pool): State<PgPool>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let email = session_from(&pool, &headers).await?;
    let handles = registrar::mine(&pool, &email).await.map_err(reg_err)?;
    Ok(Json(serde_json::json!({ "ok": true, "email": email, "handles": handles })))
}

#[derive(Deserialize)]
struct ResolveQuery {
    handle: String,
}

/// Public exact-string resolution: handle -> record + bound listing card.
async fn resolve_handle(
    State(pool): State<PgPool>,
    Query(q): Query<ResolveQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let found = registrar::resolve(&pool, &q.handle).await.map_err(reg_err)?;
    match found {
        None => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "ok": false, "error": "no agent by that name" })),
        )),
        Some(h) => {
            let listing = match h.listing_id {
                Some(id) => store::get(&pool, id).await.map_err(|e| reg_err(e.into()))?,
                None => None,
            };
            Ok(Json(serde_json::json!({ "ok": true, "handle": h, "listing": listing })))
        }
    }
}

#[derive(Deserialize)]
struct LogQuery {
    #[serde(default)]
    limit: Option<i64>,
}

async fn handles_log(
    State(pool): State<PgPool>,
    Query(q): Query<LogQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let entries =
        registrar::log_entries(&pool, q.limit.unwrap_or(100)).await.map_err(reg_err)?;
    Ok(Json(serde_json::json!({ "ok": true, "entries": entries })))
}
