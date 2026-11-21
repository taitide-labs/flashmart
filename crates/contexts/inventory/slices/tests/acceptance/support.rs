use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use flashmart_contracts::InventoryPort;
use flashmart_infra::{
    AppState, FixedClock, InMemoryEventBus, SequenceIdGenerator, collect_active_routes,
};
use flashmart_inventory_slices::port::PgInventoryPort;
use sqlx::PgPool;
use uuid::Uuid;

pub fn base_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap()
}

pub fn test_state(pool: PgPool, clock: Arc<FixedClock>) -> (AppState, Arc<PgInventoryPort>) {
    flashmart_inventory_slices::link();
    let id_gen = Arc::new(SequenceIdGenerator::new(1));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let port = Arc::new(PgInventoryPort::new(
        pool.clone(),
        clock.clone(),
        id_gen.clone(),
        event_bus.clone(),
    ));
    let state = AppState::new(
        pool,
        clock,
        id_gen,
        port.clone() as Arc<dyn InventoryPort>,
        event_bus,
        Duration::minutes(15),
    );
    (state, port)
}

pub fn test_router(state: AppState) -> axum::Router {
    collect_active_routes(state)
}

pub struct TestItemSeed<'a> {
    pub id: Uuid,
    pub title: &'a str,
    pub item_kind: &'a str,
    pub price_cents: i64,
    pub total_stock: i32,
    pub per_user_limit: i32,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

pub async fn insert_flash_item(pool: &PgPool, seed: TestItemSeed<'_>) {
    sqlx::query(
        r#"
        INSERT INTO inventory.flash_items (
            id, title, description, item_kind, price_cents, currency,
            total_stock, available_stock, reserved_stock, confirmed_stock,
            returned_after_close_stock, per_user_limit, sale_starts_at, sale_ends_at
        )
        VALUES ($1, $2, '测试限量商品', $3, $4, 'CNY', $5, $5, 0, 0, 0, $6, $7, $8)
        "#,
    )
    .bind(seed.id)
    .bind(seed.title)
    .bind(seed.item_kind)
    .bind(seed.price_cents)
    .bind(seed.total_stock)
    .bind(seed.per_user_limit)
    .bind(seed.starts_at)
    .bind(seed.ends_at)
    .execute(pool)
    .await
    .expect("seed flash item");
}
