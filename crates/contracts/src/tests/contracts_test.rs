use chrono::TimeZone;
use flashmart_primitives::{BuyerId, ItemId, MoneyCents, OrderId, Quantity, ReservationId};
use uuid::Uuid;

use crate::{IntegrationEvent, OrderLifecycleStatus, WindowStatus};

#[test]
fn open_enums_serialize_as_snake_case() {
    assert_eq!(serde_json::to_string(&WindowStatus::Active).unwrap(), "\"active\"");
    assert_eq!(
        serde_json::to_string(&OrderLifecycleStatus::PendingPayment).unwrap(),
        "\"pending_payment\""
    );
}

#[test]
fn integration_event_carries_tagged_event_type() {
    let event = IntegrationEvent::FlashOrderPlaced {
        order_id: OrderId::new(Uuid::nil()),
        reservation_id: ReservationId::new(Uuid::nil()),
        buyer_id: BuyerId::try_new("buyer-demo").unwrap(),
        item_id: ItemId::new(Uuid::nil()),
        quantity: Quantity::try_new(1).unwrap(),
        total_amount_cents: MoneyCents::try_new(9900).unwrap(),
        occurred_at: chrono::Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap(),
    };
    assert_eq!(event.event_type(), "flash_order_placed");
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["event_type"], "flash_order_placed");
}
