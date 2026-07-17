//! Embedded PostgreSQL bootstrap.
//!
//! When `DATABASE_URL` is not set, the catalog runs its own PostgreSQL server
//! via `postgresql_embedded`. Everything lives under one data root —
//! `CATALOG_DATA_DIR`, defaulting to `.data` in the working directory:
//! downloaded server binaries in `runtime/`, the cluster in `postgres/`.
//! The server listens on a random free localhost port. Deployed instances
//! set `DATABASE_URL` and none of this runs.

use std::path::PathBuf;

use postgresql_embedded::{PostgreSQL, Settings, Status};

const DB_NAME: &str = "catalog";

/// Start (installing/initializing if needed) an embedded PostgreSQL server
/// and return the handle plus a connection URL for the catalog database.
///
/// The returned [`PostgreSQL`] handle must be kept alive for the life of the
/// process — dropping it shuts the server down.
pub async fn start_embedded() -> (PostgreSQL, String) {
    // Absolute paths only: initdb/pg_ctl resolve relative paths from their
    // own working directory, not ours.
    let data_root = std::env::var("CATALOG_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".data"));
    let data_root = if data_root.is_relative() {
        std::env::current_dir()
            .expect("current_dir")
            .join(data_root)
    } else {
        data_root
    };
    std::fs::create_dir_all(&data_root).expect("could not create data root");
    log::info!("[catalog] data root: {}", data_root.display());

    let mut settings = Settings::default();
    settings.installation_dir = data_root.join("runtime");
    settings.data_dir = data_root.join("postgres");
    settings.password_file = data_root.join(".pgpass");
    // Dev-only credential for a localhost-bound server; same standing as the
    // committed docker-compose credential it replaced. Must stay fixed: initdb
    // bakes it into the data directory, so restarts reuse it.
    settings.username = "postgres".to_string();
    settings.password = "catalog_dev".to_string();
    settings.temporary = false; // persist data across restarts
    settings.port = 0; // any free port; the URL is derived after start
    settings.timeout = Some(std::time::Duration::from_secs(120)); // first run downloads + initdb

    let mut pg = PostgreSQL::new(settings);

    log::info!("[catalog] embedded postgres status: {:?}", pg.status());
    if pg.status() == Status::NotInstalled {
        log::info!("[catalog] first run: downloading PostgreSQL binaries (one-time)");
    }
    pg.setup().await.expect("embedded postgres setup failed");

    // A postmaster left over from a previous run (crash, kill) holds the data
    // dir on an unknown port. Stop it so we can start fresh on our own port;
    // a stale pid file makes stop fail, which is fine — start handles it.
    if pg.status() == Status::Started {
        log::warn!("[catalog] embedded postgres already running from a previous session; restarting it");
        if let Err(e) = pg.stop().await {
            log::warn!("[catalog] stop of leftover postgres failed ({e}); assuming stale pid file");
        }
    }

    pg.start().await.expect("embedded postgres start failed");
    log::info!(
        "[catalog] embedded postgres up on port {} (data: {})",
        pg.settings().port,
        pg.settings().data_dir.display()
    );

    if !pg
        .database_exists(DB_NAME)
        .await
        .expect("database_exists check failed")
    {
        log::info!("[catalog] creating database '{DB_NAME}'");
        pg.create_database(DB_NAME)
            .await
            .expect("create_database failed");
    }

    let url = pg.settings().url(DB_NAME);
    (pg, url)
}
