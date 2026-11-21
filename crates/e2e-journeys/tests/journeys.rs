use std::sync::Arc;

use axum::body::Body;
use chrono::Duration;
use flashmart_contracts::InventoryPort;
use flashmart_infra::{AppState, FixedClock, InMemoryEventBus, SequenceIdGenerator};
use flashmart_inventory_slices::port::PgInventoryPort;
use flashmart_ordering_slices::cmd_expire_unpaid_order;
use flashmart_ordering_slices::cmd_expire_unpaid_order::contract::ExpireUnpaidOrdersCommand;
use flashmart_server::build_app;
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "wip"]
async fn walking_skeleton_flash_sale_journey_place_pay_cancel_and_restock(pool: PgPool) {
    let now = chrono::Utc::now();
    let clock = Arc::new(FixedClock::new(now));
    let id_gen = Arc::new(SequenceIdGenerator::new(1));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let port = Arc::new(PgInventoryPort::new(
        pool.clone(),
        clock.clone(),
        id_gen.clone(),
        event_bus.clone(),
    ));
    let state = AppState::new(
        pool.clone(),
        clock.clone(),
        id_gen,
        port as Arc<dyn InventoryPort>,
        event_bus,
        Duration::minutes(15),
    );

    let item_id = Uuid::from_u128(0x1111_1111_1111_4111_8111_1111_1111_1101);
    sqlx::query(
        r#"
        INSERT INTO inventory.flash_items (
            id, title, description, item_kind, price_cents, currency,
            total_stock, available_stock, reserved_stock, confirmed_stock,
            returned_after_close_stock, per_user_limit, sale_starts_at, sale_ends_at
        )
        VALUES ($1, 'FlashMart 创世数字徽章', '限量数字通行证', 'digital', 9900, 'CNY',
                2, 2, 0, 0, 0, 1, $2, $3)
        "#,
    )
    .bind(item_id)
    .bind(now - Duration::minutes(10))
    .bind(now + Duration::minutes(60))
    .execute(&pool)
    .await
    .unwrap();

    let app = build_app(state.clone());

    // 1. 买家 A 浏览目录，确认可用库存为 2
    let list_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/flash-items")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list_res.status(), StatusCode::OK);
    let list_json: Value =
        serde_json::from_slice(&list_res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(list_json["items"][0]["available_stock"], 2);

    // 2. 买家 A 下单 1 件 -> available_stock 变为 1
    let place_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/orders")
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "buyer-alpha")
                .header("Idempotency-Key", "journey-place-1")
                .body(Body::from(
                    json!({ "item_id": item_id.to_string(), "quantity": 1 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(place_res.status().is_success());
    let order: Value =
        serde_json::from_slice(&place_res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let order_id = order["id"].as_str().unwrap();

    // 3. 买家 A 模拟结算 -> 生成数字兑换凭证
    let pay_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/orders/{order_id}/pay"))
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "buyer-alpha")
                .header("Idempotency-Key", "journey-pay-1")
                .body(Body::from(json!({ "version": 1 }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(pay_res.status(), StatusCode::OK);
    let paid: Value =
        serde_json::from_slice(&pay_res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(paid["status"], "paid");
    assert!(
        paid["fulfillment_hint"]
            .as_str()
            .unwrap()
            .contains("FM-DIGI-")
    );

    // 4. 买家 A 取消已支付订单 -> 模拟退款并回流库存至 2
    let cancel_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/orders/{order_id}/cancel"))
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "buyer-alpha")
                .header("Idempotency-Key", "journey-cancel-1")
                .body(Body::from(json!({ "version": 2 }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel_res.status(), StatusCode::OK);

    let after_cancel = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/flash-items/{item_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let item_after: Value =
        serde_json::from_slice(&after_cancel.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(item_after["available_stock"], 2);

    // 5. 买家 B 下单后超时未付 -> 过期回收释放库存
    let place_b = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/orders")
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "buyer-beta")
                .header("Idempotency-Key", "journey-place-b")
                .body(Body::from(
                    json!({ "item_id": item_id.to_string(), "quantity": 1 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(place_b.status().is_success());

    clock.advance(Duration::minutes(16));
    let expired = cmd_expire_unpaid_order::execute(&state, ExpireUnpaidOrdersCommand::default())
        .await
        .unwrap();
    assert_eq!(expired.expired_order_ids.len(), 1);
}
