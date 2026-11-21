use std::collections::BTreeSet;
use std::sync::Arc;

use axum::body::Body;
use chrono::{Duration, TimeZone, Utc};
use flashmart_contracts::InventoryPort;
use flashmart_infra::{
    AppState, FixedClock, InMemoryEventBus, SequenceIdGenerator, SliceDescriptor, SliceStatus,
};
use flashmart_inventory_slices::port::PgInventoryPort;
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;
use utoipa::OpenApi;

use crate::{ApiDoc, build_app};

fn dummy_state() -> AppState {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://postgres:postgres@localhost:5432/postgres")
        .expect("valid postgres dsn");
    let clock = Arc::new(FixedClock::new(
        Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap(),
    ));
    let id_gen = Arc::new(SequenceIdGenerator::new(1));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let port = Arc::new(PgInventoryPort::new(
        pool.clone(),
        clock.clone(),
        id_gen.clone(),
        event_bus.clone(),
    ));
    AppState::new(
        pool,
        clock,
        id_gen,
        port as Arc<dyn InventoryPort>,
        event_bus,
        Duration::minutes(15),
    )
}

#[tokio::test]
async fn health_endpoint_is_active_and_planned_slices_are_hidden() {
    let app = build_app(dummy_state());

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
    let body = health_res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");

    let registered: BTreeSet<&str> = inventory::iter::<SliceDescriptor>()
        .map(|d| d.id)
        .collect();
    for expected in [
        "qry_list_flash_items",
        "qry_get_flash_item",
        "qry_list_orders",
        "qry_get_order",
        "cmd_place_order",
        "cmd_pay_order",
        "cmd_cancel_order",
    ] {
        assert!(registered.contains(expected), "missing slice {expected}");
    }

    // 任意仍处于 Planned 状态的静态路由在生产组合根上应返回 404（禁止暴露未完工入口）
    for desc in inventory::iter::<SliceDescriptor>() {
        if desc.status == SliceStatus::Planned && !desc.path.contains('{') {
            let method = match desc.method {
                flashmart_infra::SliceMethod::Get => "GET",
                flashmart_infra::SliceMethod::Post => "POST",
            };
            let res = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(desc.path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(
                res.status() == StatusCode::NOT_FOUND
                    || res.status() == StatusCode::METHOD_NOT_ALLOWED,
                "planned slice {} should not be exposed, got {}",
                desc.id,
                res.status()
            );
        }
    }
}

#[test]
fn openapi_spec_contains_all_frozen_paths() {
    let spec = serde_json::to_value(ApiDoc::openapi()).unwrap();
    let paths = spec["paths"].as_object().unwrap();
    for path in [
        "/api/health",
        "/api/flash-items",
        "/api/flash-items/{item_id}",
        "/api/orders",
        "/api/orders/{order_id}",
        "/api/orders/{order_id}/pay",
        "/api/orders/{order_id}/cancel",
    ] {
        assert!(paths.contains_key(path), "missing openapi path {path}");
    }
}
