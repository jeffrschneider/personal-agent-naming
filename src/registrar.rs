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
const SESSION_TTL_MIN: i64 = 480; // 8h console sessions (user call 2026-07-18)
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
    pub anchor: String,
    pub bind_method: Option<String>,
    pub verified_at: Option<DateTime<Utc>>,
    pub stale: bool,
    pub created_at: DateTime<Utc>,
    pub released_at: Option<DateTime<Utc>>,
}

macro_rules! handle_cols {
    () => {
        "handle, email, listing_id, anchor, bind_method, verified_at, stale, created_at, released_at"
    };
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

/// Canonical JSON for hashing: compact, keys recursively sorted. Explicit
/// so the chain never depends on serde_json's map-ordering configuration.
fn canonical_json(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{}:{}", serde_json::Value::String(k.clone()), canonical_json(&map[k])))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        serde_json::Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", inner.join(","))
        }
        other => other.to_string(),
    }
}

/// Append to the hash-chained transparency log (PAN §6). Serialized via an
/// advisory lock; each entry's hash covers the canonical JSON serialization
/// (compact, lexicographically sorted keys) that includes the previous
/// entry's hash.
async fn log_action(
    pool: &PgPool,
    action: &str,
    handle: &str,
    email: &str,
    detail: serde_json::Value,
) -> Result<(), sqlx::Error> {
    use base64::Engine;
    use sha2::Digest;

    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(4207)").execute(&mut *tx).await?;
    let head: Option<(i64, Option<String>)> =
        sqlx::query_as("SELECT id, entry_hash FROM handle_log ORDER BY id DESC LIMIT 1")
            .fetch_optional(&mut *tx)
            .await?;
    // Pre-chain rows have NULL hashes; the chain geneses with "".
    let prev_hash = head.and_then(|(_, h)| h).unwrap_or_default();
    let at = Utc::now();

    let (seq,): (i64,) = sqlx::query_as(
        "INSERT INTO handle_log (at, action, handle, email, detail, prev_hash) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(at)
    .bind(action)
    .bind(handle)
    .bind(email)
    .bind(&detail)
    .bind(&prev_hash)
    .fetch_one(&mut *tx)
    .await?;

    let canonical = serde_json::json!({
        "action": action,
        "at": at.to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
        "detail": detail,
        "email": email,
        "handle": handle,
        "prev_hash": prev_hash,
        "seq": seq,
    });
    let digest = sha2::Sha256::digest(canonical_json(&canonical).as_bytes());
    let entry_hash = base64::engine::general_purpose::STANDARD.encode(digest);

    sqlx::query("UPDATE handle_log SET entry_hash = $1 WHERE id = $2")
        .bind(&entry_hash)
        .bind(seq)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Can this verified email bind a handle to this listing on email proof
/// alone? Only if it submitted the listing (PAN §4.1). Key-bearing listings
/// need pairing (§4.2) — email proof says nothing about agent control.
async fn check_email_bindable(
    pool: &PgPool,
    email: &str,
    listing_id: Uuid,
) -> Result<(), RegistrarError> {
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT source, owner_email FROM listings WHERE id = $1")
            .bind(listing_id)
            .fetch_optional(pool)
            .await?;
    match row {
        None => Err(RegistrarError::Invalid("no such listing".into())),
        Some((source, owner)) if source == "manual" && owner.as_deref() == Some(email) => Ok(()),
        Some((source, _)) if source == "manual" => Err(RegistrarError::Unauthorized),
        Some(_) => Err(RegistrarError::Invalid(
            "this agent requires key pairing — proving you received an email doesn't prove you operate it".into(),
        )),
    }
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
    if let Some(id) = listing_id {
        check_email_bindable(pool, email, id).await?;
    }
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
        "INSERT INTO handles (handle, handle_key, email, listing_id, bind_method) \
         VALUES ($1, $2, $3, $4, CASE WHEN $4::uuid IS NULL THEN NULL ELSE 'email-submitter' END)",
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

/// Attach (or re-attach) a handle to an agent listing on email proof
/// (submitter-match only, §4.1). Detaching (listing_id = None) is always
/// allowed for the handle's owner.
pub async fn bind(
    pool: &PgPool,
    email: &str,
    handle: &str,
    listing_id: Option<Uuid>,
) -> Result<(), RegistrarError> {
    if let Some(id) = listing_id {
        check_email_bindable(pool, email, id).await?;
    }
    let n = sqlx::query(
        "UPDATE handles SET listing_id = $3, \
             bind_method = CASE WHEN $3::uuid IS NULL THEN NULL ELSE 'email-submitter' END \
         WHERE handle_key = lower($1) AND email = $2 AND released_at IS NULL",
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
    log_action(
        pool,
        "bound",
        handle,
        email,
        serde_json::json!({ "listing_id": listing_id, "method": "email-submitter" }),
    )
    .await?;
    log::info!("[catalog:registrar] bound: {handle} -> {listing_id:?} (email-submitter)");
    Ok(())
}

/// Start pairing (§4.2): a short single-use code the handle owner relays to
/// whatever software holds their agent's key.
pub async fn pair_start(
    pool: &PgPool,
    email: &str,
    handle: &str,
) -> Result<(String, DateTime<Utc>), RegistrarError> {
    let owned: Option<(String,)> = sqlx::query_as(
        "SELECT handle FROM handles WHERE handle_key = lower($1) AND email = $2 AND released_at IS NULL",
    )
    .bind(handle)
    .bind(email)
    .fetch_optional(pool)
    .await?;
    let Some((display_handle,)) = owned else {
        return Err(RegistrarError::Unauthorized);
    };

    // Short, copyable, unambiguous: XXX-XXX from A-Z/2-9 minus 0/O/1/I.
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let bytes = *Uuid::new_v4().as_bytes();
    let chars: String =
        bytes.iter().take(6).map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char).collect();
    let code = format!("{}-{}", &chars[..3], &chars[3..]);
    let expires = Utc::now() + chrono::Duration::minutes(10);
    sqlx::query("INSERT INTO pairing_codes (code, handle, email, expires_at) VALUES ($1, $2, $3, $4)")
        .bind(&code)
        .bind(&display_handle)
        .bind(email)
        .bind(expires)
        .execute(pool)
        .await?;
    log::info!("[catalog:registrar] pairing started for {display_handle}");
    Ok((code, expires))
}

/// Complete pairing (§4.2). Unauthenticated by design: the code proves the
/// handle owner initiated it; the signature over the canonical string
/// `pan-pair-v1:<code>:<agent-id>` proves agent control. The binding is
/// the intersection of the two proofs.
pub async fn pair_complete(
    pool: &PgPool,
    code: &str,
    agent_id: &str,
    signature_b64: &str,
) -> Result<String, RegistrarError> {
    use base64::Engine;

    let code = code.trim().to_uppercase();
    let agent_id = agent_id.trim();

    let row: Option<(String, String)> = sqlx::query_as(
        "UPDATE pairing_codes SET used_at = now() \
         WHERE code = $1 AND used_at IS NULL AND expires_at > now() \
         RETURNING handle, email",
    )
    .bind(&code)
    .fetch_optional(pool)
    .await?;
    let Some((handle, email)) = row else {
        return Err(RegistrarError::Invalid("unknown, used, or expired pairing code".into()));
    };

    let sig = base64::engine::general_purpose::STANDARD
        .decode(signature_b64.trim())
        .map_err(|_| RegistrarError::Invalid("signature is not valid base64".into()))?;
    let msg = format!("pan-pair-v1:{code}:{agent_id}");
    let kp = agentmesh::KeyPair::from_public_key(agent_id)
        .map_err(|_| RegistrarError::Invalid("agent_id is not a valid public key".into()))?;
    kp.verify(msg.as_bytes(), &sig)
        .map_err(|_| RegistrarError::Invalid("signature does not verify against the agent key".into()))?;

    // The signature is the agent record's authorization. There is no prior
    // directory to look the agent up in (PAN §4.1): the proven key IS the
    // record. Create or refresh a minimal record and bind to it.
    let (name, _) = handle.split_once('.').unwrap_or((handle.as_str(), ""));
    let (listing_id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO listings (source, source_id, name, description, manifest, specialties, protocol) \
         VALUES ('agent', $1, $2, '', $3, '{}', 'mesh') \
         ON CONFLICT (source, source_id) DO UPDATE SET name = EXCLUDED.name, updated_at = now() \
         RETURNING id",
    )
    .bind(agent_id)
    .bind(name)
    .bind(serde_json::json!({ "id": agent_id }))
    .fetch_one(pool)
    .await?;

    sqlx::query(
        "UPDATE handles SET listing_id = $2, bind_method = 'agent-key' \
         WHERE handle_key = lower($1) AND released_at IS NULL",
    )
    .bind(&handle)
    .bind(listing_id)
    .execute(pool)
    .await?;
    log_action(
        pool,
        "bound",
        &handle,
        &email,
        serde_json::json!({ "listing_id": listing_id, "method": "agent-key", "agent_id": agent_id }),
    )
    .await?;
    log::info!("[catalog:registrar] bound: {handle} -> {listing_id} (agent-key, {agent_id})");
    Ok(handle)
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
    Ok(sqlx::query_as::<_, Handle>(concat!(
        "SELECT ",
        handle_cols!(),
        " FROM handles WHERE email = $1 AND released_at IS NULL ORDER BY created_at"
    ))
    .bind(email)
    .fetch_all(pool)
    .await?)
}

/// Exact-string resolution: handle -> its record (active only).
pub async fn resolve(pool: &PgPool, handle: &str) -> Result<Option<Handle>, RegistrarError> {
    Ok(sqlx::query_as::<_, Handle>(concat!(
        "SELECT ",
        handle_cols!(),
        " FROM handles WHERE handle_key = lower(trim($1)) AND released_at IS NULL"
    ))
    .bind(handle)
    .fetch_optional(pool)
    .await?)
}

/// The declared agent profile (User-Agent trust class): what kind of agent,
/// hosted by what, on which platform. Signed into the check-in canonical.
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct AgentProfile {
    pub kind: Option<String>,
    pub host: Option<String>,
    pub platform: Option<String>,
}

/// Record where an agent last connected from (owner-facing provenance) and,
/// when declared, its profile. Profile fields only overwrite when supplied;
/// first_connected_at is set once.
pub async fn record_agent_seen(
    pool: &PgPool,
    agent_id: &str,
    ip: &str,
    profile: &AgentProfile,
) -> Result<(), RegistrarError> {
    let trim = |o: &Option<String>| o.as_deref().map(str::trim).filter(|s| !s.is_empty() && s.chars().count() <= 80).map(str::to_string);
    sqlx::query(
        "UPDATE listings SET last_seen_ip = $2, last_seen_ip_at = now(), \
           first_connected_at = COALESCE(first_connected_at, now()), \
           agent_kind = COALESCE($3, agent_kind), \
           agent_host = COALESCE($4, agent_host), \
           agent_platform = COALESCE($5, agent_platform) \
         WHERE source IN ('agent','agentmesh') AND source_id = $1",
    )
    .bind(agent_id.trim())
    .bind(ip)
    .bind(trim(&profile.kind))
    .bind(trim(&profile.host))
    .bind(trim(&profile.platform))
    .execute(pool)
    .await?;
    Ok(())
}

/// Signed check-in (adapter start): proves key control, refreshes last-seen,
/// and carries the declared profile. Canonicals (newline-joined so free-text
/// fields cannot create ambiguity):
///   v1: `pan-checkin-v1:<ts>:<agent-id>`                       (no profile)
///   v2: "pan-checkin-v2"\n<ts>\n<agent-id>\n<kind>\n<host>\n<platform>
/// The timestamp must be within a small window so a captured request cannot
/// be replayed later to plant stale data.
pub async fn checkin(
    pool: &PgPool,
    agent_id: &str,
    ts: i64,
    signature_b64: &str,
    ip: &str,
    profile: &AgentProfile,
) -> Result<(), RegistrarError> {
    use base64::Engine;
    let now = Utc::now().timestamp();
    if (now - ts).abs() > 300 {
        return Err(RegistrarError::Invalid("check-in timestamp outside the accepted window".into()));
    }
    let sig = base64::engine::general_purpose::STANDARD
        .decode(signature_b64.trim())
        .map_err(|_| RegistrarError::Invalid("signature is not valid base64".into()))?;
    let agent_id = agent_id.trim();
    let has_profile = profile.kind.is_some() || profile.host.is_some() || profile.platform.is_some();
    let msg = if has_profile {
        [
            "pan-checkin-v2".to_string(),
            ts.to_string(),
            agent_id.to_string(),
            profile.kind.clone().unwrap_or_default(),
            profile.host.clone().unwrap_or_default(),
            profile.platform.clone().unwrap_or_default(),
        ]
        .join("\n")
    } else {
        format!("pan-checkin-v1:{ts}:{agent_id}")
    };
    let kp = agentmesh::KeyPair::from_public_key(agent_id)
        .map_err(|_| RegistrarError::Invalid("agent_id is not a valid public key".into()))?;
    kp.verify(msg.as_bytes(), &sig)
        .map_err(|_| RegistrarError::Invalid("signature does not verify against the agent key".into()))?;
    record_agent_seen(pool, agent_id, ip, profile).await
}

/// Owner-only: provenance + declared profile for a set of listings.
pub type SeenRow = (Option<String>, Option<DateTime<Utc>>, Option<String>, Option<String>, Option<String>, Option<DateTime<Utc>>);
pub async fn last_seen_ips(
    pool: &PgPool,
    listing_ids: &[Uuid],
) -> Result<std::collections::HashMap<Uuid, SeenRow>, RegistrarError> {
    let rows: Vec<(Uuid, Option<String>, Option<DateTime<Utc>>, Option<String>, Option<String>, Option<String>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT id, last_seen_ip, last_seen_ip_at, agent_kind, agent_host, agent_platform, first_connected_at          FROM listings WHERE id = ANY($1)",
    )
    .bind(listing_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id, ip, at, k, h, pf, fc)| (id, (ip, at, k, h, pf, fc))).collect())
}

/// Reverse resolution (PAN v0.3): agent key -> its bound handle, if any.
/// The key is public (it arrives on every signed envelope), so this maps a
/// verified sender to a name without exposing anything a card does not.
pub async fn resolve_by_agent(pool: &PgPool, agent_id: &str) -> Result<Option<Handle>, RegistrarError> {
    Ok(sqlx::query_as::<_, Handle>(concat!(
        "SELECT ",
        handle_cols!(),
        " FROM handles WHERE released_at IS NULL AND listing_id IN \
          (SELECT id FROM listings WHERE source IN ('agent','agentmesh') AND source_id = $1) \
          ORDER BY created_at DESC LIMIT 1"
    ))
    .bind(agent_id.trim())
    .fetch_optional(pool)
    .await?)
}

/// The operator's display name (PAN v0.3): one required public label per
/// verified email, shown on every card of that owner's handles. It is the
/// operator's CHOSEN label anchored to the proven email; it is not verified
/// identity, and the spec says so.
pub async fn operator_get(pool: &PgPool, email: &str) -> Result<Option<String>, RegistrarError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT display_name FROM operators WHERE email = $1")
            .bind(email)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|r| r.0))
}

/// Set or update the operator display name (logged, like every registrar action).
pub async fn operator_set(
    pool: &PgPool,
    email: &str,
    display_name: &str,
) -> Result<String, RegistrarError> {
    let name = display_name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(RegistrarError::Invalid(
            "operator display name must be 1-80 characters".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO operators (email, display_name, updated_at) VALUES ($1, $2, now()) \
         ON CONFLICT (email) DO UPDATE SET display_name = EXCLUDED.display_name, updated_at = now()",
    )
    .bind(email)
    .bind(name)
    .execute(pool)
    .await?;
    log_action(pool, "operator", "-", email, serde_json::json!({ "display_name": name })).await?;
    log::info!("[catalog:registrar] operator name set for {email}");
    Ok(name.to_string())
}

/// One owner's own handle history (PAN §6). Owner-scoped by verified
/// anchor: the log is never world-readable, since email-tier handles embed
/// the owner's address and a public log would enumerate their whole roster.
pub async fn log_entries(pool: &PgPool, email: &str, limit: i64) -> Result<Vec<serde_json::Value>, RegistrarError> {
    let rows: Vec<(i64, DateTime<Utc>, String, String, String, serde_json::Value, Option<String>, Option<String>)> =
        sqlx::query_as(
            "SELECT id, at, action, handle, email, detail, prev_hash, entry_hash \
             FROM handle_log WHERE email = $1 ORDER BY id DESC LIMIT $2",
        )
        .bind(email)
        .bind(limit.clamp(1, 500))
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(seq, at, action, handle, email, detail, prev_hash, entry_hash)| {
            serde_json::json!({
                "seq": seq,
                "at": at.to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
                "action": action, "handle": handle, "email": email, "detail": detail,
                "prev_hash": prev_hash, "entry_hash": entry_hash,
            })
        })
        .collect())
}
