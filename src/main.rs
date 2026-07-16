//! Agent Catalog — an ARD-compliant, presence-aware catalog of agents.
//!
//! The catalog indexes, verifies, and shows liveness; it never hosts agents.
//! Data enters through connectors (AgentMesh, A2A cards, manual submission);
//! this binary is the storage + search + read API core that connectors feed.
//!
//! v0 surface: health, listing search/read, manual submission. The mesh
//! harvester, probe runner, and ARD read interface build on this.

mod api;
mod model;
mod store;

use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Committed dev default — matches docker-compose.yml. The public instance
    // overrides via the deployment environment; no credential is ever typed
    // from memory.
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://catalog:catalog_dev@localhost:5433/catalog".to_string());
    let port: u16 = std::env::var("CATALOG_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080);

    let pool = store::connect_with_retry(&db_url).await;
    store::migrate(&pool).await.expect("migrations failed");
    log::info!("[catalog] database ready");

    let app = api::router(pool);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    log::info!("[catalog] listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    axum::serve(listener, app).await.expect("serve");
}
