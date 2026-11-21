use chrono::{Duration, TimeZone, Utc};
use flashmart_contracts::WindowStatus;
use flashmart_primitives::{BuyerId, IdempotencyKey, ItemId, Quantity, ReservationId};
use rstest::rstest;
use uuid::Uuid;

use crate::{
    InventoryDomainError, InventoryLedger, ReservationStatus, SaleWindow, StockReservation,
};

#[rstest]
#[case(-10, WindowStatus::Upcoming)]
#[case(0, WindowStatus::Active)]
#[case(30, WindowStatus::Active)]
#[case(60, WindowStatus::Active)]
#[case(61, WindowStatus::Ended)]
fn sale_window_boundary_classification(#[case] offset_mins: i64, #[case] expected: WindowStatus) {
    let start = Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap();
    let end = start + Duration::minutes(60);
    let window = SaleWindow::try_new(start, end).unwrap();
    assert_eq!(
        window.status_at(start + Duration::minutes(offset_mins)),
        expected
    );
}

#[test]
fn ledger_preserves_conservation_across_lifecycle_and_restock_window() {
    let mut ledger = InventoryLedger::try_new(10).unwrap();
    let qty3 = Quantity::try_new(3).unwrap();
    let qty2 = Quantity::try_new(2).unwrap();

    // 未开始与已结束均不允许预占
    assert_eq!(
        ledger.reserve(qty3, WindowStatus::Upcoming),
        Err(InventoryDomainError::SaleWindowNotActive {
            status: WindowStatus::Upcoming
        })
    );

    // 进行中预占 3 件，再确认扣减 2 件、剩余 1 件保持预占
    ledger.reserve(qty3, WindowStatus::Active).unwrap();
    assert_eq!(ledger.available_stock(), 7);
    assert_eq!(ledger.reserved_stock(), 3);

    ledger.confirm_deduction(qty2).unwrap();
    assert_eq!(ledger.reserved_stock(), 1);
    assert_eq!(ledger.confirmed_stock(), 2);

    // 时间窗进行中释放 1 件预占 -> 回流至可售库存
    let restocked = ledger
        .release_reserved(Quantity::try_new(1).unwrap(), WindowStatus::Active)
        .unwrap();
    assert!(restocked);
    assert_eq!(ledger.available_stock(), 8);
    assert_eq!(ledger.reserved_stock(), 0);

    // 时间窗结束后退回已扣减 2 件 -> 记账归还不再开售
    let restocked_after_close = ledger.return_deducted(qty2, WindowStatus::Ended).unwrap();
    assert!(!restocked_after_close);
    assert_eq!(ledger.available_stock(), 8);
    assert_eq!(ledger.confirmed_stock(), 0);
    assert_eq!(ledger.returned_after_close_stock(), 2);
}

#[test]
fn ledger_rejects_oversell_and_negative_state() {
    let mut ledger = InventoryLedger::try_new(2).unwrap();
    let res = ledger.reserve(Quantity::try_new(3).unwrap(), WindowStatus::Active);
    assert_eq!(res, Err(InventoryDomainError::InsufficientStock));
    assert_eq!(ledger.available_stock(), 2);

    assert_eq!(
        InventoryLedger::rehydrate(5, 3, 1, 0, 0),
        Err(InventoryDomainError::LedgerInvariantViolated)
    );
}

#[test]
fn stock_reservation_state_machine_is_idempotent_and_rejects_illegal_transitions() {
    let mut r = StockReservation::new_reserved(
        ReservationId::new(Uuid::nil()),
        ItemId::new(Uuid::nil()),
        BuyerId::try_new("buyer-demo").unwrap(),
        Quantity::try_new(1).unwrap(),
        IdempotencyKey::try_new("idem-1").unwrap(),
    );

    assert!(!r.confirm().unwrap());
    assert!(r.confirm().unwrap());
    assert_eq!(r.status(), ReservationStatus::Confirmed);
    assert!(matches!(
        r.release(),
        Err(InventoryDomainError::InvalidReservationTransition { .. })
    ));

    assert!(!r.return_deducted().unwrap());
    assert!(r.return_deducted().unwrap());
    assert_eq!(r.status(), ReservationStatus::Returned);
}
