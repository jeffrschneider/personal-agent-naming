//! PAN registrar — the reference registrar for Personal Agent Naming.
//!
//! It does one thing: claim a name (email anchor), bind it to an agent
//! (agent-key pairing), and resolve it to a card. No discovery, no
//! harvesting, no domain tier, no messaging. See PAN-SPEC.md.

mod api;
mod db;
mod model;
mod registrar;
mod store;

use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // PORT is the one canonical name (it's also Cloud Run's contract).
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080);

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
    log::info!("[pan] database ready");

    let app = api::router(pool);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    log::info!("[catalog] listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    axum::serve(listener, app).await.expect("serve");
}
