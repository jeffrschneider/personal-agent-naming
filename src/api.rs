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
        .route("/browse", get(ui_index))
        .route("/spec", get(spec_page))
        .route("/spec.md", get(spec_raw))
        .route("/ui/style.css", get(ui_css))
        .route("/ui/app.js", get(ui_js))
        .route("/healthz", get(healthz))
        .route("/api/stats", get(stats))
        .route("/api/listings", get(list).post(submit))
        .route("/api/listings/:id", get(get_one))
        .route("/api/resolve", get(resolve_handle))
        .route("/.well-known/webfinger", get(webfinger))
        .route("/api/listings/mine", get(listings_mine))
        .route("/api/domains/sync", post(domains_sync))
        .route("/api/pair/start", post(pair_start))
        .route("/api/pair/complete", post(pair_complete))
        .route("/api/handles/log/checkpoint", get(log_checkpoint))
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
struct DomainSyncBody {
    domain: String,
}

/// Unauthenticated by design (PAN §3.2): the published record is the
/// authorization — only the domain's controller could have put it there.
async fn domains_sync(
    State(pool): State<PgPool>,
    Json(body): Json<DomainSyncBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let summary = registrar::sync_domain(&pool, &body.domain).await.map_err(reg_err)?;
    Ok(Json(serde_json::json!({ "ok": true, "sync": summary })))
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
    Json(body): Json<PairCompleteBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let handle = registrar::pair_complete(&pool, &body.code, &body.agent_id, &body.signature)
        .await
        .map_err(reg_err)?;
    Ok(Json(serde_json::json!({ "ok": true, "handle": handle })))
}

async fn log_checkpoint(
    State(pool): State<PgPool>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let cp = registrar::checkpoint(&pool).await.map_err(reg_err)?;
    Ok(Json(serde_json::json!({ "ok": true, "checkpoint": cp })))
}

/// The PAN card (§5.1): envelope + typed endpoints + verbatim manifest.
fn build_card(h: &registrar::Handle, listing: Option<&crate::model::Listing>) -> serde_json::Value {
    let mut endpoints: Vec<serde_json::Value> = Vec::new();
    if let Some(l) = listing {
        if l.source == "agentmesh" {
            endpoints.push(serde_json::json!({
                "protocol": "agentmesh",
                "agent_id": l.source_id,
                "node": l.manifest.get("node").and_then(|n| n.get("id")).cloned(),
            }));
        } else if let Some(url) = l.manifest.get("endpoint").and_then(|v| v.as_str()) {
            endpoints.push(serde_json::json!({ "protocol": l.protocol, "url": url }));
        }
    }
    serde_json::json!({
        "handle": h.handle,
        "anchor": h.anchor,
        "binding": h.bind_method,
        "claimed_at": h.created_at,
        "verified_at": h.verified_at,
        "stale": h.stale,
        "reserved": listing.is_none(),
        "presence": listing.map(|l| serde_json::json!({
            "state": l.presence, "last_seen_at": l.last_seen_at,
        })),
        "endpoints": endpoints,
        "manifest": listing.map(|l| l.manifest.clone()),
    })
}

#[derive(Deserialize)]
struct ResolveQuery {
    handle: String,
}

/// Public exact-string resolution: handle -> card (PAN §5). The bound
/// listing rides along for this registrar's own UI.
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
            Ok(Json(serde_json::json!({
                "ok": true,
                "card": build_card(&h, listing.as_ref()),
                "listing": listing,
            })))
        }
    }
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
            "urn:pan:anchor": h.anchor,
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

async fn handles_log(
    State(pool): State<PgPool>,
    Query(q): Query<LogQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let entries =
        registrar::log_entries(&pool, q.limit.unwrap_or(100)).await.map_err(reg_err)?;
    Ok(Json(serde_json::json!({ "ok": true, "entries": entries })))
}
