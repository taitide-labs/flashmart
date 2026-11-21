use std::sync::Arc;

use chrono::{Duration, TimeZone, Utc};
use flashmart_contracts::{
    ConfirmStockDeductionRequest, IntegrationEvent, InventoryPort, InventoryPortError,
    ReleaseReservedStockRequest, ReserveStockRequest,
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

use super::super::contract::ReleaseReservedStockCommand;
use super::super::workflow::{ReleaseReservedStockDecision, decide_release_reserved_stock};
use crate::port::PgInventoryPort;

fn base_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap()
}

fn sample_item(available: i32, reserved: i32, returned_after_close: i32) -> FlashItem {
    let now = base_time();
    let sale_window =
        SaleWindow::try_new(now - Duration::minutes(10), now + Duration::minutes(30)).unwrap();
    let total = available + reserved + returned_after_close;
    let ledger =
        InventoryLedger::rehydrate(total, available, reserved, 0, returned_after_close).unwrap();
    FlashItem::try_new(
        ItemId::new(Uuid::from_u128(0x5001)),
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
        ReservationId::new(Uuid::from_u128(0x6001)),
        item_id,
        BuyerId::try_new("buyer-1").unwrap(),
        Quantity::try_new(qty).unwrap(),
        status,
        IdempotencyKey::try_new("idem-release").unwrap(),
    )
}

#[rstest]
#[case(0, true, 5, 0)]
#[case(31, false, 3, 2)]
fn decide_releases_reserved_stock_following_sale_window(
    #[case] offset_mins: i64,
    #[case] expect_restocked: bool,
    #[case] expected_available: i32,
    #[case] expected_returned_after_close: i32,
) {
    let now = base_time() + Duration::minutes(offset_mins);
    let mut item = sample_item(3, 2, 0);
    let mut reservation = sample_reservation(item.id(), 2, ReservationStatus::Reserved);
    let cmd = ReleaseReservedStockCommand {
        reservation_id: reservation.id(),
    };

    let decision = decide_release_reserved_stock(&cmd, &mut item, &mut reservation, now).unwrap();
    match decision {
        ReleaseReservedStockDecision::Released {
            restocked_to_available,
            event,
            outcome,
        } => {
            assert_eq!(restocked_to_available, expect_restocked);
            assert_eq!(reservation.status(), ReservationStatus::Released);
            assert_eq!(item.ledger().available_stock(), expected_available);
            assert_eq!(item.ledger().reserved_stock(), 0);
            assert_eq!(
                item.ledger().returned_after_close_stock(),
                expected_returned_after_close
            );
            assert!(!outcome.idempotent_replay);
            assert_eq!(outcome.restocked_to_available, expect_restocked);
            assert_eq!(
                event,
                IntegrationEvent::ReservedStockReleased {
                    reservation_id: reservation.id(),
                    item_id: item.id(),
                    quantity: reservation.quantity(),
                    restocked_to_available: expect_restocked,
                    occurred_at: now,
                }
            );
        }
        other => panic!("expected Released, got {other:?}"),
    }
}

#[test]
fn decide_replays_idempotently_when_already_released() {
    let now = base_time();
    let mut item = sample_item(5, 0, 0);
    let mut reservation = sample_reservation(item.id(), 2, ReservationStatus::Released);
    let cmd = ReleaseReservedStockCommand {
        reservation_id: reservation.id(),
    };

    let decision = decide_release_reserved_stock(&cmd, &mut item, &mut reservation, now).unwrap();
    match decision {
        ReleaseReservedStockDecision::IdempotentReplay { outcome } => {
            assert!(outcome.idempotent_replay);
            assert!(!outcome.restocked_to_available);
            assert_eq!(item.ledger().available_stock(), 5);
        }
        other => panic!("expected IdempotentReplay, got {other:?}"),
    }
}

#[rstest]
#[case(ReservationStatus::Confirmed, "confirmed")]
#[case(ReservationStatus::Returned, "returned")]
fn decide_rejects_illegal_reservation_states(
    #[case] status: ReservationStatus,
    #[case] expected_current: &str,
) {
    let now = base_time();
    let mut item = sample_item(3, 2, 0);
    let mut reservation = sample_reservation(item.id(), 2, status);
    let cmd = ReleaseReservedStockCommand {
        reservation_id: reservation.id(),
    };

    let err = decide_release_reserved_stock(&cmd, &mut item, &mut reservation, now).unwrap_err();
    assert_eq!(
        err,
        InventoryPortError::InvalidReservationState {
            current: expected_current.to_owned(),
        }
    );
    assert_eq!(item.ledger().available_stock(), 3);
    assert_eq!(item.ledger().reserved_stock(), 2);
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn pg_release_reserved_stock_handles_window_restock_and_concurrent_replay(pool: PgPool) {
    let now = base_time();
    let clock = Arc::new(FixedClock::new(now));
    let id_gen = Arc::new(SequenceIdGenerator::new(1));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let port = Arc::new(PgInventoryPort::new(
        pool.clone(),
        clock.clone(),
        id_gen,
        event_bus.clone(),
    ));

    let missing_id = ReservationId::new(Uuid::new_v4());
    assert_eq!(
        port.release_reserved_stock(ReleaseReservedStockRequest {
            reservation_id: missing_id,
        })
        .await
        .unwrap_err(),
        InventoryPortError::ReservationNotFound(missing_id)
    );

    let item_id = Uuid::from_u128(0xDDDD_0001);
    sqlx::query(
        r#"
        INSERT INTO inventory.flash_items (
            id, title, description, item_kind, price_cents, currency,
            total_stock, available_stock, reserved_stock, confirmed_stock,
            returned_after_close_stock, per_user_limit, sale_starts_at, sale_ends_at
        )
        VALUES ($1, '预占释放测试品', '描述', 'physical', 9900, 'CNY',
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
    let res_confirmed = ReservationId::new(Uuid::new_v4());

    for (res_id, key) in [
        (res_open, "idem-rel-1"),
        (res_closed, "idem-rel-2"),
        (res_confirmed, "idem-rel-3"),
    ] {
        port.reserve_stock(ReserveStockRequest {
            reservation_id: res_id,
            item_id: ItemId::new(item_id),
            buyer_id: BuyerId::try_new("buyer-rel").unwrap(),
            quantity: Quantity::try_new(1).unwrap(),
            idempotency_key: IdempotencyKey::try_new(key).unwrap(),
        })
        .await
        .unwrap();
    }

    // 并发释放 res_open，仅一次真实释放与写入 outbox，其余为幂等重放
    let mut handles = Vec::new();
    for _ in 0..6 {
        let port = port.clone();
        handles.push(tokio::spawn(async move {
            port.release_reserved_stock(ReleaseReservedStockRequest {
                reservation_id: res_open,
            })
            .await
            .unwrap()
        }));
    }
    let mut outcomes = Vec::new();
    for handle in handles {
        outcomes.push(handle.await.unwrap());
    }
    assert_eq!(outcomes.iter().filter(|o| !o.idempotent_replay).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| !o.idempotent_replay && o.restocked_to_available)
            .count(),
        1
    );
    assert_eq!(outcomes.iter().filter(|o| o.idempotent_replay).count(), 5);

    // 确认 res_confirmed 后不可释放
    port.confirm_stock_deduction(ConfirmStockDeductionRequest {
        reservation_id: res_confirmed,
    })
    .await
    .unwrap();
    let illegal = port
        .release_reserved_stock(ReleaseReservedStockRequest {
            reservation_id: res_confirmed,
        })
        .await
        .unwrap_err();
    assert_eq!(
        illegal,
        InventoryPortError::InvalidReservationState {
            current: "confirmed".to_owned(),
        }
    );

    // 推进时钟至闭窗后释放 res_closed -> 记入 returned_after_close_stock
    clock.advance(Duration::minutes(20));
    let closed_outcome = port
        .release_reserved_stock(ReleaseReservedStockRequest {
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
        (2, 0, 1, 1)
    );

    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory.outbox_events WHERE aggregate_id = $1 AND event_type = 'reserved_stock_released'",
    )
    .bind(item_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(outbox_count, 2);
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn pg_concurrent_confirm_and_release_preserve_ledger_conservation(pool: PgPool) {
    let now = base_time();
    let clock = Arc::new(FixedClock::new(now));
    let id_gen = Arc::new(SequenceIdGenerator::new(1));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let port = Arc::new(PgInventoryPort::new(
        pool.clone(),
        clock,
        id_gen,
        event_bus.clone(),
    ));

    let item_id = Uuid::from_u128(0xDDDD_0002);
    sqlx::query(
        r#"
        INSERT INTO inventory.flash_items (
            id, title, description, item_kind, price_cents, currency,
            total_stock, available_stock, reserved_stock, confirmed_stock,
            returned_after_close_stock, per_user_limit, sale_starts_at, sale_ends_at
        )
        VALUES ($1, '并发状态竞争品', '描述', 'digital', 5000, 'CNY',
                3, 3, 0, 0, 0, 2, $2, $3)
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
        buyer_id: BuyerId::try_new("buyer-race").unwrap(),
        quantity: Quantity::try_new(2).unwrap(),
        idempotency_key: IdempotencyKey::try_new("idem-race-1").unwrap(),
    })
    .await
    .unwrap();

    let p1 = port.clone();
    let p2 = port.clone();
    let h1 = tokio::spawn(async move {
        p1.confirm_stock_deduction(ConfirmStockDeductionRequest {
            reservation_id: res_id,
        })
        .await
    });
    let h2 = tokio::spawn(async move {
        p2.release_reserved_stock(ReleaseReservedStockRequest {
            reservation_id: res_id,
        })
        .await
    });

    let (r1, r2) = (h1.await.unwrap(), h2.await.unwrap());
    match (r1, r2) {
        (Ok(_), Err(InventoryPortError::InvalidReservationState { current })) => {
            assert_eq!(current, "confirmed");
        }
        (Err(InventoryPortError::InvalidReservationState { current }), Ok(_)) => {
            assert_eq!(current, "released");
        }
        other => {
            panic!("expected exactly one winner and one InvalidReservationState, got {other:?}")
        }
    }

    let (avail, reserved, confirmed, returned): (i32, i32, i32, i32) = sqlx::query_as(
        "SELECT available_stock, reserved_stock, confirmed_stock, returned_after_close_stock FROM inventory.flash_items WHERE id = $1",
    )
    .bind(item_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(avail + reserved + confirmed + returned, 3);
    assert_eq!(reserved, 0);
}
