use std::sync::Arc;

use chrono::Duration;
use flashmart_contracts::{Clock, InventoryPort};
use flashmart_infra::{AppState, InMemoryEventBus, SystemClock, UuidV4Generator};
use flashmart_inventory_slices::port::PgInventoryPort;
use flashmart_server::{
    ApiDoc, ServerConfig, build_app, seed_demo_items, spawn_expiration_sweeper,
};
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;
use utoipa::OpenApi;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--export-openapi") {
        let json = serde_json::to_string_pretty(&ApiDoc::openapi())?;
        println!("{json}");
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cfg = ServerConfig::from_env();
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&cfg.database_url)
        .await?;

    sqlx::migrate!("../../migrations").run(&pool).await?;

    let clock = Arc::new(SystemClock);
    if cfg.seed_demo_items {
        let seeded = seed_demo_items(&pool, clock.now()).await?;
        tracing::info!(
            seeded_count = seeded,
            "demo flash items initialization completed"
        );
    }

    let id_gen = Arc::new(UuidV4Generator);
    let event_bus = Arc::new(InMemoryEventBus::new());
    let inventory_port = Arc::new(PgInventoryPort::new(
        pool.clone(),
        clock.clone(),
        id_gen.clone(),
        event_bus.clone(),
    ));

    let state = AppState::new(
        pool,
        clock,
        id_gen,
        inventory_port as Arc<dyn InventoryPort>,
        event_bus,
        Duration::seconds(cfg.payment_hold_secs),
    );

    let sweeper = spawn_expiration_sweeper(
        state.clone(),
        std::time::Duration::from_secs(cfg.expire_scan_interval_secs),
    );
    let app = build_app(state);
    let listener = tokio::net::TcpListener::bind(&cfg.listen_addr).await?;
    tracing::info!(
        listen_addr = %cfg.listen_addr,
        expire_scan_interval_secs = cfg.expire_scan_interval_secs,
        "flashmart-server listening"
    );
    let serve_res = axum::serve(listener, app).await;
    sweeper.abort();
    serve_res?;
    Ok(())
}
