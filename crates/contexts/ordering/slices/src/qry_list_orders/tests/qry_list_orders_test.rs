use std::sync::Arc;

use axum::body::Body;
use chrono::{DateTime, Duration, TimeZone, Utc};
use flashmart_contracts::{InventoryPort, OrderLifecycleStatus};
use flashmart_infra::{
    AppState, FixedClock, InMemoryEventBus, SequenceIdGenerator, collect_active_routes,
};
use flashmart_primitives::{Currency, ItemKind};
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use rstest::rstest;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::super::handler::{FlashOrderRow, parse_order_status, row_to_summary};

fn sample_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap()
}

fn sample_row() -> FlashOrderRow {
    let now = sample_time();
    FlashOrderRow {
        id: Uuid::from_u128(0xAAAA_0001),
        buyer_id: "buyer-demo".to_owned(),
        item_id: Uuid::from_u128(0xBBBB_0001),
        item_title: "创世数字徽章".to_owned(),
        item_kind: "digital".to_owned(),
        quantity: 2,
        unit_price_cents: 9900,
        total_amount_cents: 19800,
        currency: "CNY".to_owned(),
        status: "pending_payment".to_owned(),
        version: 1,
        payment_deadline: now + Duration::minutes(15),
        created_at: now,
        paid_at: None,
        cancelled_at: None,
        expired_at: None,
        fulfillment_hint: None,
    }
}

#[rstest]
#[case("pending_payment", OrderLifecycleStatus::PendingPayment)]
#[case("paid", OrderLifecycleStatus::Paid)]
#[case("cancelled", OrderLifecycleStatus::Cancelled)]
#[case("expired", OrderLifecycleStatus::Expired)]
fn parse_order_status_accepts_known_values(
    #[case] raw: &str,
    #[case] expected: OrderLifecycleStatus,
) {
    assert_eq!(parse_order_status(raw).unwrap(), expected);
}

#[test]
fn parse_order_status_rejects_unknown_value() {
    assert!(parse_order_status("unknown_state").is_err());
}

#[test]
fn row_to_summary_maps_valid_row() {
    let row = sample_row();
    let id = row.id;
    let item_id = row.item_id;
    let summary = row_to_summary(row).expect("valid row should map");
    assert_eq!(summary.id.into_inner(), id);
    assert_eq!(summary.buyer_id.as_str(), "buyer-demo");
    assert_eq!(summary.item_id.into_inner(), item_id);
    assert_eq!(summary.item_title, "创世数字徽章");
    assert_eq!(summary.item_kind, ItemKind::Digital);
    assert_eq!(summary.quantity.get(), 2);
    assert_eq!(summary.unit_price_cents.get(), 9900);
    assert_eq!(summary.total_amount_cents.get(), 19800);
    assert_eq!(summary.currency, Currency::Cny);
    assert_eq!(summary.status, OrderLifecycleStatus::PendingPayment);
    assert_eq!(summary.version.get(), 1);
    assert!(summary.paid_at.is_none());
    assert!(summary.cancelled_at.is_none());
    assert!(summary.expired_at.is_none());
    assert!(summary.fulfillment_hint.is_none());
}

#[rstest]
#[case("buyer_id", "bad buyer!")]
#[case("item_kind", "virtual")]
#[case("currency", "USD")]
#[case("status", "refunding")]
fn row_to_summary_rejects_corrupted_strings(#[case] field: &str, #[case] bad_val: &str) {
    let mut row = sample_row();
    match field {
        "buyer_id" => row.buyer_id = bad_val.to_owned(),
        "item_kind" => row.item_kind = bad_val.to_owned(),
        "currency" => row.currency = bad_val.to_owned(),
        "status" => row.status = bad_val.to_owned(),
        _ => unreachable!(),
    }
    assert!(row_to_summary(row).is_err());
}

#[rstest]
#[case("quantity", 0)]
#[case("unit_price_cents", -1)]
#[case("total_amount_cents", -5)]
#[case("version", 0)]
fn row_to_summary_rejects_corrupted_numbers(#[case] field: &str, #[case] bad_val: i64) {
    let mut row = sample_row();
    match field {
        "quantity" => row.quantity = bad_val as i32,
        "unit_price_cents" => row.unit_price_cents = bad_val,
        "total_amount_cents" => row.total_amount_cents = bad_val,
        "version" => row.version = bad_val as i32,
        _ => unreachable!(),
    }
    assert!(row_to_summary(row).is_err());
}

struct StubInventoryPort;

#[async_trait::async_trait]
impl InventoryPort for StubInventoryPort {
    async fn list_flash_items(
        &self,
    ) -> Result<Vec<flashmart_contracts::FlashItemSnapshot>, flashmart_contracts::InventoryPortError>
    {
        Ok(vec![])
    }

    async fn get_flash_item(
        &self,
        _item_id: flashmart_primitives::ItemId,
    ) -> Result<
        Option<flashmart_contracts::FlashItemSnapshot>,
        flashmart_contracts::InventoryPortError,
    > {
        Ok(None)
    }

    async fn reserve_stock(
        &self,
        _req: flashmart_contracts::ReserveStockRequest,
    ) -> Result<flashmart_contracts::ReserveStockOutcome, flashmart_contracts::InventoryPortError>
    {
        unreachable!("unused in qry_list_orders tests")
    }

    async fn confirm_stock_deduction(
        &self,
        _req: flashmart_contracts::ConfirmStockDeductionRequest,
    ) -> Result<flashmart_contracts::ConfirmStockOutcome, flashmart_contracts::InventoryPortError>
    {
        unreachable!("unused in qry_list_orders tests")
    }

    async fn release_reserved_stock(
        &self,
        _req: flashmart_contracts::ReleaseReservedStockRequest,
    ) -> Result<flashmart_contracts::ReleaseStockOutcome, flashmart_contracts::InventoryPortError>
    {
        unreachable!("unused in qry_list_orders tests")
    }

    async fn return_deducted_stock(
        &self,
        _req: flashmart_contracts::ReturnDeductedStockRequest,
    ) -> Result<flashmart_contracts::ReturnStockOutcome, flashmart_contracts::InventoryPortError>
    {
        unreachable!("unused in qry_list_orders tests")
    }
}

fn test_app(pool: PgPool, now: DateTime<Utc>) -> axum::Router {
    crate::link();
    let clock = Arc::new(FixedClock::new(now));
    let id_gen = Arc::new(SequenceIdGenerator::new(1));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let state = AppState::new(
        pool,
        clock,
        id_gen,
        Arc::new(StubInventoryPort),
        event_bus,
        Duration::minutes(15),
    );
    collect_active_routes(state)
}

struct SeedOrder<'a> {
    id: Uuid,
    buyer_id: &'a str,
    title: &'a str,
    status: &'a str,
    created_at: DateTime<Utc>,
    paid_at: Option<DateTime<Utc>>,
    fulfillment_hint: Option<&'a str>,
}

async fn insert_order(pool: &PgPool, seed: SeedOrder<'_>) {
    sqlx::query(
        r#"
        INSERT INTO ordering.flash_orders (
            id, reservation_id, buyer_id, item_id, item_title, item_kind,
            quantity, unit_price_cents, total_amount_cents, currency, status, version,
            payment_deadline, created_at, paid_at, cancelled_at, expired_at, fulfillment_hint
        )
        VALUES ($1, $2, $3, $4, $5, 'digital', 1, 9900, 9900, 'CNY', $6, 1, $7, $8, $9, NULL, NULL, $10)
        "#,
    )
    .bind(seed.id)
    .bind(Uuid::new_v4())
    .bind(seed.buyer_id)
    .bind(Uuid::from_u128(0xBBBB_0001))
    .bind(seed.title)
    .bind(seed.status)
    .bind(seed.created_at + Duration::minutes(15))
    .bind(seed.created_at)
    .bind(seed.paid_at)
    .bind(seed.fulfillment_hint)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn list_orders_validates_buyer_header_format(pool: PgPool) {
    let app = test_app(pool, sample_time());

    let blank_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/orders")
                .header("X-Buyer-Id", "   ")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(blank_res.status(), StatusCode::BAD_REQUEST);
    let blank_json: Value =
        serde_json::from_slice(&blank_res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(blank_json["code"], "missing_required_header");

    let invalid_res = app
        .oneshot(
            Request::builder()
                .uri("/api/orders")
                .header("X-Buyer-Id", "invalid buyer!")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let invalid_json: Value =
        serde_json::from_slice(&invalid_res.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(invalid_json["code"], "validation_error");
    assert_eq!(invalid_json["errors"][0]["field"], "X-Buyer-Id");
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn list_orders_isolates_buyers_and_sorts_created_at_desc(pool: PgPool) {
    let now = sample_time();
    let older_id = Uuid::from_u128(0x1000_0001);
    let newer_id = Uuid::from_u128(0x1000_0002);
    let other_id = Uuid::from_u128(0x1000_0003);

    insert_order(
        &pool,
        SeedOrder {
            id: older_id,
            buyer_id: "buyer-a",
            title: "早期订单",
            status: "pending_payment",
            created_at: now - Duration::minutes(10),
            paid_at: None,
            fulfillment_hint: None,
        },
    )
    .await;
    insert_order(
        &pool,
        SeedOrder {
            id: newer_id,
            buyer_id: "buyer-a",
            title: "最新已付订单",
            status: "paid",
            created_at: now - Duration::minutes(2),
            paid_at: Some(now - Duration::minutes(1)),
            fulfillment_hint: Some("数字凭证已发放"),
        },
    )
    .await;
    insert_order(
        &pool,
        SeedOrder {
            id: other_id,
            buyer_id: "buyer-b",
            title: "其他买家订单",
            status: "pending_payment",
            created_at: now - Duration::minutes(1),
            paid_at: None,
            fulfillment_hint: None,
        },
    )
    .await;

    let app = test_app(pool, now);

    let res_a = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/orders")
                .header("X-Buyer-Id", "buyer-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_a.status(), StatusCode::OK);
    let json_a: Value =
        serde_json::from_slice(&res_a.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(
        DateTime::parse_from_rfc3339(json_a["server_time"].as_str().unwrap())
            .unwrap()
            .with_timezone(&Utc),
        now
    );
    let orders_a = json_a["orders"].as_array().unwrap();
    assert_eq!(orders_a.len(), 2);
    assert_eq!(orders_a[0]["id"], newer_id.to_string());
    assert_eq!(orders_a[0]["status"], "paid");
    assert_eq!(orders_a[0]["fulfillment_hint"], "数字凭证已发放");
    assert!(orders_a[0]["paid_at"].is_string());
    assert_eq!(orders_a[1]["id"], older_id.to_string());
    assert_eq!(orders_a[1]["status"], "pending_payment");

    let res_b = app
        .oneshot(
            Request::builder()
                .uri("/api/orders")
                .header("X-Buyer-Id", "buyer-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_b.status(), StatusCode::OK);
    let json_b: Value =
        serde_json::from_slice(&res_b.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let orders_b = json_b["orders"].as_array().unwrap();
    assert_eq!(orders_b.len(), 1);
    assert_eq!(orders_b[0]["id"], other_id.to_string());
    assert_eq!(orders_b[0]["buyer_id"], "buyer-b");
}
