use std::sync::Arc;

use axum::body::Body;
use flashmart_infra::FixedClock;
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::support::{base_time, test_router, test_state};

#[sqlx::test(migrations = "../../../../migrations")]
async fn list_orders_requires_buyer_header_and_isolates_buyers(pool: PgPool) {
    let clock = Arc::new(FixedClock::new(base_time()));
    let (state, _) = test_state(pool, clock);
    let app = test_router(state);

    let missing_header = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/orders")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_header.status(), StatusCode::BAD_REQUEST);

    let ok_res = app
        .oneshot(
            Request::builder()
                .uri("/api/orders")
                .header("X-Buyer-Id", "buyer-demo")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok_res.status(), StatusCode::OK);
    let body = ok_res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["orders"].as_array().unwrap().len(), 0);
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn get_order_returns_404_when_missing(pool: PgPool) {
    let clock = Arc::new(FixedClock::new(base_time()));
    let (state, _) = test_state(pool, clock);
    let app = test_router(state);

    let order_id = Uuid::new_v4();
    let res = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/orders/{order_id}"))
                .header("X-Buyer-Id", "buyer-demo")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], "order_not_found");
}
