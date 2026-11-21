use std::sync::Arc;

use chrono::{Duration, TimeZone, Utc};
use flashmart_contracts::{
    ConfirmStockDeductionRequest, IntegrationEvent, InventoryPort, InventoryPortError,
    ReserveStockRequest, ReturnDeductedStockRequest,
};
use flashmart_infra::{FixedClock, InMemoryEventBus, SequenceIdGenerator};
use flashmart_inventory_domain::{
    FlashItem, InventoryLedger, ReservationStatus, SaleWindow, StockReservation,
};
use flashmart_primitives::{
    BuyerId, Currency, IdempotencyKey, ItemId, ItemKind, MoneyCents, Quantity, ReservationId,
};
use rstest::rstest;
use sqlx::PgPool;
use uuid::Uuid;

use super::super::contract::ReturnDeductedStockCommand;
use super::super::workflow::{ReturnDeductedStockDecision, decide_return_deducted_stock};
use crate::port::PgInventoryPort;

fn base_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap()
}

fn sample_item(available: i32, confirmed: i32, returned_after_close: i32) -> FlashItem {
    let now = base_time();
    let sale_window =
        SaleWindow::try_new(now - Duration::minutes(10), now + Duration::minutes(30)).unwrap();
    let total = available + confirmed + returned_after_close;
    let ledger =
        InventoryLedger::rehydrate(total, available, 0, confirmed, returned_after_close).unwrap();
    FlashItem::try_new(
        ItemId::new(Uuid::from_u128(0x7001)),
        "限量潮玩".to_owned(),
        "测试描述".to_owned(),
        ItemKind::Physical,
        MoneyCents::try_new(19900).unwrap(),
        Currency::Cny,
        2,
        sale_window,
        ledger,
    )
    .unwrap()
}

fn sample_reservation(item_id: ItemId, qty: i32, status: ReservationStatus) -> StockReservation {
    StockReservation::rehydrate(
        ReservationId::new(Uuid::from_u128(0x8001)),
        item_id,
        BuyerId::try_new("buyer-1").unwrap(),
        Quantity::try_new(qty).unwrap(),
        status,
        IdempotencyKey::try_new("idem-return").unwrap(),
    )
}

#[rstest]
#[case(0, true, 5, 0)]
#[case(31, false, 3, 2)]
fn decide_returns_deducted_stock_following_sale_window(
    #[case] offset_mins: i64,
    #[case] expect_restocked: bool,
    #[case] expected_available: i32,
    #[case] expected_returned_after_close: i32,
) {
    let now = base_time() + Duration::minutes(offset_mins);
    let mut item = sample_item(3, 2, 0);
    let mut reservation = sample_reservation(item.id(), 2, ReservationStatus::Confirmed);
    let cmd = ReturnDeductedStockCommand {
        reservation_id: reservation.id(),
    };

    let decision = decide_return_deducted_stock(&cmd, &mut item, &mut reservation, now).unwrap();
    match decision {
        ReturnDeductedStockDecision::Returned {
            restocked_to_available,
            event,
            outcome,
        } => {
            assert_eq!(restocked_to_available, expect_restocked);
            assert_eq!(reservation.status(), ReservationStatus::Returned);
            assert_eq!(item.ledger().available_stock(), expected_available);
            assert_eq!(item.ledger().confirmed_stock(), 0);
            assert_eq!(
                item.ledger().returned_after_close_stock(),
                expected_returned_after_close
            );
            assert!(!outcome.idempotent_replay);
            assert_eq!(outcome.restocked_to_available, expect_restocked);
            assert_eq!(
                event,
                IntegrationEvent::DeductedStockReturned {
                    reservation_id: reservation.id(),
                    item_id: item.id(),
                    quantity: reservation.quantity(),
                    restocked_to_available: expect_restocked,
                    occurred_at: now,
                }
            );
        }
        other => panic!("expected Returned, got {other:?}"),
    }
}

#[test]
fn decide_replays_idempotently_when_already_returned() {
    let now = base_time();
    let mut item = sample_item(5, 0, 0);
    let mut reservation = sample_reservation(item.id(), 2, ReservationStatus::Returned);
    let cmd = ReturnDeductedStockCommand {
        reservation_id: reservation.id(),
    };

    let decision = decide_return_deducted_stock(&cmd, &mut item, &mut reservation, now).unwrap();
    match decision {
        ReturnDeductedStockDecision::IdempotentReplay { outcome } => {
            assert!(outcome.idempotent_replay);
            assert!(!outcome.restocked_to_available);
            assert_eq!(item.ledger().available_stock(), 5);
        }
        other => panic!("expected IdempotentReplay, got {other:?}"),
    }
}

#[rstest]
#[case(ReservationStatus::Reserved, "reserved")]
#[case(ReservationStatus::Released, "released")]
fn decide_rejects_illegal_reservation_states(
    #[case] status: ReservationStatus,
    #[case] expected_current: &str,
) {
    let now = base_time();
    let mut item = sample_item(3, 2, 0);
    let mut reservation = sample_reservation(item.id(), 2, status);
    let cmd = ReturnDeductedStockCommand {
        reservation_id: reservation.id(),
    };

    let err = decide_return_deducted_stock(&cmd, &mut item, &mut reservation, now).unwrap_err();
    assert_eq!(
        err,
        InventoryPortError::InvalidReservationState {
            current: expected_current.to_owned(),
        }
    );
    assert_eq!(item.ledger().available_stock(), 3);
    assert_eq!(item.ledger().confirmed_stock(), 2);
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn pg_return_deducted_stock_handles_window_restock_idempotency_and_errors(pool: PgPool) {
    let now = base_time();
    let clock = Arc::new(FixedClock::new(now));
    let id_gen = Arc::new(SequenceIdGenerator::new(1));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let port = PgInventoryPort::new(pool.clone(), clock.clone(), id_gen, event_bus.clone());

    let missing_id = ReservationId::new(Uuid::new_v4());
    assert_eq!(
        port.return_deducted_stock(ReturnDeductedStockRequest {
            reservation_id: missing_id,
        })
        .await
        .unwrap_err(),
        InventoryPortError::ReservationNotFound(missing_id)
    );

    let item_id = Uuid::from_u128(0xEEEE_0001);
    sqlx::query(
        r#"
        INSERT INTO inventory.flash_items (
            id, title, description, item_kind, price_cents, currency,
            total_stock, available_stock, reserved_stock, confirmed_stock,
            returned_after_close_stock, per_user_limit, sale_starts_at, sale_ends_at
        )
        VALUES ($1, '退回回流测试品', '描述', 'physical', 9900, 'CNY',
                4, 4, 0, 0, 0, 2, $2, $3)
        "#,
    )
    .bind(item_id)
    .bind(now - Duration::minutes(5))
    .bind(now + Duration::minutes(10))
    .execute(&pool)
    .await
    .unwrap();

    let res_open = ReservationId::new(Uuid::new_v4());
    let res_closed = ReservationId::new(Uuid::new_v4());
    let res_reserved = ReservationId::new(Uuid::new_v4());

    for (res_id, key) in [
        (res_open, "idem-ret-1"),
        (res_closed, "idem-ret-2"),
        (res_reserved, "idem-ret-3"),
    ] {
        port.reserve_stock(ReserveStockRequest {
            reservation_id: res_id,
            item_id: ItemId::new(item_id),
            buyer_id: BuyerId::try_new("buyer-ret").unwrap(),
            quantity: Quantity::try_new(1).unwrap(),
            idempotency_key: IdempotencyKey::try_new(key).unwrap(),
        })
        .await
        .unwrap();
    }

    // 未确认扣减的预占不允许退回
    let illegal = port
        .return_deducted_stock(ReturnDeductedStockRequest {
            reservation_id: res_reserved,
        })
        .await
        .unwrap_err();
    assert_eq!(
        illegal,
        InventoryPortError::InvalidReservationState {
            current: "reserved".to_owned(),
        }
    );

    port.confirm_stock_deduction(ConfirmStockDeductionRequest {
        reservation_id: res_open,
    })
    .await
    .unwrap();
    port.confirm_stock_deduction(ConfirmStockDeductionRequest {
        reservation_id: res_closed,
    })
    .await
    .unwrap();

    // 窗口期内退回 -> 回流为可售库存，且支持幂等重放
    let open_outcome = port
        .return_deducted_stock(ReturnDeductedStockRequest {
            reservation_id: res_open,
        })
        .await
        .unwrap();
    assert!(!open_outcome.idempotent_replay);
    assert!(open_outcome.restocked_to_available);

    let replay = port
        .return_deducted_stock(ReturnDeductedStockRequest {
            reservation_id: res_open,
        })
        .await
        .unwrap();
    assert!(replay.idempotent_replay);
    assert!(!replay.restocked_to_available);

    // 闭窗后退回 -> 记账归还，不再增加 available_stock
    clock.advance(Duration::minutes(20));
    let closed_outcome = port
        .return_deducted_stock(ReturnDeductedStockRequest {
            reservation_id: res_closed,
        })
        .await
        .unwrap();
    assert!(!closed_outcome.idempotent_replay);
    assert!(!closed_outcome.restocked_to_available);

    let (avail, reserved, confirmed, returned_after_close): (i32, i32, i32, i32) = sqlx::query_as(
        "SELECT available_stock, reserved_stock, confirmed_stock, returned_after_close_stock FROM inventory.flash_items WHERE id = $1",
    )
    .bind(item_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (avail, reserved, confirmed, returned_after_close),
        (2, 1, 0, 1)
    );

    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory.outbox_events WHERE aggregate_id = $1 AND event_type = 'deducted_stock_returned'",
    )
    .bind(item_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(outbox_count, 2);
}
