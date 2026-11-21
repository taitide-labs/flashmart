use chrono::{Duration, TimeZone, Utc};
use flashmart_contracts::{IntegrationEvent, OrderLifecycleStatus};
use flashmart_primitives::{
    BuyerId, Currency, ItemId, ItemKind, MoneyCents, OrderId, OrderVersion, Quantity, ReservationId,
};
use rstest::rstest;
use uuid::Uuid;

use crate::{ActiveQuota, FlashOrder, OrderState, OrderingDomainError, PaymentHoldDuration};

fn sample_order(now: chrono::DateTime<Utc>, kind: ItemKind) -> FlashOrder {
    let (order, _) = FlashOrder::place(
        OrderId::new(Uuid::from_u128(0x1111)),
        ReservationId::new(Uuid::from_u128(0x2222)),
        BuyerId::try_new("buyer-demo").unwrap(),
        ItemId::new(Uuid::from_u128(0x3333)),
        "创世数字徽章".to_owned(),
        kind,
        Quantity::try_new(2).unwrap(),
        MoneyCents::try_new(9900).unwrap(),
        Currency::Cny,
        PaymentHoldDuration::default(),
        now,
    )
    .unwrap();
    order
}

#[rstest]
#[case(1, 0, 1, true)]
#[case(1, 1, 1, false)]
#[case(3, 2, 1, true)]
#[case(3, 2, 2, false)]
fn active_quota_enforcement(
    #[case] limit: i32,
    #[case] active: i32,
    #[case] requested: i32,
    #[case] allowed: bool,
) {
    let quota = ActiveQuota::try_new(limit, active).unwrap();
    let res = quota.ensure_can_place(Quantity::try_new(requested).unwrap());
    assert_eq!(res.is_ok(), allowed);
}

#[test]
fn place_and_pay_within_hold_generates_fulfillment_hint_and_bumps_version() {
    let t0 = Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap();
    let buyer = BuyerId::try_new("buyer-demo").unwrap();
    let mut order = sample_order(t0, ItemKind::Digital);
    assert_eq!(order.total_amount_cents().get(), 19800);
    assert_eq!(order.version().get(), 1);
    assert_eq!(
        order.state().lifecycle_status(),
        OrderLifecycleStatus::PendingPayment
    );

    let event = order
        .pay(&buyer, OrderVersion::INITIAL, t0 + Duration::minutes(15))
        .unwrap();
    assert!(matches!(event, IntegrationEvent::FlashOrderPaid { .. }));
    assert_eq!(order.version().get(), 2);
    assert!(matches!(order.state(), OrderState::Paid { .. }));
    assert!(
        order
            .fulfillment_hint()
            .starts_with("模拟数字兑换码：FM-DIGI-")
    );
}

#[test]
fn pay_rejects_wrong_owner_stale_version_and_overdue_deadline() {
    let t0 = Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap();
    let buyer = BuyerId::try_new("buyer-demo").unwrap();
    let other = BuyerId::try_new("buyer-other").unwrap();
    let mut order = sample_order(t0, ItemKind::Physical);

    assert_eq!(
        order.pay(&other, OrderVersion::INITIAL, t0 + Duration::minutes(1)),
        Err(OrderingDomainError::OwnershipMismatch)
    );
    assert_eq!(
        order.pay(
            &buyer,
            OrderVersion::try_new(2).unwrap(),
            t0 + Duration::minutes(1)
        ),
        Err(OrderingDomainError::VersionConflict {
            expected: 2,
            actual: 1
        })
    );
    assert_eq!(
        order.pay(&buyer, OrderVersion::INITIAL, t0 + Duration::minutes(16)),
        Err(OrderingDomainError::OrderExpired)
    );
}

#[test]
fn cancel_supports_both_pending_and_paid_orders_and_blocks_double_cancel() {
    let t0 = Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap();
    let buyer = BuyerId::try_new("buyer-demo").unwrap();

    let mut pending = sample_order(t0, ItemKind::Digital);
    let (was_paid, _) = pending
        .cancel(&buyer, OrderVersion::INITIAL, t0 + Duration::minutes(2))
        .unwrap();
    assert!(!was_paid);
    assert_eq!(pending.version().get(), 2);

    let mut paid = sample_order(t0, ItemKind::Physical);
    paid.pay(&buyer, OrderVersion::INITIAL, t0 + Duration::minutes(1))
        .unwrap();
    let (was_paid, _) = paid
        .cancel(
            &buyer,
            OrderVersion::try_new(2).unwrap(),
            t0 + Duration::minutes(3),
        )
        .unwrap();
    assert!(was_paid);
    assert_eq!(paid.version().get(), 3);
    assert!(matches!(
        paid.cancel(
            &buyer,
            OrderVersion::try_new(3).unwrap(),
            t0 + Duration::minutes(4)
        ),
        Err(OrderingDomainError::InvalidOrderState { .. })
    ));
}

#[test]
fn expire_requires_deadline_elapsed() {
    let t0 = Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap();
    let mut order = sample_order(t0, ItemKind::Digital);

    assert_eq!(
        order.expire(t0 + Duration::minutes(15)),
        Err(OrderingDomainError::NotDueForExpiration)
    );

    let ev = order
        .expire(t0 + Duration::minutes(15) + Duration::seconds(1))
        .unwrap();
    assert!(matches!(ev, IntegrationEvent::FlashOrderExpired { .. }));
    assert_eq!(
        order.state().lifecycle_status(),
        OrderLifecycleStatus::Expired
    );
}
