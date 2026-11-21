use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, TimeZone, Utc};
use flashmart_contracts::{
    ConfirmStockDeductionRequest, ConfirmStockOutcome, FlashItemSnapshot, IntegrationEvent,
    InventoryPort, InventoryPortError, OrderLifecycleStatus, ReleaseReservedStockRequest,
    ReleaseStockOutcome, ReserveStockOutcome, ReserveStockRequest, ReturnDeductedStockRequest,
    ReturnStockOutcome, WindowStatus,
};
use flashmart_infra::{AppState, FixedClock, InMemoryEventBus, SequenceIdGenerator};
use flashmart_ordering_domain::{FlashOrder, OrderState, OrderingDomainError, PaymentHoldDuration};
use flashmart_primitives::{
    BuyerId, Currency, IdempotencyKey, ItemId, ItemKind, MoneyCents, OrderId, OrderVersion,
    Quantity, ReservationId,
};
use http::StatusCode;
use rstest::rstest;
use sqlx::PgPool;
use uuid::Uuid;

use super::super::contract::{CancelOrderCommand, CancelOrderRequest};
use super::super::handler;
use super::super::workflow::{
    cancel_order_instance, compute_request_hash, idempotency_conflict_problem, map_domain_error,
    map_inventory_error, order_not_found_problem, prepare_cancellation,
};

fn base_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap()
}

fn sample_pending_order(
    order_id: OrderId,
    reservation_id: ReservationId,
    buyer_id: &str,
    item_kind: ItemKind,
    now: chrono::DateTime<Utc>,
) -> FlashOrder {
    let hold = PaymentHoldDuration::try_new(Duration::minutes(15)).unwrap();
    let (order, _) = FlashOrder::place(
        order_id,
        reservation_id,
        BuyerId::try_new(buyer_id).unwrap(),
        ItemId::new(Uuid::from_u128(0xAAAA_0001)),
        match item_kind {
            ItemKind::Digital => "创世数字徽章".to_owned(),
            ItemKind::Physical => "机械旋钮宏键盘".to_owned(),
        },
        item_kind,
        Quantity::try_new(1).unwrap(),
        MoneyCents::try_new(9900).unwrap(),
        Currency::Cny,
        hold,
        now,
    )
    .unwrap();
    order
}

fn sample_paid_order(
    order_id: OrderId,
    reservation_id: ReservationId,
    buyer_id: &str,
    item_kind: ItemKind,
    placed_at: chrono::DateTime<Utc>,
    paid_at: chrono::DateTime<Utc>,
) -> FlashOrder {
    let mut order = sample_pending_order(order_id, reservation_id, buyer_id, item_kind, placed_at);
    order
        .pay(
            &BuyerId::try_new(buyer_id).unwrap(),
            OrderVersion::INITIAL,
            paid_at,
        )
        .unwrap();
    order
}

#[derive(Default)]
struct MockCancelPortState {
    release_calls: usize,
    return_calls: usize,
    released_ids: Vec<ReservationId>,
    returned_ids: Vec<ReservationId>,
    forced_error: Option<InventoryPortError>,
    reservations: HashMap<ReservationId, &'static str>,
}

#[derive(Clone, Default)]
struct MockCancelPort {
    inner: Arc<Mutex<MockCancelPortState>>,
}

impl MockCancelPort {
    fn with_reservation(reservation_id: ReservationId, status: &'static str) -> Self {
        let mut state = MockCancelPortState::default();
        state.reservations.insert(reservation_id, status);
        Self {
            inner: Arc::new(Mutex::new(state)),
        }
    }

    fn with_error(err: InventoryPortError) -> Self {
        let state = MockCancelPortState {
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

    fn return_calls(&self) -> usize {
        self.inner.lock().unwrap().return_calls
    }
}

#[async_trait]
impl InventoryPort for MockCancelPort {
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
        req: ReturnDeductedStockRequest,
    ) -> Result<ReturnStockOutcome, InventoryPortError> {
        let mut guard = self.inner.lock().unwrap();
        guard.return_calls += 1;
        if let Some(err) = guard.forced_error.clone() {
            return Err(err);
        }
        let status = guard
            .reservations
            .get_mut(&req.reservation_id)
            .ok_or(InventoryPortError::ReservationNotFound(req.reservation_id))?;
        if *status == "returned" {
            return Ok(ReturnStockOutcome {
                reservation_id: req.reservation_id,
                restocked_to_available: false,
                idempotent_replay: true,
            });
        }
        if *status != "confirmed" {
            return Err(InventoryPortError::InvalidReservationState {
                current: (*status).to_owned(),
            });
        }
        *status = "returned";
        guard.returned_ids.push(req.reservation_id);
        Ok(ReturnStockOutcome {
            reservation_id: req.reservation_id,
            restocked_to_available: true,
            idempotent_replay: false,
        })
    }
}

async fn insert_order_row(pool: &PgPool, order: &FlashOrder) {
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
        VALUES (
            $1, $2, $3, $4, $5, $6,
            $7, $8, $9, $10, $11,
            $12, $13, $14, $15, $16,
            $17, $18
        )
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

#[rstest]
#[case("11111111-1111-4111-8111-111111111111", 1, true)]
#[case("  11111111-1111-4111-8111-111111111111  ", 2, true)]
#[case("bad-uuid", 1, false)]
#[case("11111111-1111-4111-8111-111111111111", 0, false)]
#[case("11111111-1111-4111-8111-111111111111", -3, false)]
fn parse_cancel_order_command_validates_inputs(
    #[case] raw_order_id: &str,
    #[case] version: i32,
    #[case] expected_ok: bool,
) {
    let buyer = BuyerId::try_new("buyer-demo").unwrap();
    let key = IdempotencyKey::try_new("cancel-k1").unwrap();
    let res = CancelOrderCommand::parse(
        buyer,
        key,
        raw_order_id,
        CancelOrderRequest { version },
        "/api/orders/test/cancel",
    );
    assert_eq!(res.is_ok(), expected_ok);
    if let Err(err) = res {
        assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY.as_u16());
        assert_eq!(err.code, "validation_error");
    }
}

#[test]
fn request_hash_is_deterministic_and_sensitive_to_order_and_version() {
    let buyer = BuyerId::try_new("buyer-demo").unwrap();
    let key = IdempotencyKey::try_new("k1").unwrap();
    let order1 = OrderId::new(Uuid::from_u128(0x1111));
    let order2 = OrderId::new(Uuid::from_u128(0x2222));

    let cmd_a = CancelOrderCommand {
        buyer_id: buyer.clone(),
        idempotency_key: key.clone(),
        order_id: order1,
        expected_version: OrderVersion::try_new(1).unwrap(),
    };
    let cmd_a_repeat = cmd_a.clone();
    let cmd_diff_ver = CancelOrderCommand {
        buyer_id: buyer.clone(),
        idempotency_key: key.clone(),
        order_id: order1,
        expected_version: OrderVersion::try_new(2).unwrap(),
    };
    let cmd_diff_order = CancelOrderCommand {
        buyer_id: buyer,
        idempotency_key: key,
        order_id: order2,
        expected_version: OrderVersion::try_new(1).unwrap(),
    };

    assert_eq!(
        compute_request_hash(&cmd_a),
        compute_request_hash(&cmd_a_repeat)
    );
    assert_ne!(
        compute_request_hash(&cmd_a),
        compute_request_hash(&cmd_diff_ver)
    );
    assert_ne!(
        compute_request_hash(&cmd_a),
        compute_request_hash(&cmd_diff_order)
    );
}

#[test]
fn prepare_cancellation_transitions_pending_and_paid_orders() {
    let now = base_time();
    let caller = BuyerId::try_new("buyer-demo").unwrap();

    // 1. 待支付取消 -> was_paid = false, paid_at = None
    let pending_id = OrderId::new(Uuid::from_u128(0x1001));
    let pending_res = ReservationId::new(Uuid::from_u128(0x2001));
    let pending_order = sample_pending_order(
        pending_id,
        pending_res,
        "buyer-demo",
        ItemKind::Digital,
        now,
    );
    let cancel_pending_at = now + Duration::minutes(3);
    let prepared_pending = prepare_cancellation(
        pending_order,
        &caller,
        OrderVersion::INITIAL,
        cancel_pending_at,
    )
    .unwrap();

    assert!(!prepared_pending.was_paid);
    assert_eq!(
        prepared_pending.view.status,
        OrderLifecycleStatus::Cancelled
    );
    assert_eq!(prepared_pending.view.version.get(), 2);
    assert_eq!(prepared_pending.view.paid_at, None);
    assert_eq!(prepared_pending.view.cancelled_at, Some(cancel_pending_at));
    assert_eq!(prepared_pending.view.expired_at, None);
    assert_eq!(
        prepared_pending.view.fulfillment_hint.as_deref(),
        Some("已主动取消，预占库存已释放。")
    );
    assert!(matches!(
        prepared_pending.event,
        IntegrationEvent::FlashOrderCancelled {
            order_id,
            reservation_id,
            was_paid: false,
            cancelled_at,
            ..
        } if order_id == pending_id && reservation_id == pending_res && cancelled_at == cancel_pending_at
    ));

    // 2. 已支付取消 -> was_paid = true, 保留原 paid_at 并生成退款提示
    let paid_id = OrderId::new(Uuid::from_u128(0x1002));
    let paid_res = ReservationId::new(Uuid::from_u128(0x2002));
    let paid_at = now + Duration::minutes(1);
    let paid_order = sample_paid_order(
        paid_id,
        paid_res,
        "buyer-demo",
        ItemKind::Physical,
        now,
        paid_at,
    );
    let cancel_paid_at = now + Duration::minutes(5);
    let prepared_paid = prepare_cancellation(
        paid_order,
        &caller,
        OrderVersion::try_new(2).unwrap(),
        cancel_paid_at,
    )
    .unwrap();

    assert!(prepared_paid.was_paid);
    assert_eq!(prepared_paid.view.status, OrderLifecycleStatus::Cancelled);
    assert_eq!(prepared_paid.view.version.get(), 3);
    assert_eq!(prepared_paid.view.paid_at, Some(paid_at));
    assert_eq!(prepared_paid.view.cancelled_at, Some(cancel_paid_at));
    assert_eq!(prepared_paid.view.expired_at, None);
    assert_eq!(
        prepared_paid.view.fulfillment_hint.as_deref(),
        Some("已取消并完成模拟原路退款，库存已退回。")
    );
    assert!(matches!(
        prepared_paid.event,
        IntegrationEvent::FlashOrderCancelled {
            order_id,
            reservation_id,
            was_paid: true,
            cancelled_at,
            ..
        } if order_id == paid_id && reservation_id == paid_res && cancelled_at == cancel_paid_at
    ));
}

#[test]
fn prepare_cancellation_enforces_ownership_occ_and_terminal_states() {
    let now = base_time();
    let order_id = OrderId::new(Uuid::from_u128(0x1234));
    let reservation_id = ReservationId::new(Uuid::from_u128(0x5678));
    let order = sample_pending_order(
        order_id,
        reservation_id,
        "buyer-demo",
        ItemKind::Digital,
        now,
    );

    // 他人操作
    let err_owner = prepare_cancellation(
        order.clone(),
        &BuyerId::try_new("buyer-other").unwrap(),
        OrderVersion::INITIAL,
        now + Duration::minutes(1),
    )
    .unwrap_err();
    assert_eq!(err_owner, OrderingDomainError::OwnershipMismatch);

    // 版本冲突
    let err_ver = prepare_cancellation(
        order.clone(),
        &BuyerId::try_new("buyer-demo").unwrap(),
        OrderVersion::try_new(2).unwrap(),
        now + Duration::minutes(1),
    )
    .unwrap_err();
    assert_eq!(
        err_ver,
        OrderingDomainError::VersionConflict {
            expected: 2,
            actual: 1
        }
    );

    // 已取消状态不可重复取消
    let mut cancelled_order = order.clone();
    cancelled_order
        .cancel(
            &BuyerId::try_new("buyer-demo").unwrap(),
            OrderVersion::INITIAL,
            now + Duration::minutes(2),
        )
        .unwrap();
    let err_cancelled = prepare_cancellation(
        cancelled_order,
        &BuyerId::try_new("buyer-demo").unwrap(),
        OrderVersion::try_new(2).unwrap(),
        now + Duration::minutes(3),
    )
    .unwrap_err();
    assert_eq!(
        err_cancelled,
        OrderingDomainError::InvalidOrderState {
            status: OrderLifecycleStatus::Cancelled,
            action: "cancel"
        }
    );

    // 已过期状态不可取消
    let mut expired_order = order;
    expired_order.expire(now + Duration::minutes(16)).unwrap();
    let err_expired = prepare_cancellation(
        expired_order,
        &BuyerId::try_new("buyer-demo").unwrap(),
        OrderVersion::try_new(2).unwrap(),
        now + Duration::minutes(17),
    )
    .unwrap_err();
    assert_eq!(
        err_expired,
        OrderingDomainError::InvalidOrderState {
            status: OrderLifecycleStatus::Expired,
            action: "cancel"
        }
    );
}

#[test]
fn error_mappers_produce_expected_rfc9457_codes() {
    let order_id = OrderId::new(Uuid::from_u128(0x1001));
    let instance = cancel_order_instance(order_id);

    let p_missing = order_not_found_problem(order_id, &instance);
    assert_eq!(p_missing.status, StatusCode::NOT_FOUND.as_u16());
    assert_eq!(p_missing.code, "order_not_found");

    let p_idem = idempotency_conflict_problem(&instance);
    assert_eq!(p_idem.status, StatusCode::CONFLICT.as_u16());
    assert_eq!(p_idem.code, "idempotency_key_conflict");

    let p_owner = map_domain_error(OrderingDomainError::OwnershipMismatch, &instance);
    assert_eq!(p_owner.status, StatusCode::FORBIDDEN.as_u16());
    assert_eq!(p_owner.code, "order_ownership_mismatch");

    let p_ver = map_domain_error(
        OrderingDomainError::VersionConflict {
            expected: 1,
            actual: 2,
        },
        &instance,
    );
    assert_eq!(p_ver.status, StatusCode::CONFLICT.as_u16());
    assert_eq!(p_ver.code, "order_version_conflict");

    let p_state = map_domain_error(
        OrderingDomainError::InvalidOrderState {
            status: OrderLifecycleStatus::Expired,
            action: "cancel",
        },
        &instance,
    );
    assert_eq!(p_state.status, StatusCode::CONFLICT.as_u16());
    assert_eq!(p_state.code, "invalid_order_state");

    let p_inv_state = map_inventory_error(
        InventoryPortError::InvalidReservationState {
            current: "confirmed".to_owned(),
        },
        &instance,
    );
    assert_eq!(p_inv_state.status, StatusCode::CONFLICT.as_u16());
    assert_eq!(p_inv_state.code, "invalid_reservation_state");

    let p_inv_unavail = map_inventory_error(
        InventoryPortError::Unavailable("db down".to_owned()),
        &instance,
    );
    assert_eq!(
        p_inv_unavail.status,
        StatusCode::SERVICE_UNAVAILABLE.as_u16()
    );
    assert_eq!(p_inv_unavail.code, "inventory_unavailable");

    let p_inv_not_impl =
        map_inventory_error(InventoryPortError::NotImplemented("cancel"), &instance);
    assert_eq!(
        p_inv_not_impl.status,
        StatusCode::SERVICE_UNAVAILABLE.as_u16()
    );
    assert_eq!(p_inv_not_impl.code, "inventory_not_implemented");

    let p_inv_window = map_inventory_error(
        InventoryPortError::SaleWindowNotActive {
            status: WindowStatus::Ended,
        },
        &instance,
    );
    assert_eq!(p_inv_window.status, StatusCode::CONFLICT.as_u16());
    assert_eq!(p_inv_window.code, "sale_window_not_active");
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn handler_cancels_pending_order_releases_stock_writes_outbox_and_replays_idempotently(
    pool: PgPool,
) {
    let now = base_time();
    let clock = Arc::new(FixedClock::new(now));
    let id_gen = Arc::new(SequenceIdGenerator::new(1));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let order_id = OrderId::new(Uuid::from_u128(0xABCD_0001));
    let reservation_id = ReservationId::new(Uuid::from_u128(0xABCD_0002));
    let pending_order = sample_pending_order(
        order_id,
        reservation_id,
        "buyer-demo",
        ItemKind::Digital,
        now,
    );
    insert_order_row(&pool, &pending_order).await;

    let port = Arc::new(MockCancelPort::with_reservation(reservation_id, "reserved"));
    let state = AppState::new(
        pool.clone(),
        clock.clone(),
        id_gen,
        port.clone() as Arc<dyn InventoryPort>,
        event_bus.clone(),
        Duration::minutes(15),
    );

    clock.advance(Duration::minutes(2));
    let cmd = CancelOrderCommand {
        buyer_id: BuyerId::try_new("buyer-demo").unwrap(),
        idempotency_key: IdempotencyKey::try_new("cancel-pending-1").unwrap(),
        order_id,
        expected_version: OrderVersion::INITIAL,
    };

    let view = handler::handle(&state, cmd.clone()).await.unwrap();
    assert_eq!(view.status, OrderLifecycleStatus::Cancelled);
    assert_eq!(view.version.get(), 2);
    assert_eq!(view.paid_at, None);
    assert!(view.cancelled_at.is_some());
    assert_eq!(
        view.fulfillment_hint.as_deref(),
        Some("已主动取消，预占库存已释放。")
    );
    assert_eq!(port.release_calls(), 1);
    assert_eq!(port.return_calls(), 0);

    // 验证订单状态、有效限购占用清零与 Outbox 事件
    let active_qty: i64 = sqlx::query_scalar(
        r#"
        SELECT COALESCE(SUM(quantity), 0)::BIGINT
        FROM ordering.flash_orders
        WHERE buyer_id = 'buyer-demo'
          AND item_id = $1
          AND status IN ('pending_payment', 'paid')
        "#,
    )
    .bind(pending_order.item_id().into_inner())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(active_qty, 0);

    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ordering.outbox_events WHERE aggregate_id = $1 AND event_type = 'flash_order_cancelled'",
    )
    .bind(order_id.into_inner())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(outbox_count, 1);

    let published = event_bus.snapshot();
    assert_eq!(published.len(), 1);
    assert!(matches!(
        &published[0],
        IntegrationEvent::FlashOrderCancelled {
            order_id: oid,
            was_paid: false,
            ..
        } if *oid == order_id
    ));

    // 同幂等键 + 同参数重放：直接命中幂等缓存，不重复调用库存端口或重复发布事件
    let replayed = handler::handle(&state, cmd).await.unwrap();
    assert_eq!(replayed, view);
    assert_eq!(port.release_calls(), 1);
    assert_eq!(event_bus.snapshot().len(), 1);

    // 同幂等键 + 不同参数（version=2）：返回 409 idempotency_key_conflict
    let conflict = handler::handle(
        &state,
        CancelOrderCommand {
            buyer_id: BuyerId::try_new("buyer-demo").unwrap(),
            idempotency_key: IdempotencyKey::try_new("cancel-pending-1").unwrap(),
            order_id,
            expected_version: OrderVersion::try_new(2).unwrap(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(conflict.status, StatusCode::CONFLICT.as_u16());
    assert_eq!(conflict.code, "idempotency_key_conflict");
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn handler_cancels_paid_order_returns_stock_preserves_paid_at_and_writes_outbox(
    pool: PgPool,
) {
    let now = base_time();
    let paid_at = now + Duration::minutes(1);
    let clock = Arc::new(FixedClock::new(paid_at));
    let id_gen = Arc::new(SequenceIdGenerator::new(1));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let order_id = OrderId::new(Uuid::from_u128(0xABCD_0010));
    let reservation_id = ReservationId::new(Uuid::from_u128(0xABCD_0011));
    let paid_order = sample_paid_order(
        order_id,
        reservation_id,
        "buyer-demo",
        ItemKind::Physical,
        now,
        paid_at,
    );
    insert_order_row(&pool, &paid_order).await;

    let port = Arc::new(MockCancelPort::with_reservation(
        reservation_id,
        "confirmed",
    ));
    let state = AppState::new(
        pool.clone(),
        clock.clone(),
        id_gen,
        port.clone() as Arc<dyn InventoryPort>,
        event_bus.clone(),
        Duration::minutes(15),
    );

    clock.advance(Duration::minutes(4));
    let view = handler::handle(
        &state,
        CancelOrderCommand {
            buyer_id: BuyerId::try_new("buyer-demo").unwrap(),
            idempotency_key: IdempotencyKey::try_new("cancel-paid-1").unwrap(),
            order_id,
            expected_version: OrderVersion::try_new(2).unwrap(),
        },
    )
    .await
    .unwrap();

    assert_eq!(view.status, OrderLifecycleStatus::Cancelled);
    assert_eq!(view.version.get(), 3);
    assert_eq!(view.paid_at, Some(paid_at));
    assert_eq!(view.cancelled_at, Some(paid_at + Duration::minutes(4)));
    assert_eq!(
        view.fulfillment_hint.as_deref(),
        Some("已取消并完成模拟原路退款，库存已退回。")
    );
    assert_eq!(port.release_calls(), 0);
    assert_eq!(port.return_calls(), 1);

    let db_row: (
        String,
        i32,
        Option<chrono::DateTime<Utc>>,
        Option<chrono::DateTime<Utc>>,
    ) = sqlx::query_as(
        "SELECT status, version, paid_at, cancelled_at FROM ordering.flash_orders WHERE id = $1",
    )
    .bind(order_id.into_inner())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(db_row.0, "cancelled");
    assert_eq!(db_row.1, 3);
    assert_eq!(db_row.2, Some(paid_at));
    assert_eq!(db_row.3, Some(paid_at + Duration::minutes(4)));

    let published = event_bus.snapshot();
    assert_eq!(published.len(), 1);
    assert!(matches!(
        &published[0],
        IntegrationEvent::FlashOrderCancelled {
            order_id: oid,
            was_paid: true,
            ..
        } if *oid == order_id
    ));
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn handler_rejects_cancel_on_inventory_failure_without_mutating_db(pool: PgPool) {
    let now = base_time();
    let clock = Arc::new(FixedClock::new(now));
    let id_gen = Arc::new(SequenceIdGenerator::new(1));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let order_id = OrderId::new(Uuid::from_u128(0xABCD_0020));
    let reservation_id = ReservationId::new(Uuid::from_u128(0xABCD_0021));
    let order = sample_pending_order(
        order_id,
        reservation_id,
        "buyer-demo",
        ItemKind::Digital,
        now,
    );
    insert_order_row(&pool, &order).await;

    let port = Arc::new(MockCancelPort::with_error(InventoryPortError::Unavailable(
        "inventory timeout".to_owned(),
    )));
    let state = AppState::new(
        pool.clone(),
        clock,
        id_gen,
        port.clone() as Arc<dyn InventoryPort>,
        event_bus.clone(),
        Duration::minutes(15),
    );

    let err = handler::handle(
        &state,
        CancelOrderCommand {
            buyer_id: BuyerId::try_new("buyer-demo").unwrap(),
            idempotency_key: IdempotencyKey::try_new("cancel-fail-1").unwrap(),
            order_id,
            expected_version: OrderVersion::INITIAL,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE.as_u16());
    assert_eq!(err.code, "inventory_unavailable");
    assert_eq!(port.release_calls(), 1);

    let (status, version): (String, i32) =
        sqlx::query_as("SELECT status, version FROM ordering.flash_orders WHERE id = $1")
            .bind(order_id.into_inner())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "pending_payment");
    assert_eq!(version, 1);

    let idem_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ordering.idempotency_records")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(idem_count, 0);
    assert!(event_bus.snapshot().is_empty());
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn handler_enforces_occ_and_ownership_before_calling_inventory_port(pool: PgPool) {
    let now = base_time();
    let clock = Arc::new(FixedClock::new(now));
    let id_gen = Arc::new(SequenceIdGenerator::new(1));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let order_id = OrderId::new(Uuid::from_u128(0xABCD_0030));
    let reservation_id = ReservationId::new(Uuid::from_u128(0xABCD_0031));
    let order = sample_pending_order(
        order_id,
        reservation_id,
        "buyer-demo",
        ItemKind::Digital,
        now,
    );
    insert_order_row(&pool, &order).await;

    let port = Arc::new(MockCancelPort::with_reservation(reservation_id, "reserved"));
    let state = AppState::new(
        pool,
        clock,
        id_gen,
        port.clone() as Arc<dyn InventoryPort>,
        event_bus,
        Duration::minutes(15),
    );

    // 他人取消 -> 403 且不调用库存端口
    let err_owner = handler::handle(
        &state,
        CancelOrderCommand {
            buyer_id: BuyerId::try_new("buyer-other").unwrap(),
            idempotency_key: IdempotencyKey::try_new("cancel-owner").unwrap(),
            order_id,
            expected_version: OrderVersion::INITIAL,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err_owner.status, StatusCode::FORBIDDEN.as_u16());
    assert_eq!(err_owner.code, "order_ownership_mismatch");

    // 过期 version -> 409 且不调用库存端口
    let err_ver = handler::handle(
        &state,
        CancelOrderCommand {
            buyer_id: BuyerId::try_new("buyer-demo").unwrap(),
            idempotency_key: IdempotencyKey::try_new("cancel-stale").unwrap(),
            order_id,
            expected_version: OrderVersion::try_new(2).unwrap(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err_ver.status, StatusCode::CONFLICT.as_u16());
    assert_eq!(err_ver.code, "order_version_conflict");

    assert_eq!(port.release_calls(), 0);
    assert_eq!(port.return_calls(), 0);
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn handler_serializes_concurrent_occ_and_idempotency_races(pool: PgPool) {
    let now = base_time();
    let order_occ = OrderId::new(Uuid::from_u128(0xABCD_0040));
    let res_occ = ReservationId::new(Uuid::from_u128(0xABCD_0041));
    insert_order_row(
        &pool,
        &sample_pending_order(order_occ, res_occ, "occ-buyer", ItemKind::Digital, now),
    )
    .await;

    let order_idem = OrderId::new(Uuid::from_u128(0xABCD_0050));
    let res_idem = ReservationId::new(Uuid::from_u128(0xABCD_0051));
    insert_order_row(
        &pool,
        &sample_paid_order(
            order_idem,
            res_idem,
            "idem-buyer",
            ItemKind::Physical,
            now,
            now + Duration::minutes(1),
        ),
    )
    .await;

    let port = Arc::new(MockCancelPort::with_reservation(res_occ, "reserved"));
    port.inner
        .lock()
        .unwrap()
        .reservations
        .insert(res_idem, "confirmed");
    let event_bus = Arc::new(InMemoryEventBus::new());
    let clock = Arc::new(FixedClock::new(now + Duration::minutes(2)));
    let id_gen = Arc::new(SequenceIdGenerator::new(30));
    let state = AppState::new(
        pool.clone(),
        clock,
        id_gen,
        port.clone() as Arc<dyn InventoryPort>,
        event_bus,
        Duration::minutes(15),
    );

    // 1. 并发 4 个不同幂等键竞争同一待支付订单 version=1 -> 仅 1 个成功，其余 3 个 OCC 冲突
    let mut occ_handles = Vec::new();
    for idx in 0..4 {
        let state = state.clone();
        occ_handles.push(tokio::spawn(async move {
            handler::handle(
                &state,
                CancelOrderCommand {
                    buyer_id: BuyerId::try_new("occ-buyer").unwrap(),
                    idempotency_key: IdempotencyKey::try_new(format!("occ-cancel-{idx}")).unwrap(),
                    order_id: order_occ,
                    expected_version: OrderVersion::INITIAL,
                },
            )
            .await
        }));
    }

    let mut ok_count = 0;
    let mut conflict_count = 0;
    for h in occ_handles {
        match h.await.unwrap() {
            Ok(view) => {
                assert_eq!(view.status, OrderLifecycleStatus::Cancelled);
                assert_eq!(view.version.get(), 2);
                ok_count += 1;
            }
            Err(err) if err.code == "order_version_conflict" => {
                conflict_count += 1;
            }
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }
    assert_eq!(ok_count, 1);
    assert_eq!(conflict_count, 3);

    // 2. 并发 4 个相同幂等键竞争同一已支付订单 -> 全部返回一致响应且仅写入 1 条 Outbox
    let mut idem_handles = Vec::new();
    for _ in 0..4 {
        let state = state.clone();
        idem_handles.push(tokio::spawn(async move {
            handler::handle(
                &state,
                CancelOrderCommand {
                    buyer_id: BuyerId::try_new("idem-buyer").unwrap(),
                    idempotency_key: IdempotencyKey::try_new("same-cancel-key").unwrap(),
                    order_id: order_idem,
                    expected_version: OrderVersion::try_new(2).unwrap(),
                },
            )
            .await
        }));
    }

    let mut views = Vec::new();
    for h in idem_handles {
        views.push(h.await.unwrap().unwrap());
    }
    assert!(views.iter().all(|v| v == &views[0]));
    assert_eq!(views[0].status, OrderLifecycleStatus::Cancelled);
    assert_eq!(views[0].version.get(), 3);

    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ordering.outbox_events WHERE aggregate_id = $1 AND event_type = 'flash_order_cancelled'",
    )
    .bind(order_idem.into_inner())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(outbox_count, 1);
}
