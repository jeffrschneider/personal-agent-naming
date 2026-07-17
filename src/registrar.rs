//! The registrar: email-anchored handle claims.
//!
//! A handle is one globally unique string, `<name>.<email>`. Nobody parses
//! it — resolution is exact-string lookup, uniqueness is enforced on the
//! full string at claim time (first come, first served). The email is the
//! anchor: proving control of it (a 6-digit code) is what authorizes claims
//! under it. Every registrar action appends to the public handle_log.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::postgres::PgPool;
use uuid::Uuid;

/// How long a released handle stays unclaimable — a business card in a
/// drawer shouldn't silently start pointing at a stranger.
const COOLING_OFF_DAYS: i64 = 90;
const CODE_TTL_MIN: i64 = 15;
const SESSION_TTL_MIN: i64 = 30;
const MAX_CODES_PER_HOUR: i64 = 5;
const MAX_CODE_ATTEMPTS: i32 = 5;

#[derive(Debug)]
pub enum RegistrarError {
    Invalid(String),
    Taken(String),
    Unauthorized,
    Db(sqlx::Error),
}

impl std::fmt::Display for RegistrarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistrarError::Invalid(m) => write!(f, "{m}"),
            RegistrarError::Taken(m) => write!(f, "{m}"),
            RegistrarError::Unauthorized => write!(f, "not authorized"),
            RegistrarError::Db(e) => write!(f, "storage error: {e}"),
        }
    }
}
impl From<sqlx::Error> for RegistrarError {
    fn from(e: sqlx::Error) -> Self {
        RegistrarError::Db(e)
    }
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Handle {
    pub handle: String,
    pub email: String,
    pub listing_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub released_at: Option<DateTime<Utc>>,
}

fn normalize_email(email: &str) -> Result<String, RegistrarError> {
    let e = email.trim().to_lowercase();
    let parts: Vec<&str> = e.split('@').collect();
    if parts.len() != 2
        || parts[0].is_empty()
        || !parts[1].contains('.')
        || e.contains(char::is_whitespace)
        || e.len() > 254
    {
        return Err(RegistrarError::Invalid("that doesn't look like an email address".into()));
    }
    Ok(e)
}

fn validate_name(name: &str) -> Result<String, RegistrarError> {
    let n = name.trim().to_string();
    if n.is_empty() || n.len() > 64 {
        return Err(RegistrarError::Invalid("name must be 1–64 characters".into()));
    }
    if n.contains(char::is_whitespace) || n.contains('@') || n.chars().any(char::is_control) {
        return Err(RegistrarError::Invalid("name can't contain spaces or '@'".into()));
    }
    Ok(n)
}

/// Random 6-digit code (uuid v4 as the entropy source — no extra deps).
fn six_digit_code() -> String {
    let b = *Uuid::new_v4().as_bytes();
    let n = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) % 1_000_000;
    format!("{n:06}")
}

/// Start verification: mint a code for this email. The caller delivers it
/// (email provider in production; server log in dev).
pub async fn start_verification(pool: &PgPool, email: &str) -> Result<(String, String), RegistrarError> {
    let email = normalize_email(email)?;
    let (recent,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM email_codes WHERE email = $1 AND created_at > now() - interval '1 hour'",
    )
    .bind(&email)
    .fetch_one(pool)
    .await?;
    if recent >= MAX_CODES_PER_HOUR {
        return Err(RegistrarError::Invalid(
            "too many codes requested — try again in an hour".into(),
        ));
    }
    let code = six_digit_code();
    sqlx::query(
        "INSERT INTO email_codes (email, code, expires_at) VALUES ($1, $2, now() + ($3 || ' minutes')::interval)",
    )
    .bind(&email)
    .bind(&code)
    .bind(CODE_TTL_MIN.to_string())
    .execute(pool)
    .await?;
    Ok((email, code))
}

/// Trade a correct code for a short-lived session token.
pub async fn verify_code(pool: &PgPool, email: &str, code: &str) -> Result<Uuid, RegistrarError> {
    let email = normalize_email(email)?;
    let row: Option<(String, i32)> = sqlx::query_as(
        r#"
        UPDATE email_codes SET attempts = attempts + 1
        WHERE ctid = (
            SELECT ctid FROM email_codes
            WHERE email = $1 AND expires_at > now() AND attempts < $2
            ORDER BY created_at DESC LIMIT 1
        )
        RETURNING code, attempts
        "#,
    )
    .bind(&email)
    .bind(MAX_CODE_ATTEMPTS)
    .fetch_optional(pool)
    .await?;

    let ok = matches!(&row, Some((real, _)) if real == code.trim());
    if !ok {
        return Err(RegistrarError::Invalid("wrong or expired code".into()));
    }
    sqlx::query("DELETE FROM email_codes WHERE email = $1").bind(&email).execute(pool).await?;
    let (token,): (Uuid,) = sqlx::query_as(
        "INSERT INTO email_sessions (email, expires_at) VALUES ($1, now() + ($2 || ' minutes')::interval) RETURNING token",
    )
    .bind(&email)
    .bind(SESSION_TTL_MIN.to_string())
    .fetch_one(pool)
    .await?;
    Ok(token)
}

/// The email a live session belongs to.
pub async fn session_email(pool: &PgPool, token: Uuid) -> Result<String, RegistrarError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT email FROM email_sessions WHERE token = $1 AND expires_at > now()")
            .bind(token)
            .fetch_optional(pool)
            .await?;
    row.map(|(e,)| e).ok_or(RegistrarError::Unauthorized)
}

async fn log_action(
    pool: &PgPool,
    action: &str,
    handle: &str,
    email: &str,
    detail: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO handle_log (action, handle, email, detail) VALUES ($1, $2, $3, $4)")
        .bind(action)
        .bind(handle)
        .bind(email)
        .bind(detail)
        .execute(pool)
        .await?;
    Ok(())
}

/// Claim `<name>.<email>` for a verified email. Full-string uniqueness,
/// first come first served; released handles honor the cooling-off window.
pub async fn claim(
    pool: &PgPool,
    email: &str,
    name: &str,
    listing_id: Option<Uuid>,
) -> Result<String, RegistrarError> {
    let name = validate_name(name)?;
    let handle = format!("{name}.{email}");
    let key = handle.to_lowercase();

    let (cooling,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM handles WHERE handle_key = $1 AND released_at > now() - ($2 || ' days')::interval",
    )
    .bind(&key)
    .bind(COOLING_OFF_DAYS.to_string())
    .fetch_one(pool)
    .await?;
    if cooling > 0 {
        return Err(RegistrarError::Taken(
            "that handle was recently released and is in its cooling-off period".into(),
        ));
    }

    let inserted = sqlx::query(
        "INSERT INTO handles (handle, handle_key, email, listing_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(&handle)
    .bind(&key)
    .bind(email)
    .bind(listing_id)
    .execute(pool)
    .await;
    match inserted {
        Ok(_) => {}
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(RegistrarError::Taken("that handle is taken".into()));
        }
        Err(e) => return Err(e.into()),
    }

    log_action(pool, "claimed", &handle, email, serde_json::json!({ "listing_id": listing_id }))
        .await?;
    log::info!("[catalog:registrar] claimed: {handle} (bound: {})", listing_id.is_some());
    Ok(handle)
}

/// Attach (or re-attach) a handle to an agent listing.
pub async fn bind(
    pool: &PgPool,
    email: &str,
    handle: &str,
    listing_id: Option<Uuid>,
) -> Result<(), RegistrarError> {
    let n = sqlx::query(
        "UPDATE handles SET listing_id = $3 WHERE handle_key = lower($1) AND email = $2 AND released_at IS NULL",
    )
    .bind(handle)
    .bind(email)
    .bind(listing_id)
    .execute(pool)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(RegistrarError::Unauthorized);
    }
    log_action(pool, "bound", handle, email, serde_json::json!({ "listing_id": listing_id })).await?;
    log::info!("[catalog:registrar] bound: {handle} -> {listing_id:?}");
    Ok(())
}

/// Release a handle (tombstone). It stays in history and cools off.
pub async fn release(pool: &PgPool, email: &str, handle: &str) -> Result<(), RegistrarError> {
    let n = sqlx::query(
        "UPDATE handles SET released_at = now() WHERE handle_key = lower($1) AND email = $2 AND released_at IS NULL",
    )
    .bind(handle)
    .bind(email)
    .execute(pool)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(RegistrarError::Unauthorized);
    }
    log_action(pool, "released", handle, email, serde_json::json!({})).await?;
    log::info!("[catalog:registrar] released: {handle}");
    Ok(())
}

/// All active handles anchored to an email.
pub async fn mine(pool: &PgPool, email: &str) -> Result<Vec<Handle>, RegistrarError> {
    Ok(sqlx::query_as::<_, Handle>(
        "SELECT handle, email, listing_id, created_at, released_at
         FROM handles WHERE email = $1 AND released_at IS NULL ORDER BY created_at",
    )
    .bind(email)
    .fetch_all(pool)
    .await?)
}

/// Exact-string resolution: handle -> its record (active only).
pub async fn resolve(pool: &PgPool, handle: &str) -> Result<Option<Handle>, RegistrarError> {
    Ok(sqlx::query_as::<_, Handle>(
        "SELECT handle, email, listing_id, created_at, released_at
         FROM handles WHERE handle_key = lower(trim($1)) AND released_at IS NULL",
    )
    .bind(handle)
    .fetch_optional(pool)
    .await?)
}

/// The public transparency log, newest first.
pub async fn log_entries(pool: &PgPool, limit: i64) -> Result<Vec<serde_json::Value>, RegistrarError> {
    let rows: Vec<(DateTime<Utc>, String, String, serde_json::Value)> = sqlx::query_as(
        "SELECT at, action, handle, detail FROM handle_log ORDER BY id DESC LIMIT $1",
    )
    .bind(limit.clamp(1, 500))
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(at, action, handle, detail)| {
            serde_json::json!({ "at": at, "action": action, "handle": handle, "detail": detail })
        })
        .collect())
}
