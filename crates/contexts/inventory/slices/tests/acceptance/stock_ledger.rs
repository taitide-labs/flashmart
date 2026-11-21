use std::sync::Arc;

use chrono::Duration;
use flashmart_contracts::{
    ConfirmStockDeductionRequest, InventoryPort, InventoryPortError, ReleaseReservedStockRequest,
    ReserveStockRequest, ReturnDeductedStockRequest,
};
use flashmart_infra::FixedClock;
use flashmart_primitives::{BuyerId, IdempotencyKey, ItemId, Quantity, ReservationId};
use sqlx::PgPool;
use uuid::Uuid;

use super::support::{TestItemSeed, base_time, insert_flash_item, test_state};

#[sqlx::test(migrations = "../../../../migrations")]
async fn concurrent_reservations_never_oversell(pool: PgPool) {
    let now = base_time();
    let clock = Arc::new(FixedClock::new(now));
    let item_id = Uuid::from_u128(0xAAAA_0001);
    insert_flash_item(
        &pool,
        TestItemSeed {
            id: item_id,
            title: "高并发抢购品",
            item_kind: "digital",
            price_cents: 9900,
            total_stock: 3,
            per_user_limit: 1,
            starts_at: now - Duration::minutes(5),
            ends_at: now + Duration::minutes(30),
        },
    )
    .await;

    let (_, port) = test_state(pool.clone(), clock);
    let mut handles = Vec::new();
    for idx in 0..10 {
        let port = port.clone();
        handles.push(tokio::spawn(async move {
            port.reserve_stock(ReserveStockRequest {
                reservation_id: ReservationId::new(Uuid::new_v4()),
                item_id: ItemId::new(item_id),
                buyer_id: BuyerId::try_new(format!("buyer-{idx}")).unwrap(),
                quantity: Quantity::try_new(1).unwrap(),
                idempotency_key: IdempotencyKey::try_new(format!("idem-{idx}")).unwrap(),
            })
            .await
        }));
    }

    let mut succeeded = 0;
    let mut insufficient = 0;
    for handle in handles {
        match handle.await.unwrap() {
            Ok(_) => succeeded += 1,
            Err(InventoryPortError::InsufficientStock) => insufficient += 1,
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }

    assert_eq!(succeeded, 3);
    assert_eq!(insufficient, 7);

    let snapshot = port
        .get_flash_item(ItemId::new(item_id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.available_stock, 0);
}

#[sqlx::test(migrations = "../../../../migrations")]
#[ignore = "wip"]
async fn release_and_return_follow_sale_window_restock_rule(pool: PgPool) {
    let now = base_time();
    let clock = Arc::new(FixedClock::new(now));
    let item_id = Uuid::from_u128(0xAAAA_0002);
    insert_flash_item(
        &pool,
        TestItemSeed {
            id: item_id,
            title: "时间窗回流测试品",
            item_kind: "physical",
            price_cents: 19900,
            total_stock: 2,
            per_user_limit: 2,
            starts_at: now - Duration::minutes(5),
            ends_at: now + Duration::minutes(10),
        },
    )
    .await;

    let (_, port) = test_state(pool, clock.clone());
    let res1 = ReservationId::new(Uuid::new_v4());
    let res2 = ReservationId::new(Uuid::new_v4());

    port.reserve_stock(ReserveStockRequest {
        reservation_id: res1,
        item_id: ItemId::new(item_id),
        buyer_id: BuyerId::try_new("buyer-1").unwrap(),
        quantity: Quantity::try_new(1).unwrap(),
        idempotency_key: IdempotencyKey::try_new("k1").unwrap(),
    })
    .await
    .unwrap();
    port.reserve_stock(ReserveStockRequest {
        reservation_id: res2,
        item_id: ItemId::new(item_id),
        buyer_id: BuyerId::try_new("buyer-2").unwrap(),
        quantity: Quantity::try_new(1).unwrap(),
        idempotency_key: IdempotencyKey::try_new("k2").unwrap(),
    })
    .await
    .unwrap();

    port.confirm_stock_deduction(ConfirmStockDeductionRequest {
        reservation_id: res2,
    })
    .await
    .unwrap();

    // 窗口期内释放 res1 -> 幂等回流为可售库存
    let released = port
        .release_reserved_stock(ReleaseReservedStockRequest {
            reservation_id: res1,
        })
        .await
        .unwrap();
    assert!(released.restocked_to_available);
    let released_replay = port
        .release_reserved_stock(ReleaseReservedStockRequest {
            reservation_id: res1,
        })
        .await
        .unwrap();
    assert!(released_replay.idempotent_replay);

    let mid = port
        .get_flash_item(ItemId::new(item_id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(mid.available_stock, 1);

    // 窗口结束后退回 res2 -> 仅记账归还，不再增加 available_stock
    clock.advance(Duration::minutes(20));
    let returned = port
        .return_deducted_stock(ReturnDeductedStockRequest {
            reservation_id: res2,
        })
        .await
        .unwrap();
    assert!(!returned.restocked_to_available);

    let after = port
        .get_flash_item(ItemId::new(item_id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.available_stock, 1);
}
