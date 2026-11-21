use std::sync::Arc;

use axum::body::Body;
use chrono::Duration;
use flashmart_infra::FixedClock;
use flashmart_ordering_slices::cmd_expire_unpaid_order;
use flashmart_ordering_slices::cmd_expire_unpaid_order::contract::ExpireUnpaidOrdersCommand;
use flashmart_primitives::{ItemId, ItemKind};
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::support::{SeedItem, base_time, test_router, test_state};

#[sqlx::test(migrations = "../../../../migrations")]
async fn place_order_enforces_idempotency_limit_and_compensation(pool: PgPool) {
    let now = base_time();
    let clock = Arc::new(FixedClock::new(now));
    let (state, port) = test_state(pool, clock);
    let item_id = ItemId::new(Uuid::from_u128(0xBBBB_0001));
    port.seed_item(SeedItem {
        item_id,
        title: "创世数字徽章",
        kind: ItemKind::Digital,
        price_cents: 9900,
        total_stock: 5,
        per_user_limit: 1,
        starts_at: now - Duration::minutes(10),
        ends_at: now + Duration::minutes(60),
    });
    let app = test_router(state);

    // 1. 首次下单成功
    let req_body = json!({ "item_id": item_id.as_uuid().to_string(), "quantity": 1 });
    let res1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/orders")
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "buyer-demo")
                .header("Idempotency-Key", "place-k1")
                .body(Body::from(req_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(res1.status().is_success());
    let created: Value =
        serde_json::from_slice(&res1.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(created["status"], "pending_payment");
    assert_eq!(created["version"], 1);

    // 2. 相同幂等键 + 相同参数重放返回同一订单，不重复扣库存
    let replay = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/orders")
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "buyer-demo")
                .header("Idempotency-Key", "place-k1")
                .body(Body::from(req_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(replay.status().is_success());
    let replayed: Value =
        serde_json::from_slice(&replay.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(replayed["id"], created["id"]);

    // 3. 相同幂等键 + 不同参数返回 409 idempotency_key_conflict
    let conflict = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/orders")
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "buyer-demo")
                .header("Idempotency-Key", "place-k1")
                .body(Body::from(
                    json!({ "item_id": item_id.as_uuid().to_string(), "quantity": 2 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);

    // 4. 换用新幂等键再次抢购同一限购 1 件的商品 -> 超限购拒止
    let over_limit = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/orders")
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "buyer-demo")
                .header("Idempotency-Key", "place-k2")
                .body(Body::from(req_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(over_limit.status(), StatusCode::CONFLICT);
    let err: Value =
        serde_json::from_slice(&over_limit.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(err["code"], "purchase_limit_exceeded");
}

#[sqlx::test(migrations = "../../../../migrations")]
#[ignore = "wip"]
async fn pay_and_cancel_enforce_occ_ownership_and_release_quota(pool: PgPool) {
    let now = base_time();
    let clock = Arc::new(FixedClock::new(now));
    let (state, port) = test_state(pool, clock);
    let item_id = ItemId::new(Uuid::from_u128(0xBBBB_0002));
    port.seed_item(SeedItem {
        item_id,
        title: "机械旋钮宏键盘",
        kind: ItemKind::Physical,
        price_cents: 49900,
        total_stock: 3,
        per_user_limit: 1,
        starts_at: now - Duration::minutes(10),
        ends_at: now + Duration::minutes(60),
    });
    let app = test_router(state);

    let placed = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/orders")
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "buyer-demo")
                .header("Idempotency-Key", "p1")
                .body(Body::from(
                    json!({ "item_id": item_id.as_uuid().to_string(), "quantity": 1 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let order: Value =
        serde_json::from_slice(&placed.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let order_id = order["id"].as_str().unwrap();

    // 他人支付返回 403
    let forbidden = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/orders/{order_id}/pay"))
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "buyer-other")
                .header("Idempotency-Key", "pay-other")
                .body(Body::from(json!({ "version": 1 }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    // 本人正常支付 -> version=2, paid
    let paid = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/orders/{order_id}/pay"))
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "buyer-demo")
                .header("Idempotency-Key", "pay-1")
                .body(Body::from(json!({ "version": 1 }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(paid.status(), StatusCode::OK);
    let paid_json: Value =
        serde_json::from_slice(&paid.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(paid_json["status"], "paid");
    assert_eq!(paid_json["version"], 2);

    // 过期 version=1 取消 -> 409 order_version_conflict
    let stale_cancel = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/orders/{order_id}/cancel"))
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "buyer-demo")
                .header("Idempotency-Key", "cancel-stale")
                .body(Body::from(json!({ "version": 1 }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stale_cancel.status(), StatusCode::CONFLICT);

    // 正确 version=2 取消已支付订单 -> 退回库存并解除限购
    let cancel_ok = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/orders/{order_id}/cancel"))
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "buyer-demo")
                .header("Idempotency-Key", "cancel-ok")
                .body(Body::from(json!({ "version": 2 }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel_ok.status(), StatusCode::OK);
    assert_eq!(port.return_call_count(), 1);
}

#[sqlx::test(migrations = "../../../../migrations")]
#[ignore = "wip"]
async fn expire_unpaid_orders_releases_stock_after_hold_duration(pool: PgPool) {
    let now = base_time();
    let clock = Arc::new(FixedClock::new(now));
    let (state, port) = test_state(pool, clock.clone());
    let item_id = ItemId::new(Uuid::from_u128(0xBBBB_0003));
    port.seed_item(SeedItem {
        item_id,
        title: "限时过期测试品",
        kind: ItemKind::Digital,
        price_cents: 9900,
        total_stock: 2,
        per_user_limit: 1,
        starts_at: now - Duration::minutes(5),
        ends_at: now + Duration::minutes(60),
    });
    let app = test_router(state.clone());

    let placed = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/orders")
                .header("Content-Type", "application/json")
                .header("X-Buyer-Id", "buyer-demo")
                .header("Idempotency-Key", "exp-1")
                .body(Body::from(
                    json!({ "item_id": item_id.as_uuid().to_string(), "quantity": 1 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(placed.status().is_success());

    clock.advance(Duration::minutes(16));
    let outcome = cmd_expire_unpaid_order::execute(&state, ExpireUnpaidOrdersCommand::default())
        .await
        .unwrap();
    assert_eq!(outcome.expired_order_ids.len(), 1);
    assert_eq!(port.release_call_count(), 1);
}
