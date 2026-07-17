//! Storage: Postgres via sqlx, runtime queries only (the repo builds without
//! a database present). Migrations are plain SQL files applied in order.

use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::model::{Listing, SearchQuery, SubmitListing};

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

/// Search with optional full-text query and filters, presence joined in.
pub async fn search(pool: &PgPool, q: &SearchQuery) -> Result<Vec<Listing>, sqlx::Error> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    // One statement, every filter optional: NULL parameters disable their
    // clause. Text search ranks; without a query, newest-updated first.
    let rows = sqlx::query_as::<_, Listing>(
        r#"
        SELECT l.id, l.source, l.source_id, l.name, l.description, l.manifest,
               l.specialties, l.protocol, l.trust, l.created_at, l.updated_at,
               p.state AS presence, p.last_seen_at,
               (SELECT h.handle FROM handles h
                WHERE h.listing_id = l.id AND h.released_at IS NULL
                ORDER BY h.created_at LIMIT 1) AS handle
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

/// Shelf-level counts for the UI header: total listings, how many are
/// listening right now (any alive presence state).
pub async fn stats(pool: &PgPool) -> Result<(i64, i64), sqlx::Error> {
    let (total, online): (i64, i64) = sqlx::query_as(
        r#"
        SELECT count(*),
               count(*) FILTER (WHERE p.state IN ('online','busy','degraded'))
        FROM listings l
        LEFT JOIN presence p ON p.listing_id = l.id
        "#,
    )
    .fetch_one(pool)
    .await?;
    Ok((total, online))
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

/// Upsert a connector-harvested listing. (source, source_id) is the
/// idempotency key — every sweep re-upserts and stays idempotent.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_source_listing(
    pool: &PgPool,
    source: &str,
    source_id: &str,
    name: &str,
    description: &str,
    manifest: &serde_json::Value,
    specialties: &[String],
    trust: Option<&str>,
    protocol: &str,
) -> Result<uuid::Uuid, sqlx::Error> {
    let (id,): (uuid::Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO listings (source, source_id, name, description, manifest, specialties, trust, protocol)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        ON CONFLICT (source, source_id) DO UPDATE SET
            name = EXCLUDED.name,
            description = EXCLUDED.description,
            manifest = EXCLUDED.manifest,
            specialties = EXCLUDED.specialties,
            trust = EXCLUDED.trust,
            protocol = EXCLUDED.protocol,
            updated_at = now()
        RETURNING id
        "#,
    )
    .bind(source)
    .bind(source_id)
    .bind(name)
    .bind(description)
    .bind(manifest)
    .bind(specialties)
    .bind(trust)
    .bind(protocol)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Set one listing's current presence. Any alive state ('online', 'busy',
/// 'degraded') refreshes last_seen_at; 'offline'/'unknown' preserve it.
pub async fn set_presence(
    pool: &PgPool,
    listing_id: uuid::Uuid,
    state: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO presence (listing_id, state, last_seen_at, updated_at)
        VALUES ($1, $2, CASE WHEN $2 IN ('online','busy','degraded') THEN now() END, now())
        ON CONFLICT (listing_id) DO UPDATE SET
            state = EXCLUDED.state,
            last_seen_at = CASE WHEN EXCLUDED.state IN ('online','busy','degraded')
                                THEN now() ELSE presence.last_seen_at END,
            updated_at = now()
        "#,
    )
    .bind(listing_id)
    .bind(state)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fan a node-level presence change out to every listing that node hosts
/// (mesh presence is node-scoped, §9.6). Returns how many listings updated.
pub async fn set_presence_by_node(
    pool: &PgPool,
    node_id: &str,
    state: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        INSERT INTO presence (listing_id, state, last_seen_at, updated_at)
        SELECT l.id, $2, CASE WHEN $2 IN ('online','busy','degraded') THEN now() END, now()
        FROM listings l
        WHERE l.source = 'agentmesh' AND l.manifest->'node'->>'id' = $1
        ON CONFLICT (listing_id) DO UPDATE SET
            state = EXCLUDED.state,
            last_seen_at = CASE WHEN EXCLUDED.state IN ('online','busy','degraded')
                                THEN now() ELSE presence.last_seen_at END,
            updated_at = now()
        "#,
    )
    .bind(node_id)
    .bind(state)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
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
