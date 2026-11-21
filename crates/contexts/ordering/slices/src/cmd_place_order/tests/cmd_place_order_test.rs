use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, TimeZone, Utc};
use flashmart_contracts::{
    Clock, ConfirmStockDeductionRequest, ConfirmStockOutcome, FlashItemSnapshot, IntegrationEvent,
    InventoryPort, InventoryPortError, OrderLifecycleStatus, ReleaseReservedStockRequest,
    ReleaseStockOutcome, ReserveStockOutcome, ReserveStockRequest, ReturnDeductedStockRequest,
    ReturnStockOutcome, WindowStatus,
};
use flashmart_infra::{AppState, FixedClock, InMemoryEventBus, SequenceIdGenerator};
use flashmart_ordering_domain::OrderingDomainError;
use flashmart_primitives::{
    BuyerId, Currency, IdempotencyKey, ItemId, ItemKind, MoneyCents, OrderId, Quantity,
    ReservationId,
};
use http::StatusCode;
use rstest::rstest;
use sqlx::PgPool;
use uuid::Uuid;

use super::super::contract::{PlaceOrderCommand, PlaceOrderRequest};
use super::super::handler;
use super::super::workflow::{
    PLACE_ORDER_INSTANCE, PlaceOrderInput, compute_request_hash, map_domain_error,
    map_inventory_error, prepare_order,
};

fn base_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap()
}

fn sample_snapshot(
    item_id: ItemId,
    kind: ItemKind,
    price_cents: i64,
    per_user_limit: i32,
) -> FlashItemSnapshot {
    let now = base_time();
    FlashItemSnapshot {
        id: item_id,
        title: "创世数字徽章".to_owned(),
        description: "测试商品".to_owned(),
        item_kind: kind,
        price_cents: MoneyCents::try_new(price_cents).unwrap(),
        currency: Currency::Cny,
        total_stock: 10,
        available_stock: 10,
        per_user_limit,
        sale_starts_at: now - Duration::minutes(10),
        sale_ends_at: now + Duration::minutes(60),
        window_status: WindowStatus::Active,
    }
}

#[derive(Default)]
struct MockInventoryState {
    items: HashMap<ItemId, FlashItemSnapshot>,
    reservations: HashMap<ReservationId, (ItemId, i32, bool)>,
    reserve_calls: usize,
    release_calls: usize,
}

#[derive(Clone)]
struct MockInventoryPort {
    inner: Arc<Mutex<MockInventoryState>>,
}

impl MockInventoryPort {
    fn with_item(snapshot: FlashItemSnapshot) -> Self {
        let mut state = MockInventoryState::default();
        state.items.insert(snapshot.id, snapshot);
        Self {
            inner: Arc::new(Mutex::new(state)),
        }
    }

    fn available_stock(&self, item_id: ItemId) -> i32 {
        self.inner.lock().unwrap().items[&item_id].available_stock
    }

    fn reserve_calls(&self) -> usize {
        self.inner.lock().unwrap().reserve_calls
    }

    fn release_calls(&self) -> usize {
        self.inner.lock().unwrap().release_calls
    }
}

#[async_trait]
impl InventoryPort for MockInventoryPort {
    async fn list_flash_items(&self) -> Result<Vec<FlashItemSnapshot>, InventoryPortError> {
        Ok(self.inner.lock().unwrap().items.values().cloned().collect())
    }

    async fn get_flash_item(
        &self,
        item_id: ItemId,
    ) -> Result<Option<FlashItemSnapshot>, InventoryPortError> {
        Ok(self.inner.lock().unwrap().items.get(&item_id).cloned())
    }

    async fn reserve_stock(
        &self,
        req: ReserveStockRequest,
    ) -> Result<ReserveStockOutcome, InventoryPortError> {
        let mut guard = self.inner.lock().unwrap();
        guard.reserve_calls += 1;
        let item = guard
            .items
            .get_mut(&req.item_id)
            .ok_or(InventoryPortError::ItemNotFound(req.item_id))?;
        if item.window_status != WindowStatus::Active {
            return Err(InventoryPortError::SaleWindowNotActive {
                status: item.window_status,
            });
        }
        if item.available_stock < req.quantity.get() {
            return Err(InventoryPortError::InsufficientStock);
        }
        item.available_stock -= req.quantity.get();
        let snapshot = item.clone();
        guard
            .reservations
            .insert(req.reservation_id, (req.item_id, req.quantity.get(), false));
        Ok(ReserveStockOutcome {
            reservation_id: req.reservation_id,
            item_snapshot: snapshot,
            idempotent_replay: false,
        })
    }

    async fn confirm_stock_deduction(
        &self,
        req: ConfirmStockDeductionRequest,
    ) -> Result<ConfirmStockOutcome, InventoryPortError> {
        Ok(ConfirmStockOutcome {
            reservation_id: req.reservation_id,
            idempotent_replay: false,
        })
    }

    async fn release_reserved_stock(
        &self,
        req: ReleaseReservedStockRequest,
    ) -> Result<ReleaseStockOutcome, InventoryPortError> {
        let mut guard = self.inner.lock().unwrap();
        guard.release_calls += 1;
        let (item_id, qty, released) = guard
            .reservations
            .get_mut(&req.reservation_id)
            .ok_or(InventoryPortError::ReservationNotFound(req.reservation_id))?;
        if *released {
            return Ok(ReleaseStockOutcome {
                reservation_id: req.reservation_id,
                restocked_to_available: false,
                idempotent_replay: true,
            });
        }
        *released = true;
        let item_id = *item_id;
        let qty = *qty;
        guard.items.get_mut(&item_id).unwrap().available_stock += qty;
        Ok(ReleaseStockOutcome {
            reservation_id: req.reservation_id,
            restocked_to_available: true,
            idempotent_replay: false,
        })
    }

    async fn return_deducted_stock(
        &self,
        req: ReturnDeductedStockRequest,
    ) -> Result<ReturnStockOutcome, InventoryPortError> {
        Ok(ReturnStockOutcome {
            reservation_id: req.reservation_id,
            restocked_to_available: false,
            idempotent_replay: false,
        })
    }
}

#[rstest]
#[case("11111111-1111-4111-8111-111111111111", 1, true)]
#[case("  11111111-1111-4111-8111-111111111111  ", 3, true)]
#[case("not-a-uuid", 1, false)]
#[case("11111111-1111-4111-8111-111111111111", 0, false)]
#[case("11111111-1111-4111-8111-111111111111", -2, false)]
fn parse_place_order_command_validates_inputs(
    #[case] raw_item_id: &str,
    #[case] quantity: i32,
    #[case] expected_ok: bool,
) {
    let buyer_id = BuyerId::try_new("buyer-demo").unwrap();
    let key = IdempotencyKey::try_new("idem-1").unwrap();
    let res = PlaceOrderCommand::parse(
        buyer_id,
        key,
        PlaceOrderRequest {
            item_id: raw_item_id.to_owned(),
            quantity,
        },
        PLACE_ORDER_INSTANCE,
    );
    assert_eq!(res.is_ok(), expected_ok);
    if let Err(err) = res {
        assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY.as_u16());
        assert_eq!(err.code, "validation_error");
    }
}

#[test]
fn request_hash_is_deterministic_and_sensitive_to_payload() {
    let buyer = BuyerId::try_new("buyer-demo").unwrap();
    let key = IdempotencyKey::try_new("k1").unwrap();
    let item1 = ItemId::new(Uuid::from_u128(0x1111));
    let item2 = ItemId::new(Uuid::from_u128(0x2222));

    let cmd_a = PlaceOrderCommand {
        buyer_id: buyer.clone(),
        idempotency_key: key.clone(),
        item_id: item1,
        quantity: Quantity::try_new(1).unwrap(),
    };
    let cmd_a_repeat = cmd_a.clone();
    let cmd_diff_qty = PlaceOrderCommand {
        buyer_id: buyer.clone(),
        idempotency_key: key.clone(),
        item_id: item1,
        quantity: Quantity::try_new(2).unwrap(),
    };
    let cmd_diff_item = PlaceOrderCommand {
        buyer_id: buyer,
        idempotency_key: key,
        item_id: item2,
        quantity: Quantity::try_new(1).unwrap(),
    };

    assert_eq!(
        compute_request_hash(&cmd_a),
        compute_request_hash(&cmd_a_repeat)
    );
    assert_ne!(
        compute_request_hash(&cmd_a),
        compute_request_hash(&cmd_diff_qty)
    );
    assert_ne!(
        compute_request_hash(&cmd_a),
        compute_request_hash(&cmd_diff_item)
    );
}

#[rstest]
#[case(1, 0, 1, true)]
#[case(1, 1, 1, false)]
#[case(3, 2, 1, true)]
#[case(3, 2, 2, false)]
fn prepare_order_enforces_active_quota_and_builds_view(
    #[case] limit: i32,
    #[case] active: i32,
    #[case] requested: i32,
    #[case] expected_ok: bool,
) {
    let now = base_time();
    let item_id = ItemId::new(Uuid::from_u128(0xAAAA));
    let res = prepare_order(PlaceOrderInput {
        order_id: OrderId::new(Uuid::from_u128(0x0001)),
        reservation_id: ReservationId::new(Uuid::from_u128(0x0002)),
        buyer_id: BuyerId::try_new("buyer-demo").unwrap(),
        quantity: Quantity::try_new(requested).unwrap(),
        snapshot: sample_snapshot(item_id, ItemKind::Digital, 9900, limit),
        current_active_qty: active,
        payment_hold_duration: Duration::minutes(15),
        now,
    });

    assert_eq!(res.is_ok(), expected_ok);
    if let Ok(prepared) = res {
        assert_eq!(
            prepared.view.total_amount_cents.get(),
            9900 * i64::from(requested)
        );
        assert_eq!(prepared.view.status, OrderLifecycleStatus::PendingPayment);
        assert_eq!(prepared.view.version.get(), 1);
        assert_eq!(prepared.view.payment_deadline, now + Duration::minutes(15));
        assert!(matches!(
            prepared.event,
            IntegrationEvent::FlashOrderPlaced { .. }
        ));
    } else if let Err(err) = res {
        assert_eq!(
            err,
            OrderingDomainError::PurchaseLimitExceeded {
                limit,
                active_qty: active
            }
        );
    }
}

#[test]
fn prepare_order_rejects_amount_overflow_and_zero_hold_duration() {
    let now = base_time();
    let item_id = ItemId::new(Uuid::from_u128(0xBBBB));

    let overflow = prepare_order(PlaceOrderInput {
        order_id: OrderId::new(Uuid::from_u128(0x0001)),
        reservation_id: ReservationId::new(Uuid::from_u128(0x0002)),
        buyer_id: BuyerId::try_new("buyer-demo").unwrap(),
        quantity: Quantity::try_new(2).unwrap(),
        snapshot: sample_snapshot(item_id, ItemKind::Physical, i64::MAX, 5),
        current_active_qty: 0,
        payment_hold_duration: Duration::minutes(15),
        now,
    });
    assert_eq!(overflow.unwrap_err(), OrderingDomainError::AmountOverflow);

    let zero_hold = prepare_order(PlaceOrderInput {
        order_id: OrderId::new(Uuid::from_u128(0x0001)),
        reservation_id: ReservationId::new(Uuid::from_u128(0x0002)),
        buyer_id: BuyerId::try_new("buyer-demo").unwrap(),
        quantity: Quantity::try_new(1).unwrap(),
        snapshot: sample_snapshot(item_id, ItemKind::Physical, 100, 5),
        current_active_qty: 0,
        payment_hold_duration: Duration::zero(),
        now,
    });
    assert_eq!(
        zero_hold.unwrap_err(),
        OrderingDomainError::InvalidPaymentHoldDuration
    );
}

#[test]
fn error_mappers_produce_expected_rfc9457_codes() {
    let item_id = ItemId::new(Uuid::from_u128(0xCCCC));
    let p_not_found = map_inventory_error(
        InventoryPortError::ItemNotFound(item_id),
        PLACE_ORDER_INSTANCE,
    );
    assert_eq!(p_not_found.status, StatusCode::NOT_FOUND.as_u16());
    assert_eq!(p_not_found.code, "item_not_found");

    let p_upcoming = map_inventory_error(
        InventoryPortError::SaleWindowNotActive {
            status: WindowStatus::Upcoming,
        },
        PLACE_ORDER_INSTANCE,
    );
    assert_eq!(p_upcoming.status, StatusCode::CONFLICT.as_u16());
    assert_eq!(p_upcoming.code, "sale_window_not_active");

    let p_stock = map_inventory_error(InventoryPortError::InsufficientStock, PLACE_ORDER_INSTANCE);
    assert_eq!(p_stock.status, StatusCode::CONFLICT.as_u16());
    assert_eq!(p_stock.code, "insufficient_stock");

    let p_idem = map_inventory_error(
        InventoryPortError::IdempotencyConflict,
        PLACE_ORDER_INSTANCE,
    );
    assert_eq!(p_idem.status, StatusCode::CONFLICT.as_u16());
    assert_eq!(p_idem.code, "idempotency_key_conflict");

    let p_unavail = map_inventory_error(
        InventoryPortError::Unavailable("db down".to_owned()),
        PLACE_ORDER_INSTANCE,
    );
    assert_eq!(p_unavail.status, StatusCode::SERVICE_UNAVAILABLE.as_u16());
    assert_eq!(p_unavail.code, "inventory_unavailable");

    let p_limit = map_domain_error(
        OrderingDomainError::PurchaseLimitExceeded {
            limit: 1,
            active_qty: 1,
        },
        PLACE_ORDER_INSTANCE,
    );
    assert_eq!(p_limit.status, StatusCode::CONFLICT.as_u16());
    assert_eq!(p_limit.code, "purchase_limit_exceeded");
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn handler_persists_outbox_and_compensates_stock_on_quota_failure(pool: PgPool) {
    let now = base_time();
    let item_id = ItemId::new(Uuid::from_u128(0xDDDD_0001));
    let snapshot = sample_snapshot(item_id, ItemKind::Digital, 9900, 1);
    let port = Arc::new(MockInventoryPort::with_item(snapshot));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(now));
    let id_gen = Arc::new(SequenceIdGenerator::new(1));
    let state = AppState::new(
        pool.clone(),
        clock,
        id_gen,
        port.clone() as Arc<dyn InventoryPort>,
        event_bus.clone(),
        Duration::minutes(15),
    );

    let buyer = BuyerId::try_new("buyer-demo").unwrap();

    // 1. 首单成功落库 + Outbox
    let (status, view) = handler::handle(
        &state,
        PlaceOrderCommand {
            buyer_id: buyer.clone(),
            idempotency_key: IdempotencyKey::try_new("k1").unwrap(),
            item_id,
            quantity: Quantity::try_new(1).unwrap(),
        },
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(port.available_stock(item_id), 9);

    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ordering.outbox_events WHERE aggregate_id = $1 AND event_type = 'flash_order_placed' AND published_at IS NULL",
    )
    .bind(view.id.into_inner())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(outbox_count, 1);
    assert_eq!(event_bus.snapshot().len(), 1);

    // 2. 重放不触发库存调用
    let reserve_before_replay = port.reserve_calls();
    let (_, replay_view) = handler::handle(
        &state,
        PlaceOrderCommand {
            buyer_id: buyer.clone(),
            idempotency_key: IdempotencyKey::try_new("k1").unwrap(),
            item_id,
            quantity: Quantity::try_new(1).unwrap(),
        },
    )
    .await
    .unwrap();
    assert_eq!(replay_view.id, view.id);
    assert_eq!(port.reserve_calls(), reserve_before_replay);

    // 3. 新幂等键超限购 -> 触发事务外逆向释放补偿
    let err = handler::handle(
        &state,
        PlaceOrderCommand {
            buyer_id: buyer,
            idempotency_key: IdempotencyKey::try_new("k2").unwrap(),
            item_id,
            quantity: Quantity::try_new(1).unwrap(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "purchase_limit_exceeded");
    assert_eq!(port.release_calls(), 1);
    assert_eq!(port.available_stock(item_id), 9);
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn handler_serializes_concurrent_quota_and_idempotency_races(pool: PgPool) {
    let now = base_time();
    let item_id = ItemId::new(Uuid::from_u128(0xDDDD_0002));
    let snapshot = sample_snapshot(item_id, ItemKind::Physical, 19900, 1);
    let port = Arc::new(MockInventoryPort::with_item(snapshot));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(now));
    let id_gen = Arc::new(SequenceIdGenerator::new(100));
    let state = AppState::new(
        pool.clone(),
        clock,
        id_gen,
        port.clone() as Arc<dyn InventoryPort>,
        event_bus,
        Duration::minutes(15),
    );

    // 并发 4 个不同幂等键争抢限购 1 件的商品
    let mut quota_handles = Vec::new();
    for idx in 0..4 {
        let state = state.clone();
        quota_handles.push(tokio::spawn(async move {
            handler::handle(
                &state,
                PlaceOrderCommand {
                    buyer_id: BuyerId::try_new("race-buyer").unwrap(),
                    idempotency_key: IdempotencyKey::try_new(format!("race-k-{idx}")).unwrap(),
                    item_id,
                    quantity: Quantity::try_new(1).unwrap(),
                },
            )
            .await
        }));
    }

    let mut ok_count = 0;
    let mut limit_err_count = 0;
    for h in quota_handles {
        match h.await.unwrap() {
            Ok((status, _)) => {
                assert_eq!(status, StatusCode::CREATED);
                ok_count += 1;
            }
            Err(err) if err.code == "purchase_limit_exceeded" => {
                limit_err_count += 1;
            }
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }
    assert_eq!(ok_count, 1);
    assert_eq!(limit_err_count, 3);
    assert_eq!(port.release_calls(), 3);
    assert_eq!(port.available_stock(item_id), 9);

    // 并发 4 个相同幂等键争抢同一订单
    let mut idem_handles = Vec::new();
    for _ in 0..4 {
        let state = state.clone();
        idem_handles.push(tokio::spawn(async move {
            handler::handle(
                &state,
                PlaceOrderCommand {
                    buyer_id: BuyerId::try_new("idem-buyer").unwrap(),
                    idempotency_key: IdempotencyKey::try_new("same-key").unwrap(),
                    item_id,
                    quantity: Quantity::try_new(1).unwrap(),
                },
            )
            .await
        }));
    }

    let mut order_ids = Vec::new();
    for h in idem_handles {
        let (status, view) = h.await.unwrap().unwrap();
        assert_eq!(status, StatusCode::CREATED);
        order_ids.push(view.id);
    }
    assert!(order_ids.iter().all(|id| *id == order_ids[0]));
    assert_eq!(port.available_stock(item_id), 8);

    let persisted_orders: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ordering.flash_orders WHERE buyer_id = 'idem-buyer'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(persisted_orders, 1);
}
