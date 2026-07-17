//! Agent Catalog — an ARD-compliant, presence-aware catalog of agents.
//!
//! The catalog indexes, verifies, and shows liveness; it never hosts agents.
//! Data enters through connectors (AgentMesh, A2A cards, manual submission);
//! this binary is the storage + search + read API core that connectors feed.
//!
//! v0 surface: health, listing search/read, manual submission. The mesh
//! harvester, probe runner, and ARD read interface build on this.

mod api;
mod db;
mod model;
mod store;

use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let port: u16 = std::env::var("CATALOG_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080);

    // DATABASE_URL points at a deployment's database. Without it, the catalog
    // runs its own embedded PostgreSQL — no external service, no Docker. The
    // handle must stay alive: dropping it stops the server.
    let (_embedded_pg, db_url) = match std::env::var("DATABASE_URL") {
        Ok(url) => (None, url),
        Err(_) => {
            let (pg, url) = db::start_embedded().await;
            (Some(pg), url)
        }
    };

    let pool = store::connect_with_retry(&db_url).await;
    store::migrate(&pool).await.expect("migrations failed");
    log::info!("[catalog] database ready");

    let app = api::router(pool);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    log::info!("[catalog] listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    axum::serve(listener, app).await.expect("serve");
}
