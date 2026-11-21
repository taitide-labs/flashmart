use std::sync::Arc;

use chrono::{Duration, TimeZone, Utc};
use flashmart_contracts::{
    IntegrationEvent, InventoryPort, InventoryPortError, ReserveStockRequest, WindowStatus,
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

use super::super::contract::ReserveStockCommand;
use super::super::workflow::{ReserveStockDecision, decide_reserve_stock};
use crate::port::PgInventoryPort;

fn base_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap()
}

fn sample_item(available_stock: i32, total_stock: i32) -> FlashItem {
    let now = base_time();
    let sale_window =
        SaleWindow::try_new(now - Duration::minutes(10), now + Duration::minutes(30)).unwrap();
    let ledger = InventoryLedger::rehydrate(
        total_stock,
        available_stock,
        total_stock - available_stock,
        0,
        0,
    )
    .unwrap();
    FlashItem::try_new(
        ItemId::new(Uuid::from_u128(0x1001)),
        "创世徽章".to_owned(),
        "测试描述".to_owned(),
        ItemKind::Digital,
        MoneyCents::try_new(9900).unwrap(),
        Currency::Cny,
        1,
        sale_window,
        ledger,
    )
    .unwrap()
}

fn sample_cmd(item_id: ItemId, buyer: &str, qty: i32, key: &str) -> ReserveStockCommand {
    ReserveStockCommand {
        reservation_id: ReservationId::new(Uuid::from_u128(0x2001)),
        item_id,
        buyer_id: BuyerId::try_new(buyer).unwrap(),
        quantity: Quantity::try_new(qty).unwrap(),
        idempotency_key: IdempotencyKey::try_new(key).unwrap(),
    }
}

#[test]
fn decide_reserves_stock_and_emits_event_when_window_active_and_stock_sufficient() {
    let now = base_time();
    let mut item = sample_item(5, 5);
    let cmd = sample_cmd(item.id(), "buyer-1", 2, "idem-1");

    let decision = decide_reserve_stock(&cmd, &mut item, None, now).unwrap();
    match decision {
        ReserveStockDecision::Reserved {
            reservation,
            event,
            outcome,
        } => {
            assert_eq!(reservation.id(), cmd.reservation_id);
            assert_eq!(reservation.status(), ReservationStatus::Reserved);
            assert_eq!(item.ledger().available_stock(), 3);
            assert_eq!(item.ledger().reserved_stock(), 2);
            assert!(!outcome.idempotent_replay);
            assert_eq!(outcome.item_snapshot.available_stock, 3);
            assert_eq!(outcome.item_snapshot.window_status, WindowStatus::Active);
            assert_eq!(
                event,
                IntegrationEvent::StockReserved {
                    reservation_id: cmd.reservation_id,
                    item_id: cmd.item_id,
                    buyer_id: cmd.buyer_id,
                    quantity: cmd.quantity,
                    occurred_at: now,
                }
            );
        }
        other => panic!("expected Reserved, got {other:?}"),
    }
}

#[rstest]
#[case(-11, WindowStatus::Upcoming)]
#[case(31, WindowStatus::Ended)]
fn decide_rejects_outside_sale_window(#[case] offset_mins: i64, #[case] expected: WindowStatus) {
    let now = base_time() + Duration::minutes(offset_mins);
    let mut item = sample_item(5, 5);
    let cmd = sample_cmd(item.id(), "buyer-1", 1, "idem-window");

    let err = decide_reserve_stock(&cmd, &mut item, None, now).unwrap_err();
    assert_eq!(
        err,
        InventoryPortError::SaleWindowNotActive { status: expected }
    );
    assert_eq!(item.ledger().available_stock(), 5);
}

#[test]
fn decide_rejects_when_available_stock_is_insufficient() {
    let now = base_time();
    let mut item = sample_item(1, 5);
    let cmd = sample_cmd(item.id(), "buyer-1", 2, "idem-short");

    let err = decide_reserve_stock(&cmd, &mut item, None, now).unwrap_err();
    assert_eq!(err, InventoryPortError::InsufficientStock);
    assert_eq!(item.ledger().available_stock(), 1);
    assert_eq!(item.ledger().reserved_stock(), 4);
}

#[test]
fn decide_replays_idempotently_and_detects_parameter_conflict() {
    let now = base_time();
    let mut item = sample_item(3, 5);
    let existing_id = ReservationId::new(Uuid::from_u128(0x9999));
    let existing = StockReservation::new_reserved(
        existing_id,
        item.id(),
        BuyerId::try_new("buyer-1").unwrap(),
        Quantity::try_new(2).unwrap(),
        IdempotencyKey::try_new("idem-replay").unwrap(),
    );

    let replay_cmd = ReserveStockCommand {
        reservation_id: ReservationId::new(Uuid::from_u128(0xAAAA)),
        item_id: item.id(),
        buyer_id: BuyerId::try_new("buyer-1").unwrap(),
        quantity: Quantity::try_new(2).unwrap(),
        idempotency_key: IdempotencyKey::try_new("idem-replay").unwrap(),
    };
    let decision = decide_reserve_stock(&replay_cmd, &mut item, Some(&existing), now).unwrap();
    match decision {
        ReserveStockDecision::IdempotentReplay { outcome } => {
            assert_eq!(outcome.reservation_id, existing_id);
            assert!(outcome.idempotent_replay);
            assert_eq!(outcome.item_snapshot.available_stock, 3);
        }
        other => panic!("expected IdempotentReplay, got {other:?}"),
    }

    let conflict_qty_cmd = sample_cmd(item.id(), "buyer-1", 1, "idem-replay");
    assert_eq!(
        decide_reserve_stock(&conflict_qty_cmd, &mut item, Some(&existing), now).unwrap_err(),
        InventoryPortError::IdempotencyConflict
    );

    let conflict_buyer_cmd = sample_cmd(item.id(), "buyer-2", 2, "idem-replay");
    assert_eq!(
        decide_reserve_stock(&conflict_buyer_cmd, &mut item, Some(&existing), now).unwrap_err(),
        InventoryPortError::IdempotencyConflict
    );
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn pg_reserve_stock_enforces_idempotency_outbox_and_window_rules(pool: PgPool) {
    let now = base_time();
    let clock = Arc::new(FixedClock::new(now));
    let id_gen = Arc::new(SequenceIdGenerator::new(1));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let port = PgInventoryPort::new(pool.clone(), clock.clone(), id_gen, event_bus.clone());

    let item_id = Uuid::from_u128(0xBBBB_0001);
    sqlx::query(
        r#"
        INSERT INTO inventory.flash_items (
            id, title, description, item_kind, price_cents, currency,
            total_stock, available_stock, reserved_stock, confirmed_stock,
            returned_after_close_stock, per_user_limit, sale_starts_at, sale_ends_at
        )
        VALUES ($1, '并发测试品', '描述', 'digital', 5000, 'CNY',
                3, 3, 0, 0, 0, 2, $2, $3)
        "#,
    )
    .bind(item_id)
    .bind(now - Duration::minutes(5))
    .bind(now + Duration::minutes(15))
    .execute(&pool)
    .await
    .unwrap();

    // 不存在商品返回 ItemNotFound
    let missing = port
        .reserve_stock(ReserveStockRequest {
            reservation_id: ReservationId::new(Uuid::new_v4()),
            item_id: ItemId::new(Uuid::new_v4()),
            buyer_id: BuyerId::try_new("buyer-a").unwrap(),
            quantity: Quantity::try_new(1).unwrap(),
            idempotency_key: IdempotencyKey::try_new("idem-missing").unwrap(),
        })
        .await
        .unwrap_err();
    assert!(matches!(missing, InventoryPortError::ItemNotFound(_)));

    // 首次预占成功，写 reservation 与 outbox_events，并发布事件
    let first_res_id = ReservationId::new(Uuid::new_v4());
    let first = port
        .reserve_stock(ReserveStockRequest {
            reservation_id: first_res_id,
            item_id: ItemId::new(item_id),
            buyer_id: BuyerId::try_new("buyer-a").unwrap(),
            quantity: Quantity::try_new(2).unwrap(),
            idempotency_key: IdempotencyKey::try_new("idem-pg-1").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(first.reservation_id, first_res_id);
    assert!(!first.idempotent_replay);
    assert_eq!(first.item_snapshot.available_stock, 1);

    // 同幂等键重放不重复扣减也不重复发事件
    let replay = port
        .reserve_stock(ReserveStockRequest {
            reservation_id: ReservationId::new(Uuid::new_v4()),
            item_id: ItemId::new(item_id),
            buyer_id: BuyerId::try_new("buyer-a").unwrap(),
            quantity: Quantity::try_new(2).unwrap(),
            idempotency_key: IdempotencyKey::try_new("idem-pg-1").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(replay.reservation_id, first_res_id);
    assert!(replay.idempotent_replay);
    assert_eq!(replay.item_snapshot.available_stock, 1);

    // 同幂等键参数冲突返回 IdempotencyConflict
    let conflict = port
        .reserve_stock(ReserveStockRequest {
            reservation_id: ReservationId::new(Uuid::new_v4()),
            item_id: ItemId::new(item_id),
            buyer_id: BuyerId::try_new("buyer-a").unwrap(),
            quantity: Quantity::try_new(1).unwrap(),
            idempotency_key: IdempotencyKey::try_new("idem-pg-1").unwrap(),
        })
        .await
        .unwrap_err();
    assert_eq!(conflict, InventoryPortError::IdempotencyConflict);

    // 校验 outbox_events 仅有 1 条记录且 event_bus 仅有 1 条事件
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory.outbox_events WHERE aggregate_id = $1 AND event_type = 'stock_reserved'",
    )
    .bind(item_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(outbox_count, 1);
    assert_eq!(event_bus.snapshot().len(), 1);

    // 推进时钟至窗口结束后，新预占请求被拒绝
    clock.advance(Duration::minutes(20));
    let ended = port
        .reserve_stock(ReserveStockRequest {
            reservation_id: ReservationId::new(Uuid::new_v4()),
            item_id: ItemId::new(item_id),
            buyer_id: BuyerId::try_new("buyer-b").unwrap(),
            quantity: Quantity::try_new(1).unwrap(),
            idempotency_key: IdempotencyKey::try_new("idem-pg-ended").unwrap(),
        })
        .await
        .unwrap_err();
    assert_eq!(
        ended,
        InventoryPortError::SaleWindowNotActive {
            status: WindowStatus::Ended
        }
    );
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn pg_concurrent_same_idempotency_key_deducts_only_once(pool: PgPool) {
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

    let item_id = Uuid::from_u128(0xBBBB_0002);
    sqlx::query(
        r#"
        INSERT INTO inventory.flash_items (
            id, title, description, item_kind, price_cents, currency,
            total_stock, available_stock, reserved_stock, confirmed_stock,
            returned_after_close_stock, per_user_limit, sale_starts_at, sale_ends_at
        )
        VALUES ($1, '并发幂等测试品', '描述', 'digital', 5000, 'CNY',
                5, 5, 0, 0, 0, 2, $2, $3)
        "#,
    )
    .bind(item_id)
    .bind(now - Duration::minutes(5))
    .bind(now + Duration::minutes(15))
    .execute(&pool)
    .await
    .unwrap();

    let mut handles = Vec::new();
    for _ in 0..8 {
        let port = port.clone();
        handles.push(tokio::spawn(async move {
            port.reserve_stock(ReserveStockRequest {
                reservation_id: ReservationId::new(Uuid::new_v4()),
                item_id: ItemId::new(item_id),
                buyer_id: BuyerId::try_new("buyer-race").unwrap(),
                quantity: Quantity::try_new(2).unwrap(),
                idempotency_key: IdempotencyKey::try_new("same-idem-key").unwrap(),
            })
            .await
        }));
    }

    let mut outcomes = Vec::new();
    for handle in handles {
        outcomes.push(handle.await.unwrap().unwrap());
    }

    let first_id = outcomes[0].reservation_id;
    assert!(outcomes.iter().all(|o| o.reservation_id == first_id));
    assert_eq!(outcomes.iter().filter(|o| !o.idempotent_replay).count(), 1);
    assert_eq!(outcomes.iter().filter(|o| o.idempotent_replay).count(), 7);

    let snapshot = port
        .get_flash_item(ItemId::new(item_id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.available_stock, 3);
    assert_eq!(event_bus.snapshot().len(), 1);
}
