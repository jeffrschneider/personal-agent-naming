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

/// The registrar's Ed25519 signing keypair, created on first use and kept
/// in registrar_meta.
async fn signing_key(pool: &PgPool) -> Result<agentmesh::KeyPair, RegistrarError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM registrar_meta WHERE key = 'log_signing_seed'")
            .fetch_optional(pool)
            .await?;
    if let Some((seed,)) = row {
        return agentmesh::KeyPair::from_seed(&seed)
            .map_err(|e| RegistrarError::Invalid(format!("bad stored signing seed: {e}")));
    }
    let kp = agentmesh::KeyPair::new_user();
    let seed = kp
        .seed()
        .map_err(|e| RegistrarError::Invalid(format!("seed export failed: {e}")))?;
    sqlx::query(
        "INSERT INTO registrar_meta (key, value) VALUES ('log_signing_seed', $1) \
         ON CONFLICT (key) DO NOTHING",
    )
    .bind(&seed)
    .execute(pool)
    .await?;
    log::info!("[catalog:registrar] generated log signing key: {}", kp.public_key());
    Ok(kp)
}

/// Signed checkpoint over the log head (PAN §6): anyone replaying the log
/// and recomputing the chain can check it against this signature.
pub async fn checkpoint(pool: &PgPool) -> Result<serde_json::Value, RegistrarError> {
    use base64::Engine;
    let head: Option<(i64, Option<String>)> =
        sqlx::query_as("SELECT id, entry_hash FROM handle_log ORDER BY id DESC LIMIT 1")
            .fetch_optional(pool)
            .await?;
    let (seq, entry_hash) = match head {
        Some((s, Some(h))) => (s, h),
        _ => return Err(RegistrarError::Invalid("log has no chained entries yet".into())),
    };
    let kp = signing_key(pool).await?;
    let at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
    let msg = format!("pan-log-checkpoint-v1:{seq}:{entry_hash}:{at}");
    let sig = kp
        .sign(msg.as_bytes())
        .map_err(|e| RegistrarError::Invalid(format!("signing failed: {e}")))?;
    Ok(serde_json::json!({
        "seq": seq,
        "entry_hash": entry_hash,
        "at": at,
        "signature": base64::engine::general_purpose::STANDARD.encode(sig),
        "signer": kp.public_key(),
    }))
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

    let listing: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM listings WHERE source = 'agentmesh' AND source_id = $1")
            .bind(agent_id)
            .fetch_optional(pool)
            .await?;
    let Some((listing_id,)) = listing else {
        return Err(RegistrarError::Invalid(
            "no listing with that agent key — the catalog hasn't harvested this agent".into(),
        ));
    };

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

// ── domain tier (§3.2): record-driven, publicly re-verifiable ─────────────

/// One declared handle from a domain record.
#[derive(Debug, serde::Deserialize)]
pub struct DomainEntry {
    pub name: String,
    #[serde(default)]
    pub key: Option<String>,
}

/// Fetch a domain's PAN record: well-known first (HTTPS, loopback excepted
/// for dev), DNS TXT at `_pan.<domain>` via DNS-over-HTTPS as fallback.
async fn fetch_domain_record(domain: &str) -> Result<Vec<DomainEntry>, RegistrarError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| RegistrarError::Invalid(e.to_string()))?;

    // The spec requires HTTPS; loopback is the dev exception.
    let loopback = domain.starts_with("localhost") || domain.starts_with("127.0.0.1");
    let scheme = if loopback { "http" } else { "https" };
    let wk_url = format!("{scheme}://{domain}/.well-known/pan.json");
    match client.get(&wk_url).send().await {
        Ok(r) if r.status().is_success() => {
            #[derive(serde::Deserialize)]
            struct WellKnown {
                version: String,
                handles: Vec<DomainEntry>,
            }
            let wk: WellKnown = r
                .json()
                .await
                .map_err(|e| RegistrarError::Invalid(format!("pan.json didn't parse: {e}")))?;
            if !wk.version.starts_with("pan-") {
                return Err(RegistrarError::Invalid("pan.json has an unknown version".into()));
            }
            log::info!("[catalog:domains] {domain}: well-known record, {} handle(s)", wk.handles.len());
            return Ok(wk.handles);
        }
        Ok(r) => log::info!("[catalog:domains] {domain}: no well-known record (HTTP {}), trying DNS", r.status()),
        Err(e) => log::info!("[catalog:domains] {domain}: well-known fetch failed ({e}), trying DNS"),
    }

    // DNS TXT via DoH: one record per handle, "v=pan1; name=X; key=Y".
    let doh = format!("https://cloudflare-dns.com/dns-query?name=_pan.{domain}&type=TXT");
    let resp: serde_json::Value = client
        .get(&doh)
        .header("accept", "application/dns-json")
        .send()
        .await
        .map_err(|e| RegistrarError::Invalid(format!("DNS lookup failed: {e}")))?
        .json()
        .await
        .map_err(|e| RegistrarError::Invalid(format!("DNS response didn't parse: {e}")))?;
    let mut entries = Vec::new();
    for ans in resp.get("Answer").and_then(|a| a.as_array()).unwrap_or(&Vec::new()) {
        let Some(data) = ans.get("data").and_then(|d| d.as_str()) else { continue };
        // TXT payloads arrive as one or more quoted chunks; join them.
        let txt: String = data.split('"').filter(|s| !s.trim().is_empty()).collect();
        let mut fields: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        for part in txt.split(';') {
            if let Some((k, v)) = part.split_once('=') {
                fields.insert(k.trim(), v.trim());
            }
        }
        if fields.get("v") != Some(&"pan1") {
            continue;
        }
        if let Some(name) = fields.get("name") {
            entries.push(DomainEntry {
                name: (*name).to_string(),
                key: fields.get("key").map(|k| (*k).to_string()),
            });
        }
    }
    if entries.is_empty() {
        return Err(RegistrarError::Invalid(format!(
            "no PAN record found for {domain} (no /.well-known/pan.json, no _pan TXT records)"
        )));
    }
    log::info!("[catalog:domains] {domain}: DNS record, {} handle(s)", entries.len());
    Ok(entries)
}

/// Sync a domain (§3.2): mirror its published record into claims/bindings.
/// Unauthenticated by design — the record is the authorization. Returns a
/// per-handle summary.
pub async fn sync_domain(pool: &PgPool, domain: &str) -> Result<serde_json::Value, RegistrarError> {
    let domain = domain.trim().to_lowercase();
    if domain.is_empty() || domain.contains('@') || domain.contains('/') || domain.contains(char::is_whitespace) {
        return Err(RegistrarError::Invalid("that doesn't look like a domain".into()));
    }
    let entries = fetch_domain_record(&domain).await?;
    let mut summary: Vec<serde_json::Value> = Vec::new();
    let mut declared: Vec<String> = Vec::new();

    for e in &entries {
        let Ok(name) = validate_name(&e.name) else {
            summary.push(serde_json::json!({ "name": e.name, "status": "invalid-name" }));
            continue;
        };
        let handle = format!("{name}@{domain}");
        let key = handle.to_lowercase();
        declared.push(key.clone());

        // The listing a declared agent key points at, if harvested yet.
        let listing_id: Option<Uuid> = match &e.key {
            Some(k) => sqlx::query_as::<_, (Uuid,)>(
                "SELECT id FROM listings WHERE source = 'agentmesh' AND source_id = $1",
            )
            .bind(k.trim())
            .fetch_optional(pool)
            .await?
            .map(|(id,)| id),
            None => None,
        };
        let bind_method = listing_id.map(|_| "domain-record");

        let existing: Option<(String, String)> = sqlx::query_as(
            "SELECT anchor, email FROM handles WHERE handle_key = $1 AND released_at IS NULL",
        )
        .bind(&key)
        .fetch_optional(pool)
        .await?;

        match existing {
            // Ours: refresh verification, follow key/binding changes.
            Some((anchor, owner)) if anchor == "domain" && owner == domain => {
                sqlx::query(
                    "UPDATE handles SET verified_at = now(), stale = false, \
                         listing_id = COALESCE($2, listing_id), \
                         bind_method = COALESCE($3, bind_method) \
                     WHERE handle_key = $1 AND released_at IS NULL",
                )
                .bind(&key)
                .bind(listing_id)
                .bind(bind_method)
                .execute(pool)
                .await?;
                summary.push(serde_json::json!({ "handle": handle, "status": "verified" }));
            }
            // Taken at the other tier (or another domain, impossible by
            // string) — Rule 2: first come, first served. Log the refusal.
            Some(_) => {
                log_action(pool, "refused", &handle, &domain,
                    serde_json::json!({ "reason": "taken", "tier": "domain" })).await?;
                summary.push(serde_json::json!({ "handle": handle, "status": "taken" }));
            }
            // New claim (cooling-off still applies).
            None => {
                let (cooling,): (i64,) = sqlx::query_as(
                    "SELECT count(*) FROM handles WHERE handle_key = $1 AND released_at > now() - ($2 || ' days')::interval",
                )
                .bind(&key)
                .bind(COOLING_OFF_DAYS.to_string())
                .fetch_one(pool)
                .await?;
                if cooling > 0 {
                    summary.push(serde_json::json!({ "handle": handle, "status": "cooling-off" }));
                    continue;
                }
                sqlx::query(
                    "INSERT INTO handles (handle, handle_key, email, anchor, listing_id, bind_method, verified_at, stale) \
                     VALUES ($1, $2, $3, 'domain', $4, $5, now(), false)",
                )
                .bind(&handle)
                .bind(&key)
                .bind(&domain)
                .bind(listing_id)
                .bind(bind_method)
                .execute(pool)
                .await?;
                log_action(pool, "claimed", &handle, &domain,
                    serde_json::json!({ "anchor": "domain", "listing_id": listing_id, "method": bind_method })).await?;
                summary.push(serde_json::json!({ "handle": handle, "status": "claimed", "bound": listing_id.is_some() }));
            }
        }
    }

    // Entries the record no longer declares: mark stale (released later by
    // the sweep, after the grace window).
    let removed: Vec<(String,)> = sqlx::query_as(
        "UPDATE handles SET stale = true \
         WHERE anchor = 'domain' AND email = $1 AND released_at IS NULL \
           AND stale = false AND NOT (handle_key = ANY($2)) \
         RETURNING handle",
    )
    .bind(&domain)
    .bind(&declared)
    .fetch_all(pool)
    .await?;
    for (h,) in &removed {
        log_action(pool, "stale", h, &domain,
            serde_json::json!({ "reason": "removed-from-record" })).await?;
        summary.push(serde_json::json!({ "handle": h, "status": "stale" }));
    }

    Ok(serde_json::json!({ "domain": domain, "handles": summary }))
}

/// Periodic re-verification (§3.2): re-sync every known domain; mark
/// domains whose records stopped resolving stale after the grace window;
/// release long-stale handles (cooling-off then applies as usual).
pub async fn domain_sweep(pool: &PgPool) {
    let domains: Vec<(String,)> = match sqlx::query_as(
        "SELECT DISTINCT email FROM handles WHERE anchor = 'domain' AND released_at IS NULL",
    )
    .fetch_all(pool)
    .await
    {
        Ok(d) => d,
        Err(e) => {
            log::error!("[catalog:domains] sweep query failed: {e}");
            return;
        }
    };
    for (domain,) in domains {
        match sync_domain(pool, &domain).await {
            Ok(_) => {}
            Err(e) => {
                log::warn!("[catalog:domains] sweep: {domain} unfetchable ({e})");
                // Unfetchable: stale after 7 days without verification.
                let marked: Result<Vec<(String,)>, _> = sqlx::query_as(
                    "UPDATE handles SET stale = true \
                     WHERE anchor = 'domain' AND email = $1 AND released_at IS NULL \
                       AND stale = false AND verified_at < now() - interval '7 days' \
                     RETURNING handle",
                )
                .bind(&domain)
                .fetch_all(pool)
                .await;
                if let Ok(rows) = marked {
                    for (h,) in rows {
                        let _ = log_action(pool, "stale", &h, &domain,
                            serde_json::json!({ "reason": "record-unfetchable" })).await;
                    }
                }
            }
        }
    }
    // Release anything stale past the 30-day grace.
    let released: Result<Vec<(String, String)>, _> = sqlx::query_as(
        "UPDATE handles SET released_at = now() \
         WHERE anchor = 'domain' AND released_at IS NULL AND stale = true \
           AND verified_at < now() - interval '30 days' \
         RETURNING handle, email",
    )
    .fetch_all(pool)
    .await;
    if let Ok(rows) = released {
        for (h, d) in rows {
            let _ = log_action(pool, "released", &h, &d,
                serde_json::json!({ "reason": "stale-past-grace" })).await;
            log::info!("[catalog:domains] released stale handle: {h}");
        }
    }
}

/// The public transparency log, newest first, with chain hashes.
pub async fn log_entries(pool: &PgPool, limit: i64) -> Result<Vec<serde_json::Value>, RegistrarError> {
    let rows: Vec<(i64, DateTime<Utc>, String, String, String, serde_json::Value, Option<String>, Option<String>)> =
        sqlx::query_as(
            "SELECT id, at, action, handle, email, detail, prev_hash, entry_hash \
             FROM handle_log ORDER BY id DESC LIMIT $1",
        )
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
