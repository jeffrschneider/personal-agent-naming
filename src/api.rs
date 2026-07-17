//! HTTP surface, v0: health, search, read, manual submission.
//! The ARD-compliant read interface will be a sibling router that projects
//! the same listings into the standard's shapes.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::model::{SearchQuery, SubmitListing};
use crate::store;

pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/listings", get(list).post(submit))
        .route("/api/listings/:id", get(get_one))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(pool)
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
        Some(listing) => Ok(Json(serde_json::json!({ "ok": true, "listing": listing }))),
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
