use std::sync::Arc;

use axum::body::Body;
use chrono::{Duration, TimeZone, Utc};
use flashmart_contracts::{InventoryPort, WindowStatus};
use flashmart_infra::{
    AppState, FixedClock, InMemoryEventBus, SequenceIdGenerator, collect_active_routes,
};
use flashmart_primitives::{Currency, ItemId, ItemKind, MoneyCents};
use http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use rstest::rstest;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::super::handler::{
    FlashItemRow, parse_item_id, row_to_snapshot, snapshot_to_detail_view,
};
use crate::port::PgInventoryPort;

fn base_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap()
}

fn build_test_state(pool: PgPool, clock: Arc<FixedClock>) -> (AppState, Arc<PgInventoryPort>) {
    crate::link();
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

struct TestItemSeed<'a> {
    id: Uuid,
    title: &'a str,
    item_kind: &'a str,
    price_cents: i64,
    total_stock: i32,
    available_stock: i32,
    starts_at: chrono::DateTime<Utc>,
    ends_at: chrono::DateTime<Utc>,
}

async fn insert_test_item(pool: &PgPool, seed: TestItemSeed<'_>) {
    let reserved_stock = seed.total_stock - seed.available_stock;
    sqlx::query(
        r#"
        INSERT INTO inventory.flash_items (
            id, title, description, item_kind, price_cents, currency,
            total_stock, available_stock, reserved_stock, confirmed_stock,
            returned_after_close_stock, per_user_limit, sale_starts_at, sale_ends_at
        )
        VALUES ($1, $2, '限量详情测试商品', $3, $4, 'CNY', $5, $6, $7, 0, 0, 2, $8, $9)
        "#,
    )
    .bind(seed.id)
    .bind(seed.title)
    .bind(seed.item_kind)
    .bind(seed.price_cents)
    .bind(seed.total_stock)
    .bind(seed.available_stock)
    .bind(reserved_stock)
    .bind(seed.starts_at)
    .bind(seed.ends_at)
    .execute(pool)
    .await
    .expect("seed flash item");
}

#[rstest]
#[case("11111111-1111-4111-8111-111111111101")]
#[case(" 11111111-1111-4111-8111-111111111101 ")]
#[case("11111111-1111-4111-8111-111111111101".to_uppercase())]
fn parse_item_id_accepts_valid_uuid(#[case] raw: impl AsRef<str>) {
    let parsed = parse_item_id(raw.as_ref(), "/api/flash-items/test").unwrap();
    assert_eq!(
        parsed,
        ItemId::new(Uuid::parse_str("11111111-1111-4111-8111-111111111101").unwrap())
    );
}

#[rstest]
#[case("")]
#[case("   ")]
#[case("not-a-uuid")]
#[case("11111111-1111-4111-8111-11111111110")]
fn parse_item_id_rejects_invalid_uuid_with_422_validation_error(#[case] raw: &str) {
    let instance = format!("/api/flash-items/{raw}");
    let problem = parse_item_id(raw, &instance).unwrap_err();
    assert_eq!(problem.status, 422);
    assert_eq!(problem.code, "validation_error");
    assert_eq!(
        problem.problem_type,
        "urn:flashmart:problem:validation_error"
    );
    assert_eq!(problem.instance, instance);
    let errors = problem.errors.as_ref().expect("validation field errors");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].field, "item_id");
}

#[rstest]
#[case(-5, WindowStatus::Upcoming, "digital", ItemKind::Digital)]
#[case(0, WindowStatus::Active, "physical", ItemKind::Physical)]
#[case(65, WindowStatus::Ended, "digital", ItemKind::Digital)]
fn row_mapping_preserves_window_status_and_fields(
    #[case] offset_mins: i64,
    #[case] expected_status: WindowStatus,
    #[case] raw_kind: &str,
    #[case] expected_kind: ItemKind,
) {
    let starts_at = base_time();
    let ends_at = starts_at + Duration::minutes(60);
    let now = starts_at + Duration::minutes(offset_mins);
    let id = Uuid::from_u128(0x2222_0001);

    let row = FlashItemRow {
        id,
        title: "创世徽章".to_owned(),
        description: "限定发售".to_owned(),
        item_kind: raw_kind.to_owned(),
        price_cents: 9900,
        currency: "CNY".to_owned(),
        total_stock: 10,
        available_stock: 4,
        per_user_limit: 2,
        sale_starts_at: starts_at,
        sale_ends_at: ends_at,
    };

    let snapshot = row_to_snapshot(row, now).unwrap();
    assert_eq!(snapshot.id, ItemId::new(id));
    assert_eq!(snapshot.item_kind, expected_kind);
    assert_eq!(snapshot.price_cents, MoneyCents::try_new(9900).unwrap());
    assert_eq!(snapshot.currency, Currency::Cny);
    assert_eq!(snapshot.available_stock, 4);
    assert_eq!(snapshot.window_status, expected_status);

    let view = snapshot_to_detail_view(snapshot, now);
    assert_eq!(view.server_time, now);
    assert_eq!(view.window_status, expected_status);
}

#[test]
fn row_to_snapshot_rejects_corrupted_currency_or_kind() {
    let starts_at = base_time();
    let ends_at = starts_at + Duration::minutes(30);
    let bad_kind = FlashItemRow {
        id: Uuid::new_v4(),
        title: "坏数据".to_owned(),
        description: "desc".to_owned(),
        item_kind: "unknown".to_owned(),
        price_cents: 100,
        currency: "CNY".to_owned(),
        total_stock: 1,
        available_stock: 1,
        per_user_limit: 1,
        sale_starts_at: starts_at,
        sale_ends_at: ends_at,
    };
    assert!(row_to_snapshot(bad_kind, starts_at).is_err());

    let bad_currency = FlashItemRow {
        id: Uuid::new_v4(),
        title: "坏数据".to_owned(),
        description: "desc".to_owned(),
        item_kind: "digital".to_owned(),
        price_cents: 100,
        currency: "USD".to_owned(),
        total_stock: 1,
        available_stock: 1,
        per_user_limit: 1,
        sale_starts_at: starts_at,
        sale_ends_at: ends_at,
    };
    assert!(row_to_snapshot(bad_currency, starts_at).is_err());
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn get_flash_item_endpoint_returns_200_detail_and_422_on_invalid_uuid(pool: PgPool) {
    let now = base_time();
    let clock = Arc::new(FixedClock::new(now));
    let item_id = Uuid::from_u128(0x3333_0001);

    insert_test_item(
        &pool,
        TestItemSeed {
            id: item_id,
            title: "钛金属限定键帽",
            item_kind: "physical",
            price_cents: 29900,
            total_stock: 20,
            available_stock: 13,
            starts_at: now - Duration::minutes(5),
            ends_at: now + Duration::minutes(55),
        },
    )
    .await;

    let (state, port) = build_test_state(pool, clock);
    let app = collect_active_routes(state);

    let ok_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/flash-items/{item_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(ok_res.status(), StatusCode::OK);
    let ok_body = ok_res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&ok_body).unwrap();
    assert_eq!(json["id"], item_id.to_string());
    assert_eq!(json["title"], "钛金属限定键帽");
    assert_eq!(json["item_kind"], "physical");
    assert_eq!(json["price_cents"], 29900);
    assert_eq!(json["currency"], "CNY");
    assert_eq!(json["total_stock"], 20);
    assert_eq!(json["available_stock"], 13);
    assert_eq!(json["per_user_limit"], 2);
    assert_eq!(json["window_status"], "active");
    assert_eq!(json["server_time"], serde_json::to_value(now).unwrap());

    let bad_res = app
        .oneshot(
            Request::builder()
                .uri("/api/flash-items/not-a-uuid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(bad_res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        bad_res
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/problem+json")
    );
    let bad_body = bad_res.into_body().collect().await.unwrap().to_bytes();
    let bad_json: Value = serde_json::from_slice(&bad_body).unwrap();
    assert_eq!(bad_json["code"], "validation_error");
    assert_eq!(bad_json["status"], 422);
    assert_eq!(bad_json["instance"], "/api/flash-items/not-a-uuid");
    assert_eq!(bad_json["errors"][0]["field"], "item_id");

    // 同步验证 PgInventoryPort::get_flash_item
    let found = port
        .get_flash_item(ItemId::new(item_id))
        .await
        .unwrap()
        .expect("existing flash item snapshot");
    assert_eq!(found.id, ItemId::new(item_id));
    assert_eq!(found.available_stock, 13);
    assert_eq!(found.window_status, WindowStatus::Active);

    let missing = port
        .get_flash_item(ItemId::new(Uuid::new_v4()))
        .await
        .unwrap();
    assert!(missing.is_none());
}
