//! Storage: Postgres via sqlx, runtime queries only (the repo builds without
//! a database present). Migrations are plain SQL files applied in order.

use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::model::{Listing, SubmitListing};

/// Connect, retrying briefly — the embedded server may still be settling
/// when the pool first dials in.
pub async fn connect_with_retry(url: &str) -> PgPool {
    let mut delay = std::time::Duration::from_millis(250);
    for attempt in 1..=8 {
        match PgPoolOptions::new().max_connections(8).connect(url).await {
            Ok(pool) => return pool,
            Err(e) if attempt < 8 => {
                log::warn!("[catalog] db connect attempt {attempt} failed ({e}); retrying in {delay:?}");
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
            Err(e) => panic!("database unreachable at {url}: {e}"),
        }
    }
    unreachable!()
}

pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}

pub async fn get(pool: &PgPool, id: uuid::Uuid) -> Result<Option<Listing>, sqlx::Error> {
    sqlx::query_as::<_, Listing>(
        r#"
        SELECT l.id, l.source, l.source_id, l.name, l.description, l.manifest,
               l.specialties, l.protocol, l.trust, l.created_at, l.updated_at,
               p.state AS presence, p.last_seen_at,
               (SELECT h.handle FROM handles h
                WHERE h.listing_id = l.id AND h.released_at IS NULL
                ORDER BY h.created_at LIMIT 1) AS handle
        FROM listings l
        LEFT JOIN presence p ON p.listing_id = l.id
        WHERE l.id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Recent probe outcomes for one listing, newest first — the "prove it"
/// record shown on the detail view.
pub async fn probes_for(
    pool: &PgPool,
    listing_id: uuid::Uuid,
) -> Result<Vec<serde_json::Value>, sqlx::Error> {
    let rows: Vec<(chrono::DateTime<chrono::Utc>, bool, Option<i32>, Option<String>)> =
        sqlx::query_as(
            r#"
            SELECT at, ok, latency_ms, detail
            FROM probes WHERE listing_id = $1
            ORDER BY at DESC LIMIT 20
            "#,
        )
        .bind(listing_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(at, ok, latency_ms, detail)| {
            serde_json::json!({ "at": at, "ok": ok, "latency_ms": latency_ms, "detail": detail })
        })
        .collect())
}

/// Upsert a manual submission under a verified owner. (source, source_id)
/// is the idempotency key; re-submission only updates the owner's own rows.
pub async fn submit(
    pool: &PgPool,
    s: &SubmitListing,
    owner_email: &str,
) -> Result<Option<uuid::Uuid>, sqlx::Error> {
    let source_id = s
        .source_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let protocol = s.protocol.clone().unwrap_or_else(|| "unknown".to_string());
    // A failed owner check on conflict yields no row: someone else's source_id.
    let row: Option<(uuid::Uuid,)> = sqlx::query_as(
        r#"
        INSERT INTO listings (source, source_id, name, description, manifest, specialties, protocol, owner_email)
        VALUES ('manual', $1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (source, source_id) DO UPDATE SET
            name = EXCLUDED.name,
            description = EXCLUDED.description,
            manifest = EXCLUDED.manifest,
            specialties = EXCLUDED.specialties,
            protocol = EXCLUDED.protocol,
            updated_at = now()
        WHERE listings.owner_email = EXCLUDED.owner_email
        RETURNING id
        "#,
    )
    .bind(&source_id)
    .bind(&s.name)
    .bind(&s.description)
    .bind(&s.manifest)
    .bind(&s.specialties)
    .bind(&protocol)
    .bind(owner_email)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id,)| id))
}

/// The manual listings a verified email submitted (its bindable set).
pub async fn listings_mine(pool: &PgPool, email: &str) -> Result<Vec<Listing>, sqlx::Error> {
    sqlx::query_as::<_, Listing>(
        r#"
        SELECT l.id, l.source, l.source_id, l.name, l.description, l.manifest,
               l.specialties, l.protocol, l.trust, l.created_at, l.updated_at,
               p.state AS presence, p.last_seen_at,
               (SELECT h.handle FROM handles h
                WHERE h.listing_id = l.id AND h.released_at IS NULL
                ORDER BY h.created_at LIMIT 1) AS handle
        FROM listings l
        LEFT JOIN presence p ON p.listing_id = l.id
        WHERE l.source = 'manual' AND l.owner_email = $1
        ORDER BY l.updated_at DESC
        "#,
    )
    .bind(email)
    .fetch_all(pool)
    .await
}
