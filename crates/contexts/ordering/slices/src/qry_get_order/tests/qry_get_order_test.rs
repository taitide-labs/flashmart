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

use super::super::handler::{FlashOrderRow, parse_order_id, parse_order_status, row_to_detail};

fn sample_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap()
}

fn sample_row() -> FlashOrderRow {
    let now = sample_time();
    FlashOrderRow {
        id: Uuid::from_u128(0xAAAA_0001),
        buyer_id: "buyer-demo".to_owned(),
        item_id: Uuid::from_u128(0xBBBB_0001),
        item_title: "钛金纪念机械键盘".to_owned(),
        item_kind: "physical".to_owned(),
        quantity: 1,
        unit_price_cents: 49900,
        total_amount_cents: 49900,
        currency: "CNY".to_owned(),
        status: "paid".to_owned(),
        version: 2,
        payment_deadline: now + Duration::minutes(15),
        created_at: now,
        paid_at: Some(now + Duration::minutes(1)),
        cancelled_at: None,
        expired_at: None,
        fulfillment_hint: Some("将于 24 小时内安排顺丰发货".to_owned()),
    }
}

#[test]
fn parse_order_id_accepts_valid_uuid_with_trim() {
    let raw_id = Uuid::from_u128(0xAAAA_0001);
    let parsed = parse_order_id(&format!("  {raw_id}  "), "/api/orders/test").unwrap();
    assert_eq!(parsed.into_inner(), raw_id);
}

#[rstest]
#[case("")]
#[case("   ")]
#[case("not-a-uuid")]
fn parse_order_id_rejects_invalid_uuid(#[case] raw: &str) {
    let instance = format!("/api/orders/{raw}");
    let problem = parse_order_id(raw, &instance).unwrap_err();
    assert_eq!(problem.status, StatusCode::UNPROCESSABLE_ENTITY.as_u16());
    assert_eq!(problem.code, "validation_error");
    assert_eq!(problem.instance, instance);
    let errors = problem.errors.as_ref().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].field, "order_id");
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
fn row_to_detail_maps_valid_row() {
    let row = sample_row();
    let id = row.id;
    let item_id = row.item_id;
    let paid_at = row.paid_at;
    let detail = row_to_detail(row).expect("valid row should map");
    assert_eq!(detail.id.into_inner(), id);
    assert_eq!(detail.buyer_id.as_str(), "buyer-demo");
    assert_eq!(detail.item_id.into_inner(), item_id);
    assert_eq!(detail.item_title, "钛金纪念机械键盘");
    assert_eq!(detail.item_kind, ItemKind::Physical);
    assert_eq!(detail.quantity.get(), 1);
    assert_eq!(detail.unit_price_cents.get(), 49900);
    assert_eq!(detail.total_amount_cents.get(), 49900);
    assert_eq!(detail.currency, Currency::Cny);
    assert_eq!(detail.status, OrderLifecycleStatus::Paid);
    assert_eq!(detail.version.get(), 2);
    assert_eq!(detail.paid_at, paid_at);
    assert!(detail.cancelled_at.is_none());
    assert!(detail.expired_at.is_none());
    assert_eq!(
        detail.fulfillment_hint.as_deref(),
        Some("将于 24 小时内安排顺丰发货")
    );
}

#[rstest]
#[case("buyer_id", "bad buyer!")]
#[case("item_kind", "virtual")]
#[case("currency", "USD")]
#[case("status", "refunding")]
fn row_to_detail_rejects_corrupted_strings(#[case] field: &str, #[case] bad_val: &str) {
    let mut row = sample_row();
    match field {
        "buyer_id" => row.buyer_id = bad_val.to_owned(),
        "item_kind" => row.item_kind = bad_val.to_owned(),
        "currency" => row.currency = bad_val.to_owned(),
        "status" => row.status = bad_val.to_owned(),
        _ => unreachable!(),
    }
    assert!(row_to_detail(row).is_err());
}

#[rstest]
#[case("quantity", 0)]
#[case("unit_price_cents", -1)]
#[case("total_amount_cents", -5)]
#[case("version", 0)]
fn row_to_detail_rejects_corrupted_numbers(#[case] field: &str, #[case] bad_val: i64) {
    let mut row = sample_row();
    match field {
        "quantity" => row.quantity = bad_val as i32,
        "unit_price_cents" => row.unit_price_cents = bad_val,
        "total_amount_cents" => row.total_amount_cents = bad_val,
        "version" => row.version = bad_val as i32,
        _ => unreachable!(),
    }
    assert!(row_to_detail(row).is_err());
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
        unreachable!("unused in qry_get_order tests")
    }

    async fn confirm_stock_deduction(
        &self,
        _req: flashmart_contracts::ConfirmStockDeductionRequest,
    ) -> Result<flashmart_contracts::ConfirmStockOutcome, flashmart_contracts::InventoryPortError>
    {
        unreachable!("unused in qry_get_order tests")
    }

    async fn release_reserved_stock(
        &self,
        _req: flashmart_contracts::ReleaseReservedStockRequest,
    ) -> Result<flashmart_contracts::ReleaseStockOutcome, flashmart_contracts::InventoryPortError>
    {
        unreachable!("unused in qry_get_order tests")
    }

    async fn return_deducted_stock(
        &self,
        _req: flashmart_contracts::ReturnDeductedStockRequest,
    ) -> Result<flashmart_contracts::ReturnStockOutcome, flashmart_contracts::InventoryPortError>
    {
        unreachable!("unused in qry_get_order tests")
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

async fn insert_sample_order(pool: &PgPool, order_id: Uuid, buyer_id: &str, now: DateTime<Utc>) {
    sqlx::query(
        r#"
        INSERT INTO ordering.flash_orders (
            id, reservation_id, buyer_id, item_id, item_title, item_kind,
            quantity, unit_price_cents, total_amount_cents, currency, status, version,
            payment_deadline, created_at, paid_at, cancelled_at, expired_at, fulfillment_hint
        )
        VALUES ($1, $2, $3, $4, '创世数字徽章', 'digital', 2, 9900, 19800, 'CNY', 'paid', 2, $5, $6, $7, NULL, NULL, '数字凭证已发放')
        "#,
    )
    .bind(order_id)
    .bind(Uuid::new_v4())
    .bind(buyer_id)
    .bind(Uuid::from_u128(0xBBBB_0001))
    .bind(now + Duration::minutes(15))
    .bind(now)
    .bind(now + Duration::minutes(1))
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn get_order_validates_buyer_header_and_order_uuid(pool: PgPool) {
    let app = test_app(pool, sample_time());
    let order_id = Uuid::new_v4();

    let missing_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/orders/{order_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_res.status(), StatusCode::BAD_REQUEST);
    let missing_json: Value =
        serde_json::from_slice(&missing_res.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(missing_json["code"], "missing_required_header");

    let bad_buyer_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/orders/{order_id}"))
                .header("X-Buyer-Id", "bad buyer!")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_buyer_res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bad_buyer_json: Value = serde_json::from_slice(
        &bad_buyer_res
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
    )
    .unwrap();
    assert_eq!(bad_buyer_json["code"], "validation_error");
    assert_eq!(bad_buyer_json["errors"][0]["field"], "X-Buyer-Id");

    let bad_id_res = app
        .oneshot(
            Request::builder()
                .uri("/api/orders/not-a-uuid")
                .header("X-Buyer-Id", "buyer-demo")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_id_res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bad_id_json: Value =
        serde_json::from_slice(&bad_id_res.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(bad_id_json["code"], "validation_error");
    assert_eq!(bad_id_json["errors"][0]["field"], "order_id");
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn get_order_enforces_ownership_and_returns_detail(pool: PgPool) {
    let now = sample_time();
    let order_id = Uuid::from_u128(0x2000_0001);
    insert_sample_order(&pool, order_id, "buyer-owner", now).await;

    let app = test_app(pool, now);

    let forbidden_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/orders/{order_id}"))
                .header("X-Buyer-Id", "buyer-other")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(forbidden_res.status(), StatusCode::FORBIDDEN);
    let forbidden_json: Value = serde_json::from_slice(
        &forbidden_res
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
    )
    .unwrap();
    assert_eq!(forbidden_json["code"], "order_ownership_mismatch");

    let ok_res = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/orders/{order_id}"))
                .header("X-Buyer-Id", "buyer-owner")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok_res.status(), StatusCode::OK);
    let ok_json: Value =
        serde_json::from_slice(&ok_res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(ok_json["id"], order_id.to_string());
    assert_eq!(ok_json["buyer_id"], "buyer-owner");
    assert_eq!(ok_json["item_id"], Uuid::from_u128(0xBBBB_0001).to_string());
    assert_eq!(ok_json["item_title"], "创世数字徽章");
    assert_eq!(ok_json["item_kind"], "digital");
    assert_eq!(ok_json["quantity"], 2);
    assert_eq!(ok_json["unit_price_cents"], 9900);
    assert_eq!(ok_json["total_amount_cents"], 19800);
    assert_eq!(ok_json["currency"], "CNY");
    assert_eq!(ok_json["status"], "paid");
    assert_eq!(ok_json["version"], 2);
    assert!(ok_json["paid_at"].is_string());
    assert!(ok_json["cancelled_at"].is_null());
    assert!(ok_json["expired_at"].is_null());
    assert_eq!(ok_json["fulfillment_hint"], "数字凭证已发放");
}
