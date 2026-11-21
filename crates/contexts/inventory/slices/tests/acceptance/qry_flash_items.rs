use std::sync::Arc;

use axum::body::Body;
use chrono::Duration;
use flashmart_infra::FixedClock;
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::support::{TestItemSeed, base_time, insert_flash_item, test_router, test_state};

#[sqlx::test(migrations = "../../../../migrations")]
async fn list_flash_items_returns_window_status_and_server_time(pool: PgPool) {
    let now = base_time();
    let clock = Arc::new(FixedClock::new(now));

    let active_id = Uuid::from_u128(0x1111_1111_1111_4111_8111_1111_1111_1101);
    let upcoming_id = Uuid::from_u128(0x1111_1111_1111_4111_8111_1111_1111_1104);
    let ended_id = Uuid::from_u128(0x1111_1111_1111_4111_8111_1111_1111_1105);

    insert_flash_item(
        &pool,
        TestItemSeed {
            id: active_id,
            title: "创世数字徽章",
            item_kind: "digital",
            price_cents: 9900,
            total_stock: 10,
            per_user_limit: 1,
            starts_at: now - Duration::minutes(10),
            ends_at: now + Duration::minutes(60),
        },
    )
    .await;
    insert_flash_item(
        &pool,
        TestItemSeed {
            id: upcoming_id,
            title: "钛金属钥匙扣",
            item_kind: "physical",
            price_cents: 18900,
            total_stock: 8,
            per_user_limit: 1,
            starts_at: now + Duration::minutes(20),
            ends_at: now + Duration::minutes(90),
        },
    )
    .await;
    insert_flash_item(
        &pool,
        TestItemSeed {
            id: ended_id,
            title: "测试贴纸",
            item_kind: "physical",
            price_cents: 3900,
            total_stock: 5,
            per_user_limit: 1,
            starts_at: now - Duration::minutes(90),
            ends_at: now - Duration::minutes(10),
        },
    )
    .await;

    let (state, _) = test_state(pool, clock);
    let app = test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/flash-items")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["items"].as_array().unwrap().len(), 3);
    assert!(json["server_time"].is_string());
}

#[sqlx::test(migrations = "../../../../migrations")]
#[ignore = "wip"]
async fn get_flash_item_returns_404_problem_details_when_missing(pool: PgPool) {
    let clock = Arc::new(FixedClock::new(base_time()));
    let (state, _) = test_state(pool, clock);
    let app = test_router(state);

    let missing_id = Uuid::new_v4();
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/flash-items/{missing_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], "item_not_found");
}
