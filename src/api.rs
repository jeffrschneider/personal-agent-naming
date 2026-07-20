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

use crate::model::SubmitListing;
use crate::registrar::{self, RegistrarError};
use crate::store;

pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/", get(ui_index))
        // The public domain serves the static site at "/" (load balancer default
        // backend); /app is the door the LB routes to this UI.
        .route("/app", get(ui_index))
        .route("/app/", get(ui_index))
        .route("/spec", get(spec_page))
        .route("/spec.md", get(spec_raw))
        .route("/ui/style.css", get(ui_css))
        .route("/ui/app.js", get(ui_js))
        .route("/healthz", get(healthz))
        .route("/api/listings", post(submit))
        .route("/api/listings/:id", get(get_one))
        .route("/api/resolve", get(resolve_handle))
        .route("/api/operator", get(operator_get).post(operator_set))
        .route("/.well-known/webfinger", get(webfinger))
        .route("/api/listings/mine", get(listings_mine))
        .route("/api/pair/start", post(pair_start))
        .route("/api/pair/complete", post(pair_complete))
        .route("/api/agents/checkin", post(agent_checkin))
        .route("/api/handles/start", post(handles_start))
        .route("/api/handles/verify", post(handles_verify))
        .route("/api/handles/session-delegated", post(session_delegated))
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
/// The spec is a document, not code — it loads from disk (PAN_SPEC_PATH,
/// default ./PAN-SPEC.md) so editing it never requires a rebuild.
async fn read_spec() -> Result<String, (StatusCode, String)> {
    let path = std::env::var("PAN_SPEC_PATH").unwrap_or_else(|_| "PAN-SPEC.md".to_string());
    tokio::fs::read_to_string(&path).await.map_err(|e| {
        log::error!("[catalog] spec file unreadable at {path}: {e}");
        (StatusCode::INTERNAL_SERVER_ERROR, "spec file missing on this deployment".to_string())
    })
}

/// The PAN spec, rendered. The registrar publishes the protocol it speaks.
async fn spec_page() -> Result<axum::response::Html<String>, (StatusCode, String)> {
    let md = read_spec().await?;
    let parser = pulldown_cmark::Parser::new_ext(&md, pulldown_cmark::Options::all());
    let mut body = String::new();
    pulldown_cmark::html::push_html(&mut body, parser);
    Ok(axum::response::Html(format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Personal Agent Naming (PAN) · Agent Catalog</title>
<style>
  body {{ background:#0a0e14; color:#e5edf7; font-family:Inter,system-ui,sans-serif;
         line-height:1.65; margin:0; }}
  main {{ max-width:760px; margin:0 auto; padding:2.5rem 1.5rem 5rem; }}
  a {{ color:#d9a441; }} h1,h2,h3 {{ letter-spacing:-.01em; }}
  h1 {{ font-size:2rem; }} h2 {{ margin-top:2.2rem; border-bottom:1px solid #1e2937; padding-bottom:.3rem; }}
  code {{ background:#111722; border:1px solid #1e2937; border-radius:4px; padding:.08em .35em;
          font-family:"JetBrains Mono",ui-monospace,monospace; font-size:.85em; }}
  pre {{ background:#111722; border:1px solid #1e2937; border-radius:8px; padding:.9rem 1rem;
         overflow-x:auto; }} pre code {{ background:none; border:none; padding:0; }}
  table {{ border-collapse:collapse; width:100%; font-size:.9rem; }}
  th,td {{ border:1px solid #1e2937; padding:.45rem .6rem; text-align:left; }}
  blockquote {{ border-left:3px solid #d9a441; margin:0; padding:.1rem 1rem; color:#8695a8; }}
  .top {{ font-size:.85rem; }} hr {{ border:none; border-top:1px solid #1e2937; }}
</style></head><body><main>
<p class="top"><a href="/">← look up a handle</a> · <a href="/spec.md">raw markdown</a></p>
{body}
</main></body></html>"#
    )))
}

async fn spec_raw() -> Result<([(axum::http::HeaderName, &'static str); 1], String), (StatusCode, String)> {
    Ok(([(axum::http::header::CONTENT_TYPE, "text/markdown; charset=utf-8")], read_spec().await?))
}

async fn ui_css() -> ([(axum::http::HeaderName, &'static str); 1], &'static str) {
    ([(axum::http::header::CONTENT_TYPE, "text/css")], include_str!("../ui/style.css"))
}
async fn ui_js() -> ([(axum::http::HeaderName, &'static str); 1], &'static str) {
    ([(axum::http::header::CONTENT_TYPE, "text/javascript")], include_str!("../ui/app.js"))
}

async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
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
    headers: HeaderMap,
    Json(body): Json<SubmitListing>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // Manual submissions carry a verified owner (PAN §4.1) — it's what makes
    // them email-bindable, and it's the public instance's spam control.
    let email = session_from(&pool, &headers).await?;
    if body.name.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "name is required" })),
        ));
    }
    let id = store::submit(&pool, &body, &email)
        .await
        .map_err(|e| reg_err(e.into()))?;
    match id {
        Some(id) => {
            log::info!("[catalog] manual submission upserted: {} ({id}) by {email}", body.name);
            Ok(Json(serde_json::json!({ "ok": true, "id": id })))
        }
        None => Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "ok": false, "error": "that source_id belongs to another submitter" })),
        )),
    }
}

async fn listings_mine(
    State(pool): State<PgPool>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let email = session_from(&pool, &headers).await?;
    let listings = store::listings_mine(&pool, &email).await.map_err(|e| reg_err(e.into()))?;
    Ok(Json(serde_json::json!({ "ok": true, "listings": listings })))
}

fn internal(e: sqlx::Error) -> (StatusCode, String) {
    log::error!("[catalog] db error: {e}");
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

// ── the registrar: email-anchored handles ──────────────────────────────────

/// The caller's public IP, spoof-resistant. X-Forwarded-For is scanned from
/// the RIGHT (entries our infrastructure appended), skipping Google Front End
/// ranges (130.211.0.0/22, 35.191.0.0/16) and private/loopback addresses;
/// anything a client planted sits on the left and is never reached first.
/// Falls back to the socket peer (local dev has no proxy).
fn client_ip(headers: &HeaderMap, peer: Option<std::net::SocketAddr>) -> Option<String> {
    // Our own load balancer's public forwarding IP(s) also appear as the
    // rightmost X-Forwarded-For entry and must be skipped (verified in prod:
    // without this, the LB IP gets recorded as the client). Overridable for
    // other deployments via LB_PUBLIC_IPS (comma-separated).
    let lb_ips = std::env::var("LB_PUBLIC_IPS").unwrap_or_else(|_| "8.233.242.102".into());
    let is_lb = |ip: &std::net::IpAddr| lb_ips.split(',').any(|s| s.trim() == ip.to_string());
    fn is_infra(ip: &std::net::IpAddr) -> bool {
        match ip {
            std::net::IpAddr::V4(v4) => {
                let o = v4.octets();
                v4.is_private() || v4.is_loopback() || v4.is_link_local()
                    || (o[0] == 130 && o[1] == 211 && o[2] < 4)
                    || (o[0] == 35 && o[1] == 191)
            }
            std::net::IpAddr::V6(v6) => {
                v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00
            }
        }
    }
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        for part in xff.split(',').rev() {
            if let Ok(ip) = part.trim().parse::<std::net::IpAddr>() {
                if !is_infra(&ip) && !is_lb(&ip) {
                    return Some(ip.to_string());
                }
            }
        }
    }
    peer.map(|p| p.ip().to_string())
}

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

/// Deliver a verification code. With RESEND_API_KEY set, sends via Resend
/// (from RESEND_FROM, default onboarding@resend.dev); otherwise dev mode
/// logs it to the server console. The response says which delivery happened.
/// NOTE: the Resend path is unverified until a real API key exists.
async fn deliver_code(email: &str, code: &str) -> &'static str {
    let Ok(api_key) = std::env::var("RESEND_API_KEY") else {
        log::info!("[catalog:registrar] verification code for {email}: {code} (dev mode — no email provider configured)");
        return "console";
    };
    let from = std::env::var("RESEND_FROM").unwrap_or_else(|_| "onboarding@resend.dev".to_string());
    let body = serde_json::json!({
        "from": from,
        "to": [email],
        "subject": format!("{code} is your Agent Catalog verification code"),
        "text": format!("Your verification code is {code}. It expires in 15 minutes.\n\nIf you didn't request this, ignore this email."),
    });
    let result = reqwest::Client::new()
        .post("https://api.resend.com/emails")
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await;
    match result {
        Ok(r) if r.status().is_success() => {
            log::info!("[catalog:registrar] verification code emailed to {email}");
            "email"
        }
        Ok(r) => {
            log::error!("[catalog:registrar] Resend send failed for {email}: HTTP {}", r.status());
            "failed"
        }
        Err(e) => {
            log::error!("[catalog:registrar] Resend send failed for {email}: {e}");
            "failed"
        }
    }
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
    let delivery = deliver_code(&email, &code).await;
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
struct DelegatedBody {
    email: String,
}

/// Delegated verification (trusted server-to-server): a partner service that
/// has already verified the user's email — the AgentMesh control plane, whose
/// account session proves the same email (core §4.9) — mints a PAN session
/// without a second email round trip, then claims/binds a name on the user's
/// behalf. Authorized by a shared secret in `X-Delegate-Secret`, NOT a user
/// session. Disabled unless `PAN_DELEGATE_SECRET` is set. The name stays a
/// normal PAN handle: portability and the direct two-service path are unchanged.
async fn session_delegated(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(body): Json<DelegatedBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let expected = std::env::var("PAN_DELEGATE_SECRET").ok().filter(|s| !s.is_empty()).ok_or_else(|| {
        (StatusCode::NOT_IMPLEMENTED, Json(serde_json::json!({ "ok": false, "error": "delegation not configured" })))
    })?;
    let provided = headers.get("x-delegate-secret").and_then(|v| v.to_str().ok()).unwrap_or("");
    // Constant-time comparison so a wrong secret leaks no length/timing signal.
    let a = provided.as_bytes();
    let b = expected.as_bytes();
    let equal = a.len() == b.len() && a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0;
    if !equal {
        return Err((StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "ok": false, "error": "invalid delegate secret" }))));
    }
    let token = registrar::mint_session(&pool, &body.email).await.map_err(reg_err)?;
    Ok(Json(serde_json::json!({ "ok": true, "token": token })))
}

#[derive(Deserialize)]
struct ClaimBody {
    name: String,
    #[serde(default)]
    listing_id: Option<Uuid>,
    /// PAN v0.3: every operator has a required public display name. Provide
    /// it here on (at least) the first claim; later claims inherit it.
    #[serde(default)]
    operator_name: Option<String>,
}

async fn handles_claim(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(body): Json<ClaimBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let email = session_from(&pool, &headers).await?;
    if let Some(ref n) = body.operator_name {
        registrar::operator_set(&pool, &email, n).await.map_err(reg_err)?;
    } else if registrar::operator_get(&pool, &email).await.map_err(reg_err)?.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "ok": false,
                "error": "operator_name required: PAN requires a public display name for the operator (your chosen label, shown on your handles' cards)"
            })),
        ));
    }
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
    // Owner-only provenance: attach "last connected from" per bound handle.
    // This never appears on the public card (resolve/build_card).
    let ids: Vec<Uuid> = handles.iter().filter_map(|h| h.listing_id).collect();
    let seen = registrar::last_seen_ips(&pool, &ids).await.map_err(reg_err)?;
    let rows: Vec<serde_json::Value> = handles
        .iter()
        .map(|h| {
            let mut v = serde_json::to_value(h).unwrap_or_default();
            if let (Some(id), Some(obj)) = (h.listing_id, v.as_object_mut()) {
                if let Some((ip, at, kind, host, platform, first, enc_key)) = seen.get(&id) {
                    obj.insert("last_seen_ip".into(), serde_json::json!(ip));
                    obj.insert("last_seen_ip_at".into(), serde_json::json!(at));
                    obj.insert("agent_kind".into(), serde_json::json!(kind));
                    obj.insert("agent_host".into(), serde_json::json!(host));
                    obj.insert("agent_platform".into(), serde_json::json!(platform));
                    obj.insert("first_connected_at".into(), serde_json::json!(first));
                    obj.insert("encryption_key".into(), serde_json::json!(enc_key));
                }
            }
            v
        })
        .collect();
    Ok(Json(serde_json::json!({ "ok": true, "email": email, "handles": rows })))
}

#[derive(Deserialize)]
struct PairStartBody {
    handle: String,
}

async fn pair_start(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(body): Json<PairStartBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let email = session_from(&pool, &headers).await?;
    let (code, expires_at) =
        registrar::pair_start(&pool, &email, &body.handle).await.map_err(reg_err)?;
    Ok(Json(serde_json::json!({ "ok": true, "code": code, "expires_at": expires_at })))
}

#[derive(Deserialize)]
struct PairCompleteBody {
    code: String,
    agent_id: String,
    signature: String,
}

/// Unauthenticated by design (PAN §4.2): code + agent-key signature are the
/// two proofs, and the binding is their intersection.
async fn pair_complete(
    State(pool): State<PgPool>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<PairCompleteBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let handle = registrar::pair_complete(&pool, &body.code, &body.agent_id, &body.signature)
        .await
        .map_err(reg_err)?;
    // Owner-facing provenance: the pairing request comes from the agent's own
    // machine, so its source IP is "last connected from".
    if let Some(ip) = client_ip(&headers, Some(peer)) {
        registrar::record_agent_seen(&pool, &body.agent_id, &ip, &Default::default())
            .await
            .map_err(reg_err)?;
    }
    Ok(Json(serde_json::json!({ "ok": true, "handle": handle })))
}

#[derive(Deserialize)]
struct CheckinBody {
    agent_id: String,
    ts: i64,
    signature: String,
    #[serde(flatten)]
    profile: registrar::AgentProfile,
}

/// Signed agent check-in (adapter start): refreshes "last connected from".
/// The response echoes what was recorded so callers can self-verify.
async fn agent_checkin(
    State(pool): State<PgPool>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<CheckinBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let Some(ip) = client_ip(&headers, Some(peer)) else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "could not determine caller address" })),
        ));
    };
    registrar::checkin(&pool, &body.agent_id, body.ts, &body.signature, &ip, &body.profile)
        .await
        .map_err(reg_err)?;
    log::info!("[catalog:registrar] check-in: {} from {ip}", body.agent_id);
    Ok(Json(serde_json::json!({ "ok": true, "recorded_ip": ip })))
}

/// The PAN card (§5.1): the endpoints are the address. Presence is optional
/// and only present when the registrar actually observes it.
fn build_card(
    h: &registrar::Handle,
    listing: Option<&crate::model::Listing>,
    operator_name: Option<&str>,
    agent_decl: Option<&registrar::SeenRow>,
) -> serde_json::Value {
    // The declared agent profile (kind/host/platform) is public by decision:
    // self-declared, User-Agent trust class. IP and timestamps from the same
    // row stay owner-only and are NOT read here.
    let agent = agent_decl.and_then(|(_, _, kind, host, platform, _, _)| {
        if kind.is_some() || host.is_some() || platform.is_some() {
            Some(serde_json::json!({
                "kind": kind, "host": host, "platform": platform,
                "declared": true,
            }))
        } else {
            None
        }
    });
    // The encryption key (SPEC 4.3 / PAN SPEC 5.1): public so a correspondent
    // can seal content to this agent before first contact. A capability the
    // card advertises, not an address; absent means cleartext only.
    let encryption_key = agent_decl.and_then(|(_, _, _, _, _, _, ek)| ek.clone());
    let mut endpoints: Vec<serde_json::Value> = Vec::new();
    if let Some(l) = listing {
        // A key-bearing agent: the key is the address (its mesh inbox).
        if matches!(l.source.as_str(), "agent" | "agentmesh") {
            endpoints.push(serde_json::json!({
                "protocol": "agentmesh",
                "agent_id": l.source_id,
                "node": l.manifest.get("node").and_then(|n| n.get("id")).cloned(),
            }));
        } else if let Some(url) = l.manifest.get("endpoint").and_then(|v| v.as_str()) {
            endpoints.push(serde_json::json!({ "protocol": l.protocol, "url": url }));
        }
    }
    // Presence only where observed; PAN builds no presence subsystem.
    let presence = listing.and_then(|l| {
        l.presence.as_ref().map(|state| serde_json::json!({
            "state": state, "last_seen_at": l.last_seen_at,
        }))
    });
    serde_json::json!({
        "handle": h.handle,
        // The operator's chosen public label, set under the verified email
        // session (PAN v0.3). Anchored, logged, consistent across the owner's
        // handles — but NOT verified identity.
        "operator": operator_name.map(|n| serde_json::json!({ "name": n })),
        "agent": agent,
        "encryption_key": encryption_key,
        "binding": h.bind_method,
        "claimed_at": h.created_at,
        "reserved": listing.is_none(),
        "presence": presence,
        "endpoints": endpoints,
    })
}

#[derive(Deserialize)]
struct ResolveQuery {
    handle: Option<String>,
    /// Reverse resolution (PAN v0.3): look up by agent key instead. The key
    /// is public (it rides on every signed envelope), so this exposes nothing
    /// a forward resolve would not.
    agent_id: Option<String>,
}

/// Public exact-string resolution: handle -> card (PAN §5), or agent_id ->
/// card (reverse). The bound listing rides along for this registrar's own UI.
async fn resolve_handle(
    State(pool): State<PgPool>,
    Query(q): Query<ResolveQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let found = match (&q.handle, &q.agent_id) {
        (Some(h), _) => registrar::resolve(&pool, h).await.map_err(reg_err)?,
        (None, Some(a)) => registrar::resolve_by_agent(&pool, a).await.map_err(reg_err)?,
        (None, None) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "ok": false, "error": "pass handle= or agent_id=" })),
            ))
        }
    };
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
            let operator = registrar::operator_get(&pool, &h.email).await.map_err(reg_err)?;
            let decl = match h.listing_id {
                Some(id) => registrar::last_seen_ips(&pool, &[id])
                    .await
                    .map_err(reg_err)?
                    .remove(&id),
                None => None,
            };
            Ok(Json(serde_json::json!({
                "ok": true,
                "card": build_card(&h, listing.as_ref(), operator.as_deref(), decl.as_ref()),
                "listing": listing,
            })))
        }
    }
}

#[derive(Deserialize)]
struct OperatorBody {
    name: String,
}

/// Set/update the operator display name (session-scoped, logged).
async fn operator_set(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(body): Json<OperatorBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let email = session_from(&pool, &headers).await?;
    let name = registrar::operator_set(&pool, &email, &body.name).await.map_err(reg_err)?;
    Ok(Json(serde_json::json!({ "ok": true, "name": name })))
}

/// Read the caller's own operator record (for UI prefill).
async fn operator_get(
    State(pool): State<PgPool>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let email = session_from(&pool, &headers).await?;
    let name = registrar::operator_get(&pool, &email).await.map_err(reg_err)?;
    Ok(Json(serde_json::json!({ "ok": true, "name": name })))
}

#[derive(Deserialize)]
struct WebFingerQuery {
    resource: String,
}

/// WebFinger (RFC 7033, PAN §5.2): a handle is a valid acct: URI.
async fn webfinger(
    State(pool): State<PgPool>,
    Query(q): Query<WebFingerQuery>,
) -> Result<([(axum::http::HeaderName, &'static str); 1], Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let handle = q.resource.strip_prefix("acct:").unwrap_or(&q.resource);
    let found = registrar::resolve(&pool, handle).await.map_err(reg_err)?;
    let Some(h) = found else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "ok": false, "error": "no agent by that name" })),
        ));
    };
    let jrd = serde_json::json!({
        "subject": format!("acct:{}", h.handle),
        "properties": {
            "urn:pan:binding": h.bind_method,
        },
        "links": [{
            "rel": "urn:pan:card",
            "type": "application/json",
            "href": format!("/api/resolve?handle={}", urlencode(&h.handle)),
        }],
    });
    Ok(([(axum::http::header::CONTENT_TYPE, "application/jrd+json")], Json(jrd)))
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Deserialize)]
struct LogQuery {
    #[serde(default)]
    limit: Option<i64>,
}

/// Owner-scoped: the caller's own handle history, behind the verified
/// session. The log is not world-readable (PAN §6).
async fn handles_log(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Query(q): Query<LogQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let email = session_from(&pool, &headers).await?;
    let entries =
        registrar::log_entries(&pool, &email, q.limit.unwrap_or(100)).await.map_err(reg_err)?;
    Ok(Json(serde_json::json!({ "ok": true, "entries": entries })))
}
