//! AgentMesh connector — the first source connector.
//!
//! Harvests the mesh registry into listings and feeds live presence. The
//! catalog is a passive observer on the mesh: it connects, discovers, and
//! listens to presence events. It never registers as an agent and never
//! hosts anything.
//!
//! Two feeds compose:
//! - **Harvest sweep** (poll): `discover` returns every visible manifest with
//!   availability joined from presence — upserted as listings, presence
//!   refreshed. Poll makes the sweep self-healing: missed events wash out.
//! - **Presence events** (push): `presence.node_online` / `node_offline`
//!   from the registry's heartbeat monitor update liveness for all listings
//!   hosted by that node, between sweeps.
//!
//! Enabled by `MESH_NATS_URL`; optional `MESH_JWT` (guarded servers, e.g. the
//! public sandbox guest credential) + `MESH_SEED` (stable connector identity),
//! `MESH_POLL_SECS` (default 30). Connector failures never take the catalog
//! down — it serves whatever it has.

use std::time::Duration;

use agentmesh::{AgentMesh, ConnectOptions, DiscoverQuery};
use serde_json::Value;
use sqlx::postgres::PgPool;

use crate::store;

pub struct MeshConfig {
    pub nats_url: String,
    pub jwt: Option<String>,
    pub seed: Option<String>,
    pub poll: Duration,
}

pub fn config_from_env() -> Option<MeshConfig> {
    let nats_url = std::env::var("MESH_NATS_URL").ok()?;
    Some(MeshConfig {
        nats_url,
        jwt: std::env::var("MESH_JWT").ok(),
        seed: std::env::var("MESH_SEED").ok(),
        poll: Duration::from_secs(
            std::env::var("MESH_POLL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
        ),
    })
}

/// Run the connector forever: connect (with backoff), subscribe to presence
/// events, sweep the registry on an interval. Reconnects on any failure.
pub async fn run(pool: PgPool, cfg: MeshConfig) {
    let mut backoff = Duration::from_secs(2);
    loop {
        match connect_and_serve(&pool, &cfg).await {
            Ok(()) => backoff = Duration::from_secs(2),
            Err(e) => {
                log::error!("[catalog:mesh] connector error: {e}; reconnecting in {backoff:?}");
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(300));
    }
}

async fn connect_and_serve(
    pool: &PgPool,
    cfg: &MeshConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("[catalog:mesh] connecting to {}", cfg.nats_url);
    let mesh = AgentMesh::connect(
        &cfg.nats_url,
        ConnectOptions {
            agent_seed: cfg.seed.clone(),
            node_seed: None,
            jwt: cfg.jwt.clone(),
        },
    )
    .await?;
    log::info!("[catalog:mesh] connected as {}", mesh.id());

    // Presence push: node-level events fan out to every listing that node
    // hosts. The envelope's payload carries {domain, event_type, data:{node}}.
    let event_pool = pool.clone();
    mesh.subscribe("presence.*", move |data, env| {
        let event_type = env
            .payload
            .as_ref()
            .and_then(|p| p.get("event_type"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        let node = data
            .get("node")
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_string();
        if node.is_empty() {
            log::warn!("[catalog:mesh] presence event without node: {event_type}");
            return;
        }
        let state = match event_type.as_str() {
            "node_online" => "online",
            "node_offline" => "offline",
            other => {
                log::info!("[catalog:mesh] ignoring presence event: {other}");
                return;
            }
        };
        let pool = event_pool.clone();
        tokio::spawn(async move {
            match store::set_presence_by_node(&pool, &node, state).await {
                Ok(n) => log::info!("[catalog:mesh] presence: node {node} -> {state} ({n} listings)"),
                Err(e) => log::error!("[catalog:mesh] presence update failed for node {node}: {e}"),
            }
        });
    })
    .await?;
    log::info!("[catalog:mesh] subscribed to presence events");

    loop {
        if let Err(e) = harvest(pool, &mesh).await {
            // A failed sweep usually means the connection died — surface it
            // to the outer loop so we reconnect rather than sweeping into a void.
            return Err(e);
        }
        tokio::time::sleep(cfg.poll).await;
    }
}

/// One registry sweep: discover everything visible, upsert each manifest as a
/// listing, refresh its presence from the joined availability.
async fn harvest(
    pool: &PgPool,
    mesh: &AgentMesh,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let manifests = mesh
        .discover_raw(DiscoverQuery {
            limit: Some(500),
            ..Default::default()
        })
        .await?;
    log::info!("[catalog:mesh] harvest sweep: {} manifests", manifests.len());

    for m in &manifests {
        match upsert_manifest(pool, m).await {
            Ok(name) => log::debug!("[catalog:mesh] upserted: {name}"),
            // One malformed manifest must not fail the sweep.
            Err(e) => log::error!("[catalog:mesh] upsert failed for {:?}: {e}", m.get("id")),
        }
    }
    Ok(())
}

async fn upsert_manifest(
    pool: &PgPool,
    m: &Value,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let id = m
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("manifest missing id")?;
    let name = m.get("name").and_then(|v| v.as_str()).unwrap_or(id);
    let description = m
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Specialty claims from the manifest: capabilities as-is, each skill as
    // `skill:{id}`, plus skill tags. Namespacing stays thin — these are the
    // mesh's own claim vocabulary, searchable alongside manual claims.
    let mut specialties: Vec<String> = Vec::new();
    if let Some(caps) = m.get("capabilities").and_then(|v| v.as_array()) {
        specialties.extend(caps.iter().filter_map(|c| c.as_str().map(String::from)));
    }
    if let Some(skills) = m.get("skills").and_then(|v| v.as_array()) {
        for s in skills {
            if let Some(sid) = s.get("id").and_then(|v| v.as_str()) {
                specialties.push(format!("skill:{sid}"));
            }
            if let Some(tags) = s.get("tags").and_then(|v| v.as_array()) {
                specialties.extend(tags.iter().filter_map(|t| t.as_str().map(String::from)));
            }
        }
    }
    specialties.sort();
    specialties.dedup();

    // Operator-attested node standing (§9.7) — the registry joins it into
    // discover results; self-declared trust never reaches this field.
    let trust = m
        .get("node")
        .and_then(|n| n.get("profile"))
        .and_then(|p| p.get("trust_tier"))
        .and_then(|t| t.as_str());

    let listing_id =
        store::upsert_source_listing(pool, "agentmesh", id, name, description, m, &specialties, trust, "mesh")
            .await?;

    // Availability is presence-joined at discover time; absent = unknown.
    let state = m
        .get("availability")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    store::set_presence(pool, listing_id, state).await?;

    Ok(name.to_string())
}
