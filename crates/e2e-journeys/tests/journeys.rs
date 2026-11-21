use std::sync::Arc;

use axum::body::Body;
use chrono::Duration;
use flashmart_contracts::InventoryPort;
use flashmart_infra::{AppState, FixedClock, InMemoryEventBus, SequenceIdGenerator};
use flashmart_inventory_slices::port::PgInventoryPort;
use flashmart_ordering_slices::cmd_expire_unpaid_order;
use flashmart_ordering_slices::cmd_expire_unpaid_order::contract::ExpireUnpaidOrdersCommand;
use flashmart_server::{build_app, run_expiration_sweep_once, seed_demo_items};
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

#[sqlx::test(migrations = "../../migrations")]
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

#[sqlx::test(migrations = "../../migrations")]
async fn e2e_demo_seed_full_http_lifecycle_and_expiration_restock(pool: PgPool) {
    let now = chrono::Utc::now();
    let clock = Arc::new(FixedClock::new(now));
    let id_gen = Arc::new(SequenceIdGenerator::new(100));
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

    // 1. 幂等初始化演示商品：首次写入 5 条，二次调用写入 0 条
    assert_eq!(seed_demo_items(&pool, now).await.unwrap(), 5);
    assert_eq!(seed_demo_items(&pool, now).await.unwrap(), 0);

    let app = build_app(state.clone());

    // 2. 验证健康检查与商品目录/详情查询
    let health_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(health_res.status(), StatusCode::OK);

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
    let items = list_json["items"].as_array().unwrap();
    assert_eq!(items.len(), 5);

    let item_id = "11111111-1111-4111-8111-111111111101";
    let detail_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/flash-items/{item_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail_res.status(), StatusCode::OK);
    let detail_json: Value =
        serde_json::from_slice(&detail_res.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(detail_json["available_stock"], 10);

    // 3. 下单、幂等重放、幂等冲突与限购拒绝
    let place_req = json!({ "item_id": item_id, "quantity": 1 }).to_string();
    let place_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/orders")
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "e2e-alice")
                .header("Idempotency-Key", "e2e-place-alice-1")
                .body(Body::from(place_req.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(place_res.status(), StatusCode::CREATED);
    let order_json: Value =
        serde_json::from_slice(&place_res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let order_id = order_json["id"].as_str().unwrap().to_owned();

    // 相同幂等键重放应返回同一笔订单
    let replay_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/orders")
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "e2e-alice")
                .header("Idempotency-Key", "e2e-place-alice-1")
                .body(Body::from(place_req))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(replay_res.status().is_success());
    let replay_json: Value =
        serde_json::from_slice(&replay_res.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(replay_json["id"], order_id);

    // 相同幂等键不同载荷触发 409 冲突
    let conflict_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/orders")
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "e2e-alice")
                .header("Idempotency-Key", "e2e-place-alice-1")
                .body(Body::from(
                    json!({ "item_id": item_id, "quantity": 2 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(conflict_res.status(), StatusCode::CONFLICT);

    // 同一买家换新幂等键重复下单触发单人限购拒绝
    let limit_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/orders")
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "e2e-alice")
                .header("Idempotency-Key", "e2e-place-alice-2")
                .body(Body::from(
                    json!({ "item_id": item_id, "quantity": 1 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(limit_res.status(), StatusCode::CONFLICT);

    // 4. 订单列表、详情查询与跨买家归属隔离校验
    let orders_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/orders")
                .header("X-Buyer-Id", "e2e-alice")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(orders_res.status(), StatusCode::OK);
    let orders_json: Value =
        serde_json::from_slice(&orders_res.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(orders_json["orders"].as_array().unwrap().len(), 1);

    let forbidden_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/orders/{order_id}"))
                .header("X-Buyer-Id", "e2e-bob")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(forbidden_res.status(), StatusCode::FORBIDDEN);

    // 5. 支付与 OCC 版本冲突校验
    let stale_pay_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/orders/{order_id}/pay"))
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "e2e-alice")
                .header("Idempotency-Key", "e2e-pay-stale")
                .body(Body::from(json!({ "version": 99 }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stale_pay_res.status(), StatusCode::CONFLICT);

    let pay_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/orders/{order_id}/pay"))
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "e2e-alice")
                .header("Idempotency-Key", "e2e-pay-alice-1")
                .body(Body::from(json!({ "version": 1 }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(pay_res.status(), StatusCode::OK);
    let paid_json: Value =
        serde_json::from_slice(&pay_res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(paid_json["status"], "paid");
    assert_eq!(paid_json["version"], 2);

    // 6. 买家 Bob 下单后超时未付，后台巡检回收并恢复可售库存
    let bob_place = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/orders")
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "e2e-bob")
                .header("Idempotency-Key", "e2e-place-bob-1")
                .body(Body::from(
                    json!({ "item_id": item_id, "quantity": 1 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bob_place.status(), StatusCode::CREATED);
    let bob_order: Value =
        serde_json::from_slice(&bob_place.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let bob_order_id = bob_order["id"].as_str().unwrap();

    // 此时 101 库存应为 8（Alice 已付 1 + Bob 预占 1）
    let mid_detail = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/flash-items/{item_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let mid_json: Value =
        serde_json::from_slice(&mid_detail.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(mid_json["available_stock"], 8);

    clock.advance(Duration::minutes(16));
    assert_eq!(run_expiration_sweep_once(&state).await, 1);

    let bob_get = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/orders/{bob_order_id}"))
                .header("X-Buyer-Id", "e2e-bob")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bob_get_json: Value =
        serde_json::from_slice(&bob_get.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(bob_get_json["status"], "expired");

    let after_sweep = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/flash-items/{item_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let after_json: Value =
        serde_json::from_slice(&after_sweep.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(after_json["available_stock"], 9);
}
