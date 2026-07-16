//! Storage: Postgres via sqlx, runtime queries only (the repo builds without
//! a database present). Migrations are plain SQL files applied in order.

use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::model::{Listing, SearchQuery, SubmitListing};

/// Connect, retrying briefly — `docker compose up` may still be starting the
/// database when the server launches.
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

/// Search with optional full-text query and filters, presence joined in.
pub async fn search(pool: &PgPool, q: &SearchQuery) -> Result<Vec<Listing>, sqlx::Error> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    // One statement, every filter optional: NULL parameters disable their
    // clause. Text search ranks; without a query, newest-updated first.
    let rows = sqlx::query_as::<_, Listing>(
        r#"
        SELECT l.id, l.source, l.source_id, l.name, l.description, l.manifest,
               l.specialties, l.protocol, l.trust, l.created_at, l.updated_at,
               p.state AS presence, p.last_seen_at
        FROM listings l
        LEFT JOIN presence p ON p.listing_id = l.id
        WHERE ($1::text IS NULL OR l.search @@ websearch_to_tsquery('english', $1))
          AND ($2::text IS NULL OR $2 = ANY(l.specialties))
          AND ($3::text IS NULL OR l.protocol = $3)
          AND ($4::text IS NULL OR l.source = $4)
          AND ($5::text IS NULL OR p.state = $5)
        ORDER BY
          CASE WHEN $1::text IS NULL THEN 0
               ELSE ts_rank(l.search, websearch_to_tsquery('english', $1)) END DESC,
          l.updated_at DESC
        LIMIT $6
        "#,
    )
    .bind(&q.q)
    .bind(&q.specialty)
    .bind(&q.protocol)
    .bind(&q.source)
    .bind(&q.presence)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get(pool: &PgPool, id: uuid::Uuid) -> Result<Option<Listing>, sqlx::Error> {
    sqlx::query_as::<_, Listing>(
        r#"
        SELECT l.id, l.source, l.source_id, l.name, l.description, l.manifest,
               l.specialties, l.protocol, l.trust, l.created_at, l.updated_at,
               p.state AS presence, p.last_seen_at
        FROM listings l
        LEFT JOIN presence p ON p.listing_id = l.id
        WHERE l.id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Upsert a manual submission. (source, source_id) is the idempotency key —
/// re-submitting updates rather than duplicating.
pub async fn submit(pool: &PgPool, s: &SubmitListing) -> Result<uuid::Uuid, sqlx::Error> {
    let source_id = s
        .source_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let protocol = s.protocol.clone().unwrap_or_else(|| "unknown".to_string());
    let (id,): (uuid::Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO listings (source, source_id, name, description, manifest, specialties, protocol)
        VALUES ('manual', $1, $2, $3, $4, $5, $6)
        ON CONFLICT (source, source_id) DO UPDATE SET
            name = EXCLUDED.name,
            description = EXCLUDED.description,
            manifest = EXCLUDED.manifest,
            specialties = EXCLUDED.specialties,
            protocol = EXCLUDED.protocol,
            updated_at = now()
        RETURNING id
        "#,
    )
    .bind(&source_id)
    .bind(&s.name)
    .bind(&s.description)
    .bind(&s.manifest)
    .bind(&s.specialties)
    .bind(&protocol)
    .fetch_one(pool)
    .await?;
    Ok(id)
}
