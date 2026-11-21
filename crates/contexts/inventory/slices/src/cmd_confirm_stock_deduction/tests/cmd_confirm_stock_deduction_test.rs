use std::sync::Arc;

use chrono::{Duration, TimeZone, Utc};
use flashmart_contracts::{
    ConfirmStockDeductionRequest, IntegrationEvent, InventoryPort, InventoryPortError,
    ReserveStockRequest,
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

use super::super::contract::ConfirmStockDeductionCommand;
use super::super::workflow::{ConfirmStockDeductionDecision, decide_confirm_stock_deduction};
use crate::port::PgInventoryPort;

fn base_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap()
}

fn sample_item(available: i32, reserved: i32, confirmed: i32) -> FlashItem {
    let now = base_time();
    let sale_window =
        SaleWindow::try_new(now - Duration::minutes(10), now + Duration::minutes(30)).unwrap();
    let total = available + reserved + confirmed;
    let ledger = InventoryLedger::rehydrate(total, available, reserved, confirmed, 0).unwrap();
    FlashItem::try_new(
        ItemId::new(Uuid::from_u128(0x3001)),
        "创世徽章".to_owned(),
        "测试描述".to_owned(),
        ItemKind::Digital,
        MoneyCents::try_new(9900).unwrap(),
        Currency::Cny,
        2,
        sale_window,
        ledger,
    )
    .unwrap()
}

fn sample_reservation(item_id: ItemId, qty: i32, status: ReservationStatus) -> StockReservation {
    StockReservation::rehydrate(
        ReservationId::new(Uuid::from_u128(0x4001)),
        item_id,
        BuyerId::try_new("buyer-1").unwrap(),
        Quantity::try_new(qty).unwrap(),
        status,
        IdempotencyKey::try_new("idem-confirm").unwrap(),
    )
}

#[test]
fn decide_confirms_reserved_stock_and_emits_event() {
    let now = base_time();
    let mut item = sample_item(3, 2, 0);
    let mut reservation = sample_reservation(item.id(), 2, ReservationStatus::Reserved);
    let cmd = ConfirmStockDeductionCommand {
        reservation_id: reservation.id(),
    };

    let decision = decide_confirm_stock_deduction(&cmd, &mut item, &mut reservation, now).unwrap();
    match decision {
        ConfirmStockDeductionDecision::Confirmed { event, outcome } => {
            assert_eq!(reservation.status(), ReservationStatus::Confirmed);
            assert_eq!(item.ledger().reserved_stock(), 0);
            assert_eq!(item.ledger().confirmed_stock(), 2);
            assert!(!outcome.idempotent_replay);
            assert_eq!(outcome.reservation_id, reservation.id());
            assert_eq!(
                event,
                IntegrationEvent::StockDeductionConfirmed {
                    reservation_id: reservation.id(),
                    item_id: item.id(),
                    quantity: reservation.quantity(),
                    occurred_at: now,
                }
            );
        }
        other => panic!("expected Confirmed, got {other:?}"),
    }
}

#[test]
fn decide_replays_idempotently_when_already_confirmed() {
    let now = base_time();
    let mut item = sample_item(3, 0, 2);
    let mut reservation = sample_reservation(item.id(), 2, ReservationStatus::Confirmed);
    let cmd = ConfirmStockDeductionCommand {
        reservation_id: reservation.id(),
    };

    let decision = decide_confirm_stock_deduction(&cmd, &mut item, &mut reservation, now).unwrap();
    match decision {
        ConfirmStockDeductionDecision::IdempotentReplay { outcome } => {
            assert!(outcome.idempotent_replay);
            assert_eq!(outcome.reservation_id, reservation.id());
            assert_eq!(item.ledger().reserved_stock(), 0);
            assert_eq!(item.ledger().confirmed_stock(), 2);
        }
        other => panic!("expected IdempotentReplay, got {other:?}"),
    }
}

#[rstest]
#[case(ReservationStatus::Released, "released")]
#[case(ReservationStatus::Returned, "returned")]
fn decide_rejects_illegal_reservation_states(
    #[case] status: ReservationStatus,
    #[case] expected_current: &str,
) {
    let now = base_time();
    let mut item = sample_item(3, 2, 0);
    let mut reservation = sample_reservation(item.id(), 2, status);
    let cmd = ConfirmStockDeductionCommand {
        reservation_id: reservation.id(),
    };

    let err = decide_confirm_stock_deduction(&cmd, &mut item, &mut reservation, now).unwrap_err();
    assert_eq!(
        err,
        InventoryPortError::InvalidReservationState {
            current: expected_current.to_owned(),
        }
    );
    assert_eq!(item.ledger().reserved_stock(), 2);
    assert_eq!(item.ledger().confirmed_stock(), 0);
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn pg_confirm_stock_deduction_handles_lifecycle_idempotency_and_errors(pool: PgPool) {
    let now = base_time();
    let clock = Arc::new(FixedClock::new(now));
    let id_gen = Arc::new(SequenceIdGenerator::new(1));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let port = PgInventoryPort::new(pool.clone(), clock.clone(), id_gen, event_bus.clone());

    // 预占不存在时返回 ReservationNotFound
    let missing_id = ReservationId::new(Uuid::new_v4());
    let missing = port
        .confirm_stock_deduction(ConfirmStockDeductionRequest {
            reservation_id: missing_id,
        })
        .await
        .unwrap_err();
    assert_eq!(missing, InventoryPortError::ReservationNotFound(missing_id));

    let item_id = Uuid::from_u128(0xCCCC_0001);
    sqlx::query(
        r#"
        INSERT INTO inventory.flash_items (
            id, title, description, item_kind, price_cents, currency,
            total_stock, available_stock, reserved_stock, confirmed_stock,
            returned_after_close_stock, per_user_limit, sale_starts_at, sale_ends_at
        )
        VALUES ($1, '确认扣减测试品', '描述', 'digital', 5000, 'CNY',
                5, 5, 0, 0, 0, 3, $2, $3)
        "#,
    )
    .bind(item_id)
    .bind(now - Duration::minutes(5))
    .bind(now + Duration::minutes(15))
    .execute(&pool)
    .await
    .unwrap();

    let res_id = ReservationId::new(Uuid::new_v4());
    port.reserve_stock(ReserveStockRequest {
        reservation_id: res_id,
        item_id: ItemId::new(item_id),
        buyer_id: BuyerId::try_new("buyer-confirm").unwrap(),
        quantity: Quantity::try_new(2).unwrap(),
        idempotency_key: IdempotencyKey::try_new("idem-confirm-1").unwrap(),
    })
    .await
    .unwrap();

    // 即使抢购时间窗刚刚结束，已预占的订单仍允许确认扣减
    clock.advance(Duration::minutes(20));

    let first = port
        .confirm_stock_deduction(ConfirmStockDeductionRequest {
            reservation_id: res_id,
        })
        .await
        .unwrap();
    assert_eq!(first.reservation_id, res_id);
    assert!(!first.idempotent_replay);

    let (reserved_stock, confirmed_stock): (i32, i32) = sqlx::query_as(
        "SELECT reserved_stock, confirmed_stock FROM inventory.flash_items WHERE id = $1",
    )
    .bind(item_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((reserved_stock, confirmed_stock), (0, 2));

    let status: String =
        sqlx::query_scalar("SELECT status FROM inventory.stock_reservations WHERE id = $1")
            .bind(res_id.into_inner())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "confirmed");

    // 幂等重放不再新增 outbox 事件或发布事件
    let replay = port
        .confirm_stock_deduction(ConfirmStockDeductionRequest {
            reservation_id: res_id,
        })
        .await
        .unwrap();
    assert_eq!(replay.reservation_id, res_id);
    assert!(replay.idempotent_replay);

    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory.outbox_events WHERE aggregate_id = $1 AND event_type = 'stock_deduction_confirmed'",
    )
    .bind(item_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(outbox_count, 1);
    assert_eq!(event_bus.snapshot().len(), 2);

    // 对已释放的预占调用确认扣减返回 InvalidReservationState
    clock.advance(Duration::minutes(-15));
    let released_res_id = ReservationId::new(Uuid::new_v4());
    port.reserve_stock(ReserveStockRequest {
        reservation_id: released_res_id,
        item_id: ItemId::new(item_id),
        buyer_id: BuyerId::try_new("buyer-released").unwrap(),
        quantity: Quantity::try_new(1).unwrap(),
        idempotency_key: IdempotencyKey::try_new("idem-confirm-2").unwrap(),
    })
    .await
    .unwrap();
    port.release_reserved_stock(flashmart_contracts::ReleaseReservedStockRequest {
        reservation_id: released_res_id,
    })
    .await
    .unwrap();

    let illegal = port
        .confirm_stock_deduction(ConfirmStockDeductionRequest {
            reservation_id: released_res_id,
        })
        .await
        .unwrap_err();
    assert_eq!(
        illegal,
        InventoryPortError::InvalidReservationState {
            current: "released".to_owned(),
        }
    );
}
