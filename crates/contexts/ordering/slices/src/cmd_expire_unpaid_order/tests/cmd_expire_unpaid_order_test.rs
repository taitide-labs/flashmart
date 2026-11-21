use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, TimeZone, Utc};
use flashmart_contracts::{
    ConfirmStockDeductionRequest, ConfirmStockOutcome, FlashItemSnapshot, IntegrationEvent,
    InventoryPort, InventoryPortError, OrderLifecycleStatus, ReleaseReservedStockRequest,
    ReleaseStockOutcome, ReserveStockOutcome, ReserveStockRequest, ReturnDeductedStockRequest,
    ReturnStockOutcome,
};
use flashmart_infra::{AppState, FixedClock, InMemoryEventBus, SequenceIdGenerator};
use flashmart_ordering_domain::{FlashOrder, OrderState, OrderingDomainError, PaymentHoldDuration};
use flashmart_primitives::{
    BuyerId, Currency, ItemId, ItemKind, MoneyCents, OrderId, OrderVersion, Quantity, ReservationId,
};
use rstest::rstest;
use sqlx::PgPool;
use uuid::Uuid;

use super::super::contract::{ExpireUnpaidOrdersCommand, ExpireUnpaidOrdersError};
use super::super::execute;
use super::super::workflow::{normalize_batch_limit, prepare_expiration};

fn base_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap()
}

fn sample_pending_order(
    order_id: OrderId,
    reservation_id: ReservationId,
    buyer_id: &str,
    placed_at: chrono::DateTime<Utc>,
) -> FlashOrder {
    let hold = PaymentHoldDuration::try_new(Duration::minutes(15)).unwrap();
    let (order, _) = FlashOrder::place(
        order_id,
        reservation_id,
        BuyerId::try_new(buyer_id).unwrap(),
        ItemId::new(Uuid::from_u128(0xAAAA_0001)),
        "创世数字徽章".to_owned(),
        ItemKind::Digital,
        Quantity::try_new(1).unwrap(),
        MoneyCents::try_new(9900).unwrap(),
        Currency::Cny,
        hold,
        placed_at,
    )
    .unwrap();
    order
}

#[derive(Default)]
struct MockExpirePortState {
    release_calls: usize,
    released_ids: Vec<ReservationId>,
    forced_error: Option<InventoryPortError>,
    reservations: HashMap<ReservationId, &'static str>,
}

#[derive(Clone, Default)]
struct MockExpirePort {
    inner: Arc<Mutex<MockExpirePortState>>,
}

impl MockExpirePort {
    fn with_reserved(reservation_ids: &[ReservationId]) -> Self {
        let mut state = MockExpirePortState::default();
        for &id in reservation_ids {
            state.reservations.insert(id, "reserved");
        }
        Self {
            inner: Arc::new(Mutex::new(state)),
        }
    }

    fn with_statuses(entries: &[(ReservationId, &'static str)]) -> Self {
        let mut state = MockExpirePortState::default();
        for &(id, status) in entries {
            state.reservations.insert(id, status);
        }
        Self {
            inner: Arc::new(Mutex::new(state)),
        }
    }

    fn with_error(err: InventoryPortError) -> Self {
        let state = MockExpirePortState {
            forced_error: Some(err),
            ..Default::default()
        };
        Self {
            inner: Arc::new(Mutex::new(state)),
        }
    }

    fn release_calls(&self) -> usize {
        self.inner.lock().unwrap().release_calls
    }

    fn released_ids(&self) -> Vec<ReservationId> {
        self.inner.lock().unwrap().released_ids.clone()
    }
}

#[async_trait]
impl InventoryPort for MockExpirePort {
    async fn list_flash_items(&self) -> Result<Vec<FlashItemSnapshot>, InventoryPortError> {
        Ok(Vec::new())
    }

    async fn get_flash_item(
        &self,
        _item_id: ItemId,
    ) -> Result<Option<FlashItemSnapshot>, InventoryPortError> {
        Ok(None)
    }

    async fn reserve_stock(
        &self,
        _req: ReserveStockRequest,
    ) -> Result<ReserveStockOutcome, InventoryPortError> {
        Err(InventoryPortError::NotImplemented("reserve_stock"))
    }

    async fn confirm_stock_deduction(
        &self,
        _req: ConfirmStockDeductionRequest,
    ) -> Result<ConfirmStockOutcome, InventoryPortError> {
        Err(InventoryPortError::NotImplemented(
            "confirm_stock_deduction",
        ))
    }

    async fn release_reserved_stock(
        &self,
        req: ReleaseReservedStockRequest,
    ) -> Result<ReleaseStockOutcome, InventoryPortError> {
        let mut guard = self.inner.lock().unwrap();
        guard.release_calls += 1;
        if let Some(err) = guard.forced_error.clone() {
            return Err(err);
        }
        let status = guard
            .reservations
            .get_mut(&req.reservation_id)
            .ok_or(InventoryPortError::ReservationNotFound(req.reservation_id))?;
        if *status == "released" {
            return Ok(ReleaseStockOutcome {
                reservation_id: req.reservation_id,
                restocked_to_available: false,
                idempotent_replay: true,
            });
        }
        if *status != "reserved" {
            return Err(InventoryPortError::InvalidReservationState {
                current: (*status).to_owned(),
            });
        }
        *status = "released";
        guard.released_ids.push(req.reservation_id);
        Ok(ReleaseStockOutcome {
            reservation_id: req.reservation_id,
            restocked_to_available: true,
            idempotent_replay: false,
        })
    }

    async fn return_deducted_stock(
        &self,
        _req: ReturnDeductedStockRequest,
    ) -> Result<ReturnStockOutcome, InventoryPortError> {
        Err(InventoryPortError::NotImplemented("return_deducted_stock"))
    }
}

/// 包装端口：在 `release_reserved_stock` 调用期间将目标订单流转为 cancelled，验证行锁与 OCC 竞争处理
struct RaceInjectingPort {
    inner: MockExpirePort,
    pool: PgPool,
    cancel_order_on_release: Mutex<Option<OrderId>>,
}

#[async_trait]
impl InventoryPort for RaceInjectingPort {
    async fn list_flash_items(&self) -> Result<Vec<FlashItemSnapshot>, InventoryPortError> {
        self.inner.list_flash_items().await
    }

    async fn get_flash_item(
        &self,
        item_id: ItemId,
    ) -> Result<Option<FlashItemSnapshot>, InventoryPortError> {
        self.inner.get_flash_item(item_id).await
    }

    async fn reserve_stock(
        &self,
        req: ReserveStockRequest,
    ) -> Result<ReserveStockOutcome, InventoryPortError> {
        self.inner.reserve_stock(req).await
    }

    async fn confirm_stock_deduction(
        &self,
        req: ConfirmStockDeductionRequest,
    ) -> Result<ConfirmStockOutcome, InventoryPortError> {
        self.inner.confirm_stock_deduction(req).await
    }

    async fn release_reserved_stock(
        &self,
        req: ReleaseReservedStockRequest,
    ) -> Result<ReleaseStockOutcome, InventoryPortError> {
        let outcome = self.inner.release_reserved_stock(req).await?;
        let target = self.cancel_order_on_release.lock().unwrap().take();
        if let Some(order_id) = target {
            sqlx::query(
                "UPDATE ordering.flash_orders SET status = 'cancelled', version = 2, cancelled_at = NOW() WHERE id = $1",
            )
            .bind(order_id.into_inner())
            .execute(&self.pool)
            .await
            .unwrap();
        }
        Ok(outcome)
    }

    async fn return_deducted_stock(
        &self,
        req: ReturnDeductedStockRequest,
    ) -> Result<ReturnStockOutcome, InventoryPortError> {
        self.inner.return_deducted_stock(req).await
    }
}

async fn insert_order(pool: &PgPool, order: &FlashOrder) {
    let (paid_at, cancelled_at, expired_at) = match order.state() {
        OrderState::PendingPayment { .. } => (None, None, None),
        OrderState::Paid { paid_at, .. } => (Some(paid_at), None, None),
        OrderState::Cancelled { cancelled_at, .. } => (None, Some(cancelled_at), None),
        OrderState::Expired { expired_at, .. } => (None, None, Some(expired_at)),
    };

    sqlx::query(
        r#"
        INSERT INTO ordering.flash_orders (
            id, reservation_id, buyer_id, item_id, item_title, item_kind,
            quantity, unit_price_cents, total_amount_cents, currency, status,
            version, payment_deadline, created_at, paid_at, cancelled_at,
            expired_at, fulfillment_hint
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
        "#,
    )
    .bind(order.id().into_inner())
    .bind(order.reservation_id().into_inner())
    .bind(order.buyer_id().as_str())
    .bind(order.item_id().into_inner())
    .bind(order.item_title())
    .bind(order.item_kind().as_str())
    .bind(order.quantity().get())
    .bind(order.unit_price_cents().get())
    .bind(order.total_amount_cents().get())
    .bind(order.currency().as_str())
    .bind(order.state().lifecycle_status().as_str())
    .bind(order.version().get())
    .bind(order.state().payment_deadline())
    .bind(order.created_at())
    .bind(paid_at)
    .bind(cancelled_at)
    .bind(expired_at)
    .bind(Some(order.fulfillment_hint()))
    .execute(pool)
    .await
    .unwrap();
}

fn build_state(
    pool: PgPool,
    clock: Arc<FixedClock>,
    port: Arc<dyn InventoryPort>,
) -> (AppState, Arc<InMemoryEventBus>) {
    let id_gen = Arc::new(SequenceIdGenerator::new(100));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let state = AppState::new(
        pool,
        clock,
        id_gen,
        port,
        event_bus.clone(),
        Duration::minutes(15),
    );
    (state, event_bus)
}

#[rstest]
#[case(-5, None)]
#[case(0, None)]
#[case(1, Some(1))]
#[case(50, Some(50))]
fn normalize_batch_limit_rejects_non_positive(#[case] input: i64, #[case] expected: Option<i64>) {
    assert_eq!(normalize_batch_limit(input), expected);
}

#[test]
fn prepare_expiration_transitions_overdue_pending_order() {
    let placed_at = base_time();
    let order_id = OrderId::new(Uuid::from_u128(0xD001));
    let reservation_id = ReservationId::new(Uuid::from_u128(0xE001));
    let order = sample_pending_order(order_id, reservation_id, "buyer-1", placed_at);
    let expire_at = placed_at + Duration::minutes(15) + Duration::seconds(1);

    let prepared = prepare_expiration(order, expire_at).unwrap();
    assert_eq!(prepared.expected_version, OrderVersion::INITIAL);
    assert_eq!(
        prepared.order.state().lifecycle_status(),
        OrderLifecycleStatus::Expired
    );
    assert_eq!(prepared.order.version().get(), 2);
    assert_eq!(prepared.expired_at, expire_at);
    assert!(prepared.order.fulfillment_hint().contains("自动回流"));
    assert_eq!(
        prepared.event,
        IntegrationEvent::FlashOrderExpired {
            order_id,
            reservation_id,
            buyer_id: BuyerId::try_new("buyer-1").unwrap(),
            item_id: ItemId::new(Uuid::from_u128(0xAAAA_0001)),
            quantity: Quantity::try_new(1).unwrap(),
            expired_at: expire_at,
        }
    );
}

#[rstest]
#[case(14)]
#[case(15)]
fn prepare_expiration_rejects_not_due_orders(#[case] elapsed_minutes: i64) {
    let placed_at = base_time();
    let order = sample_pending_order(
        OrderId::new(Uuid::from_u128(0xD002)),
        ReservationId::new(Uuid::from_u128(0xE002)),
        "buyer-1",
        placed_at,
    );

    let err =
        prepare_expiration(order, placed_at + Duration::minutes(elapsed_minutes)).unwrap_err();
    assert_eq!(err, OrderingDomainError::NotDueForExpiration);
}

#[test]
fn prepare_expiration_rejects_non_pending_states() {
    let placed_at = base_time();
    let mut paid = sample_pending_order(
        OrderId::new(Uuid::from_u128(0xD003)),
        ReservationId::new(Uuid::from_u128(0xE003)),
        "buyer-1",
        placed_at,
    );
    paid.pay(
        &BuyerId::try_new("buyer-1").unwrap(),
        OrderVersion::INITIAL,
        placed_at + Duration::minutes(1),
    )
    .unwrap();
    let err_paid = prepare_expiration(paid, placed_at + Duration::minutes(20)).unwrap_err();
    assert_eq!(
        err_paid,
        OrderingDomainError::InvalidOrderState {
            status: OrderLifecycleStatus::Paid,
            action: "expire",
        }
    );

    let mut cancelled = sample_pending_order(
        OrderId::new(Uuid::from_u128(0xD004)),
        ReservationId::new(Uuid::from_u128(0xE004)),
        "buyer-1",
        placed_at,
    );
    cancelled
        .cancel(
            &BuyerId::try_new("buyer-1").unwrap(),
            OrderVersion::INITIAL,
            placed_at + Duration::minutes(2),
        )
        .unwrap();
    let err_cancelled =
        prepare_expiration(cancelled, placed_at + Duration::minutes(20)).unwrap_err();
    assert_eq!(
        err_cancelled,
        OrderingDomainError::InvalidOrderState {
            status: OrderLifecycleStatus::Cancelled,
            action: "expire",
        }
    );

    let mut expired = sample_pending_order(
        OrderId::new(Uuid::from_u128(0xD005)),
        ReservationId::new(Uuid::from_u128(0xE005)),
        "buyer-1",
        placed_at,
    );
    expired.expire(placed_at + Duration::minutes(16)).unwrap();
    let err_expired = prepare_expiration(expired, placed_at + Duration::minutes(20)).unwrap_err();
    assert_eq!(
        err_expired,
        OrderingDomainError::InvalidOrderState {
            status: OrderLifecycleStatus::Expired,
            action: "expire",
        }
    );
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn execute_expires_only_overdue_pending_orders_and_writes_outbox(pool: PgPool) {
    let t0 = base_time();
    let scan_at = t0 + Duration::minutes(20);
    let clock = Arc::new(FixedClock::new(scan_at));

    let due_id = OrderId::new(Uuid::from_u128(0xD101));
    let due_res_id = ReservationId::new(Uuid::from_u128(0xE101));
    let due_order = sample_pending_order(due_id, due_res_id, "buyer-due", t0);

    // deadline 恰好等于 scan_at（未严格超过），不应过期
    let boundary_id = OrderId::new(Uuid::from_u128(0xD102));
    let boundary_res_id = ReservationId::new(Uuid::from_u128(0xE102));
    let boundary_order = sample_pending_order(
        boundary_id,
        boundary_res_id,
        "buyer-boundary",
        scan_at - Duration::minutes(15),
    );

    // 未到期订单，不应过期
    let fresh_id = OrderId::new(Uuid::from_u128(0xD103));
    let fresh_res_id = ReservationId::new(Uuid::from_u128(0xE103));
    let fresh_order = sample_pending_order(
        fresh_id,
        fresh_res_id,
        "buyer-fresh",
        scan_at - Duration::minutes(5),
    );

    // 已支付但超过原支付截止时刻的订单，不应过期
    let paid_id = OrderId::new(Uuid::from_u128(0xD104));
    let paid_res_id = ReservationId::new(Uuid::from_u128(0xE104));
    let mut paid_order = sample_pending_order(paid_id, paid_res_id, "buyer-paid", t0);
    paid_order
        .pay(
            &BuyerId::try_new("buyer-paid").unwrap(),
            OrderVersion::INITIAL,
            t0 + Duration::minutes(2),
        )
        .unwrap();

    insert_order(&pool, &due_order).await;
    insert_order(&pool, &boundary_order).await;
    insert_order(&pool, &fresh_order).await;
    insert_order(&pool, &paid_order).await;

    let port = Arc::new(MockExpirePort::with_reserved(&[
        due_res_id,
        boundary_res_id,
        fresh_res_id,
    ]));
    let (state, event_bus) = build_state(pool.clone(), clock, port.clone());

    let outcome = execute(&state, ExpireUnpaidOrdersCommand::default())
        .await
        .unwrap();
    assert_eq!(outcome.expired_order_ids, vec![due_id]);
    assert_eq!(port.release_calls(), 1);
    assert_eq!(port.released_ids(), vec![due_res_id]);

    let due_row: (String, i32, Option<chrono::DateTime<Utc>>) = sqlx::query_as(
        "SELECT status, version, expired_at FROM ordering.flash_orders WHERE id = $1",
    )
    .bind(due_id.into_inner())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(due_row.0, "expired");
    assert_eq!(due_row.1, 2);
    assert_eq!(due_row.2, Some(scan_at));

    let boundary_status: String =
        sqlx::query_scalar("SELECT status FROM ordering.flash_orders WHERE id = $1")
            .bind(boundary_id.into_inner())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(boundary_status, "pending_payment");

    let outbox_rows: Vec<(String, Uuid, serde_json::Value)> = sqlx::query_as(
        "SELECT event_type, aggregate_id, payload FROM ordering.outbox_events ORDER BY occurred_at ASC",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(outbox_rows.len(), 1);
    assert_eq!(outbox_rows[0].0, "flash_order_expired");
    assert_eq!(outbox_rows[0].1, due_id.into_inner());
    let parsed_event: IntegrationEvent = serde_json::from_value(outbox_rows[0].2.clone()).unwrap();
    assert!(matches!(
        parsed_event,
        IntegrationEvent::FlashOrderExpired { order_id, .. } if order_id == due_id
    ));

    let published = event_bus.snapshot();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0], parsed_event);

    // 重复执行不再重复扫描或重复释放库存
    let second = execute(&state, ExpireUnpaidOrdersCommand::default())
        .await
        .unwrap();
    assert!(second.expired_order_ids.is_empty());
    assert_eq!(port.release_calls(), 1);
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn execute_respects_batch_limit_and_zero_limit(pool: PgPool) {
    let t0 = base_time();
    let scan_at = t0 + Duration::minutes(30);
    let clock = Arc::new(FixedClock::new(scan_at));

    let oldest_id = OrderId::new(Uuid::from_u128(0xD201));
    let oldest_res_id = ReservationId::new(Uuid::from_u128(0xE201));
    let oldest_order = sample_pending_order(oldest_id, oldest_res_id, "buyer-1", t0);

    let newer_id = OrderId::new(Uuid::from_u128(0xD202));
    let newer_res_id = ReservationId::new(Uuid::from_u128(0xE202));
    let newer_order =
        sample_pending_order(newer_id, newer_res_id, "buyer-2", t0 + Duration::minutes(5));

    insert_order(&pool, &newer_order).await;
    insert_order(&pool, &oldest_order).await;

    let port = Arc::new(MockExpirePort::with_reserved(&[
        oldest_res_id,
        newer_res_id,
    ]));
    let (state, _) = build_state(pool, clock, port.clone());

    let empty = execute(&state, ExpireUnpaidOrdersCommand { batch_limit: 0 })
        .await
        .unwrap();
    assert!(empty.expired_order_ids.is_empty());
    assert_eq!(port.release_calls(), 0);

    let first_batch = execute(&state, ExpireUnpaidOrdersCommand { batch_limit: 1 })
        .await
        .unwrap();
    assert_eq!(first_batch.expired_order_ids, vec![oldest_id]);

    let second_batch = execute(&state, ExpireUnpaidOrdersCommand { batch_limit: 1 })
        .await
        .unwrap();
    assert_eq!(second_batch.expired_order_ids, vec![newer_id]);
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn execute_propagates_inventory_failure_without_mutating_order(pool: PgPool) {
    let t0 = base_time();
    let scan_at = t0 + Duration::minutes(20);
    let clock = Arc::new(FixedClock::new(scan_at));

    let order_id = OrderId::new(Uuid::from_u128(0xD301));
    let res_id = ReservationId::new(Uuid::from_u128(0xE301));
    let order = sample_pending_order(order_id, res_id, "buyer-1", t0);
    insert_order(&pool, &order).await;

    let port = Arc::new(MockExpirePort::with_error(InventoryPortError::Unavailable(
        "inventory down".to_owned(),
    )));
    let (state, event_bus) = build_state(pool.clone(), clock, port.clone());

    let err = execute(&state, ExpireUnpaidOrdersCommand::default())
        .await
        .unwrap_err();
    assert!(matches!(err, ExpireUnpaidOrdersError::Inventory(_)));
    assert_eq!(port.release_calls(), 1);
    assert!(event_bus.snapshot().is_empty());

    let row: (String, i32, Option<chrono::DateTime<Utc>>) = sqlx::query_as(
        "SELECT status, version, expired_at FROM ordering.flash_orders WHERE id = $1",
    )
    .bind(order_id.into_inner())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.0, "pending_payment");
    assert_eq!(row.1, 1);
    assert_eq!(row.2, None);

    let outbox_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ordering.outbox_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(outbox_count, 0);
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn execute_skips_order_when_concurrent_tx_already_transitioned_it(pool: PgPool) {
    let t0 = base_time();
    let scan_at = t0 + Duration::minutes(20);
    let clock = Arc::new(FixedClock::new(scan_at));

    let order_id = OrderId::new(Uuid::from_u128(0xD401));
    let res_id = ReservationId::new(Uuid::from_u128(0xE401));
    let order = sample_pending_order(order_id, res_id, "buyer-1", t0);
    insert_order(&pool, &order).await;

    let race_port = Arc::new(RaceInjectingPort {
        inner: MockExpirePort::with_reserved(&[res_id]),
        pool: pool.clone(),
        cancel_order_on_release: Mutex::new(Some(order_id)),
    });

    let (state, event_bus) = build_state(pool.clone(), clock, race_port);
    let outcome = execute(&state, ExpireUnpaidOrdersCommand::default())
        .await
        .unwrap();
    assert!(outcome.expired_order_ids.is_empty());
    assert!(event_bus.snapshot().is_empty());

    let row: (String, i32) =
        sqlx::query_as("SELECT status, version FROM ordering.flash_orders WHERE id = $1")
            .bind(order_id.into_inner())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row.0, "cancelled");
    assert_eq!(row.1, 2);

    let outbox_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ordering.outbox_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(outbox_count, 0);
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn execute_skips_invalid_reservation_state_when_order_already_paid(pool: PgPool) {
    let t0 = base_time();
    let scan_at = t0 + Duration::minutes(20);
    let clock = Arc::new(FixedClock::new(scan_at));

    let raced_order_id = OrderId::new(Uuid::from_u128(0xD501));
    let raced_res_id = ReservationId::new(Uuid::from_u128(0xE501));
    let due_order_id = OrderId::new(Uuid::from_u128(0xD502));
    let due_res_id = ReservationId::new(Uuid::from_u128(0xE502));

    let raced_order = sample_pending_order(raced_order_id, raced_res_id, "buyer-1", t0);
    let due_order = sample_pending_order(
        due_order_id,
        due_res_id,
        "buyer-2",
        t0 + Duration::minutes(1),
    );
    insert_order(&pool, &raced_order).await;
    insert_order(&pool, &due_order).await;

    // 模拟 raced_res_id 已被确认扣减，同时订单在库中也已流转为 paid，但未影响后续待过期订单
    struct ConfirmedRacePort {
        inner: MockExpirePort,
        pool: PgPool,
        raced_order_id: OrderId,
        raced_res_id: ReservationId,
    }

    #[async_trait]
    impl InventoryPort for ConfirmedRacePort {
        async fn list_flash_items(&self) -> Result<Vec<FlashItemSnapshot>, InventoryPortError> {
            self.inner.list_flash_items().await
        }
        async fn get_flash_item(
            &self,
            item_id: ItemId,
        ) -> Result<Option<FlashItemSnapshot>, InventoryPortError> {
            self.inner.get_flash_item(item_id).await
        }
        async fn reserve_stock(
            &self,
            req: ReserveStockRequest,
        ) -> Result<ReserveStockOutcome, InventoryPortError> {
            self.inner.reserve_stock(req).await
        }
        async fn confirm_stock_deduction(
            &self,
            req: ConfirmStockDeductionRequest,
        ) -> Result<ConfirmStockOutcome, InventoryPortError> {
            self.inner.confirm_stock_deduction(req).await
        }
        async fn release_reserved_stock(
            &self,
            req: ReleaseReservedStockRequest,
        ) -> Result<ReleaseStockOutcome, InventoryPortError> {
            if req.reservation_id == self.raced_res_id {
                sqlx::query(
                    "UPDATE ordering.flash_orders SET status = 'paid', version = 2, paid_at = NOW() WHERE id = $1",
                )
                .bind(self.raced_order_id.into_inner())
                .execute(&self.pool)
                .await
                .unwrap();
            }
            self.inner.release_reserved_stock(req).await
        }
        async fn return_deducted_stock(
            &self,
            req: ReturnDeductedStockRequest,
        ) -> Result<ReturnStockOutcome, InventoryPortError> {
            self.inner.return_deducted_stock(req).await
        }
    }

    let port = Arc::new(ConfirmedRacePort {
        inner: MockExpirePort::with_statuses(&[
            (raced_res_id, "confirmed"),
            (due_res_id, "reserved"),
        ]),
        pool: pool.clone(),
        raced_order_id,
        raced_res_id,
    });
    let (state, _) = build_state(pool.clone(), clock, port);

    let outcome = execute(&state, ExpireUnpaidOrdersCommand::default())
        .await
        .unwrap();
    assert_eq!(outcome.expired_order_ids, vec![due_order_id]);

    let raced_status: String =
        sqlx::query_scalar("SELECT status FROM ordering.flash_orders WHERE id = $1")
            .bind(raced_order_id.into_inner())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(raced_status, "paid");
}
