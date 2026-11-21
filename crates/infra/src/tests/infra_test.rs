use axum::response::IntoResponse;
use chrono::{Duration, TimeZone, Utc};
use flashmart_contracts::{Clock, IdGenerator, IntegrationEvent};
use flashmart_primitives::{BuyerId, ItemId, OrderId, Quantity, ReservationId};
use http::{HeaderMap, HeaderValue, StatusCode, header};
use uuid::Uuid;

use crate::{
    EventBus, FixedClock, InMemoryEventBus, ProblemDetails, SequenceIdGenerator, extract_buyer_id,
    extract_idempotency_key,
};

#[test]
fn fixed_clock_and_sequence_id_are_deterministic() {
    let base = Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap();
    let clock = FixedClock::new(base);
    assert_eq!(clock.now(), base);
    clock.advance(Duration::minutes(15));
    assert_eq!(clock.now(), base + Duration::minutes(15));

    let id_gen = SequenceIdGenerator::new(10);
    let first = id_gen.next_uuid();
    let second = id_gen.next_uuid();
    assert_ne!(first, second);
}

#[test]
fn header_extractors_enforce_buyer_and_idempotency_rules() {
    let empty = HeaderMap::new();
    let err = extract_buyer_id(&empty, "/api/orders").unwrap_err();
    assert_eq!(err.status, 400);
    assert_eq!(err.code, "missing_required_header");

    let mut bad_buyer = HeaderMap::new();
    bad_buyer.insert("X-Buyer-Id", HeaderValue::from_static("invalid buyer!"));
    let err = extract_buyer_id(&bad_buyer, "/api/orders").unwrap_err();
    assert_eq!(err.status, 422);
    assert_eq!(err.code, "validation_error");

    let mut valid = HeaderMap::new();
    valid.insert("X-Buyer-Id", HeaderValue::from_static("buyer-demo"));
    valid.insert("Idempotency-Key", HeaderValue::from_static("req-001"));
    assert_eq!(
        extract_buyer_id(&valid, "/api/orders").unwrap().as_str(),
        "buyer-demo"
    );
    assert_eq!(
        extract_idempotency_key(&valid, "/api/orders")
            .unwrap()
            .as_str(),
        "req-001"
    );
}

#[test]
fn problem_details_sets_rfc9457_content_type() {
    let problem = ProblemDetails::new(
        StatusCode::CONFLICT,
        "insufficient_stock",
        "剩余库存不足",
        "库存不足",
        "/api/orders",
    );
    let response = problem.into_response();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/problem+json"
    );
}

#[tokio::test]
async fn in_memory_event_bus_records_published_events() {
    let bus = InMemoryEventBus::new();
    let event = IntegrationEvent::FlashOrderPaid {
        order_id: OrderId::new(Uuid::nil()),
        reservation_id: ReservationId::new(Uuid::nil()),
        buyer_id: BuyerId::try_new("buyer-demo").unwrap(),
        item_id: ItemId::new(Uuid::nil()),
        quantity: Quantity::try_new(1).unwrap(),
        paid_at: Utc::now(),
    };
    bus.publish(event.clone()).await;
    assert_eq!(bus.snapshot(), vec![event]);
}
