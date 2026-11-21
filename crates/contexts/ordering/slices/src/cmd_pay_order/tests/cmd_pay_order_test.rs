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
use flashmart_ordering_domain::{FlashOrder, OrderState, OrderingDomainError, PaymentHoldDuration};
use flashmart_primitives::{
    BuyerId, Currency, IdempotencyKey, ItemId, ItemKind, MoneyCents, OrderId, OrderVersion,
    Quantity, ReservationId,
};
use http::StatusCode;
use rstest::rstest;
use sqlx::PgPool;
use uuid::Uuid;

use super::super::contract::{PayOrderCommand, PayOrderRequest};
use super::super::handler;
use super::super::workflow::{
    compute_request_hash, map_domain_error, map_inventory_error, order_not_found_problem,
    pay_order_instance, prepare_payment,
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

#[derive(Default)]
struct MockConfirmState {
    confirm_calls: usize,
    confirmed_ids: Vec<ReservationId>,
    forced_error: Option<InventoryPortError>,
    reservations: HashMap<ReservationId, bool>,
}

#[derive(Clone, Default)]
struct MockConfirmPort {
    inner: Arc<Mutex<MockConfirmState>>,
}

impl MockConfirmPort {
    fn with_reservation(reservation_id: ReservationId) -> Self {
        let mut state = MockConfirmState::default();
        state.reservations.insert(reservation_id, false);
        Self {
            inner: Arc::new(Mutex::new(state)),
        }
    }

    fn with_error(err: InventoryPortError) -> Self {
        let state = MockConfirmState {
            forced_error: Some(err),
            ..Default::default()
        };
        Self {
            inner: Arc::new(Mutex::new(state)),
        }
    }

    fn confirm_calls(&self) -> usize {
        self.inner.lock().unwrap().confirm_calls
    }
}

#[async_trait]
impl InventoryPort for MockConfirmPort {
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
        req: ConfirmStockDeductionRequest,
    ) -> Result<ConfirmStockOutcome, InventoryPortError> {
        let mut guard = self.inner.lock().unwrap();
        guard.confirm_calls += 1;
        if let Some(err) = guard.forced_error.clone() {
            return Err(err);
        }
        let confirmed = guard
            .reservations
            .get_mut(&req.reservation_id)
            .ok_or(InventoryPortError::ReservationNotFound(req.reservation_id))?;
        let replay = *confirmed;
        *confirmed = true;
        guard.confirmed_ids.push(req.reservation_id);
        Ok(ConfirmStockOutcome {
            reservation_id: req.reservation_id,
            idempotent_replay: replay,
        })
    }

    async fn release_reserved_stock(
        &self,
        req: ReleaseReservedStockRequest,
    ) -> Result<ReleaseStockOutcome, InventoryPortError> {
        Ok(ReleaseStockOutcome {
            reservation_id: req.reservation_id,
            restocked_to_available: false,
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
fn parse_pay_order_command_validates_inputs(
    #[case] raw_order_id: &str,
    #[case] version: i32,
    #[case] expected_ok: bool,
) {
    let buyer = BuyerId::try_new("buyer-demo").unwrap();
    let key = IdempotencyKey::try_new("pay-k1").unwrap();
    let res = PayOrderCommand::parse(
        buyer,
        key,
        raw_order_id,
        PayOrderRequest { version },
        "/api/orders/test/pay",
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

    let cmd_a = PayOrderCommand {
        buyer_id: buyer.clone(),
        idempotency_key: key.clone(),
        order_id: order1,
        expected_version: OrderVersion::try_new(1).unwrap(),
    };
    let cmd_a_repeat = cmd_a.clone();
    let cmd_diff_ver = PayOrderCommand {
        buyer_id: buyer.clone(),
        idempotency_key: key.clone(),
        order_id: order1,
        expected_version: OrderVersion::try_new(2).unwrap(),
    };
    let cmd_diff_order = PayOrderCommand {
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

#[rstest]
#[case(ItemKind::Digital, "FM-DIGI-")]
#[case(ItemKind::Physical, "FM-PKG-")]
fn prepare_payment_builds_fulfillment_hint_and_event(
    #[case] kind: ItemKind,
    #[case] expected_prefix: &str,
) {
    let now = base_time();
    let order_id = OrderId::new(Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_00AB_CDEF));
    let reservation_id = ReservationId::new(Uuid::from_u128(0x9999));
    let order = sample_pending_order(order_id, reservation_id, "buyer-demo", kind, now);

    // 恰好在支付保留期截止边界时刻仍允许结算
    let pay_time = now + Duration::minutes(15);
    let prepared = prepare_payment(
        order,
        &BuyerId::try_new("buyer-demo").unwrap(),
        OrderVersion::INITIAL,
        pay_time,
    )
    .unwrap();

    assert_eq!(prepared.view.status, OrderLifecycleStatus::Paid);
    assert_eq!(prepared.view.version.get(), 2);
    assert_eq!(prepared.view.paid_at, Some(pay_time));
    let hint = prepared.view.fulfillment_hint.as_deref().unwrap();
    assert!(
        hint.contains(expected_prefix),
        "expected {hint} to contain {expected_prefix}"
    );
    assert!(hint.contains("ABCDEF"));
    assert!(matches!(
        prepared.event,
        IntegrationEvent::FlashOrderPaid {
            order_id: oid,
            reservation_id: rid,
            ..
        } if oid == order_id && rid == reservation_id
    ));
}

#[test]
fn prepare_payment_enforces_ownership_occ_expiration_and_state() {
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
    let err_owner = prepare_payment(
        order.clone(),
        &BuyerId::try_new("buyer-other").unwrap(),
        OrderVersion::INITIAL,
        now + Duration::minutes(1),
    )
    .unwrap_err();
    assert_eq!(err_owner, OrderingDomainError::OwnershipMismatch);

    // 版本冲突
    let err_ver = prepare_payment(
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

    // 超过支付保留期
    let err_exp = prepare_payment(
        order.clone(),
        &BuyerId::try_new("buyer-demo").unwrap(),
        OrderVersion::INITIAL,
        now + Duration::minutes(15) + Duration::seconds(1),
    )
    .unwrap_err();
    assert_eq!(err_exp, OrderingDomainError::OrderExpired);

    // 已支付状态再次支付
    let mut paid_order = order;
    paid_order
        .pay(
            &BuyerId::try_new("buyer-demo").unwrap(),
            OrderVersion::INITIAL,
            now + Duration::minutes(2),
        )
        .unwrap();
    let err_state = prepare_payment(
        paid_order,
        &BuyerId::try_new("buyer-demo").unwrap(),
        OrderVersion::try_new(2).unwrap(),
        now + Duration::minutes(3),
    )
    .unwrap_err();
    assert_eq!(
        err_state,
        OrderingDomainError::InvalidOrderState {
            status: OrderLifecycleStatus::Paid,
            action: "pay"
        }
    );
}

#[test]
fn error_mappers_produce_expected_rfc9457_codes() {
    let order_id = OrderId::new(Uuid::from_u128(0x1001));
    let instance = pay_order_instance(order_id);

    let p_missing = order_not_found_problem(order_id, &instance);
    assert_eq!(p_missing.status, StatusCode::NOT_FOUND.as_u16());
    assert_eq!(p_missing.code, "order_not_found");

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

    let p_exp = map_domain_error(OrderingDomainError::OrderExpired, &instance);
    assert_eq!(p_exp.status, StatusCode::CONFLICT.as_u16());
    assert_eq!(p_exp.code, "order_expired");

    let p_state = map_domain_error(
        OrderingDomainError::InvalidOrderState {
            status: OrderLifecycleStatus::Paid,
            action: "pay",
        },
        &instance,
    );
    assert_eq!(p_state.status, StatusCode::CONFLICT.as_u16());
    assert_eq!(p_state.code, "invalid_order_state");

    let p_inv_state = map_inventory_error(
        InventoryPortError::InvalidReservationState {
            current: "released".to_owned(),
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
async fn handler_persists_paid_state_outbox_and_skips_port_on_rejections(pool: PgPool) {
    let now = base_time();
    let order_id = OrderId::new(Uuid::from_u128(0xEEEE_0001));
    let reservation_id = ReservationId::new(Uuid::from_u128(0xFFFF_0001));
    let order = sample_pending_order(
        order_id,
        reservation_id,
        "buyer-demo",
        ItemKind::Physical,
        now,
    );
    insert_order_row(&pool, &order).await;

    let port = Arc::new(MockConfirmPort::with_reservation(reservation_id));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let clock = Arc::new(FixedClock::new(now + Duration::minutes(2)));
    let id_gen = Arc::new(SequenceIdGenerator::new(1));
    let state = AppState::new(
        pool.clone(),
        clock.clone(),
        id_gen,
        port.clone() as Arc<dyn InventoryPort>,
        event_bus.clone(),
        Duration::minutes(15),
    );

    // 1. 他人操作与版本冲突在调用 InventoryPort 前直接拦截
    let err_owner = handler::handle(
        &state,
        PayOrderCommand {
            buyer_id: BuyerId::try_new("buyer-other").unwrap(),
            idempotency_key: IdempotencyKey::try_new("k-other").unwrap(),
            order_id,
            expected_version: OrderVersion::INITIAL,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err_owner.code, "order_ownership_mismatch");
    assert_eq!(port.confirm_calls(), 0);

    let err_ver = handler::handle(
        &state,
        PayOrderCommand {
            buyer_id: BuyerId::try_new("buyer-demo").unwrap(),
            idempotency_key: IdempotencyKey::try_new("k-ver").unwrap(),
            order_id,
            expected_version: OrderVersion::try_new(2).unwrap(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err_ver.code, "order_version_conflict");
    assert_eq!(port.confirm_calls(), 0);

    // 2. 合法支付：核销预占、更新为 paid/v2、写入 Outbox 并发布事件
    let view = handler::handle(
        &state,
        PayOrderCommand {
            buyer_id: BuyerId::try_new("buyer-demo").unwrap(),
            idempotency_key: IdempotencyKey::try_new("pay-k1").unwrap(),
            order_id,
            expected_version: OrderVersion::INITIAL,
        },
    )
    .await
    .unwrap();
    assert_eq!(view.status, OrderLifecycleStatus::Paid);
    assert_eq!(view.version.get(), 2);
    assert_eq!(view.paid_at, Some(clock.now()));
    assert!(
        view.fulfillment_hint
            .as_deref()
            .unwrap()
            .contains("FM-PKG-")
    );
    assert_eq!(port.confirm_calls(), 1);

    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ordering.outbox_events WHERE aggregate_id = $1 AND event_type = 'flash_order_paid' AND published_at IS NULL",
    )
    .bind(order_id.into_inner())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(outbox_count, 1);
    assert_eq!(event_bus.snapshot().len(), 1);

    // 3. 相同幂等键重放不重复调用 InventoryPort
    let replayed = handler::handle(
        &state,
        PayOrderCommand {
            buyer_id: BuyerId::try_new("buyer-demo").unwrap(),
            idempotency_key: IdempotencyKey::try_new("pay-k1").unwrap(),
            order_id,
            expected_version: OrderVersion::INITIAL,
        },
    )
    .await
    .unwrap();
    assert_eq!(replayed, view);
    assert_eq!(port.confirm_calls(), 1);

    // 4. 相同幂等键 + 不同版本号 -> 409 idempotency_key_conflict
    let err_idem = handler::handle(
        &state,
        PayOrderCommand {
            buyer_id: BuyerId::try_new("buyer-demo").unwrap(),
            idempotency_key: IdempotencyKey::try_new("pay-k1").unwrap(),
            order_id,
            expected_version: OrderVersion::try_new(2).unwrap(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err_idem.code, "idempotency_key_conflict");
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn handler_leaves_order_unchanged_when_inventory_confirm_fails(pool: PgPool) {
    let now = base_time();
    let order_id = OrderId::new(Uuid::from_u128(0xEEEE_0002));
    let reservation_id = ReservationId::new(Uuid::from_u128(0xFFFF_0002));
    let order = sample_pending_order(
        order_id,
        reservation_id,
        "buyer-demo",
        ItemKind::Digital,
        now,
    );
    insert_order_row(&pool, &order).await;

    let port = Arc::new(MockConfirmPort::with_error(
        InventoryPortError::Unavailable("inventory offline".to_owned()),
    ));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(now + Duration::minutes(1)));
    let id_gen = Arc::new(SequenceIdGenerator::new(10));
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
        PayOrderCommand {
            buyer_id: BuyerId::try_new("buyer-demo").unwrap(),
            idempotency_key: IdempotencyKey::try_new("pay-fail").unwrap(),
            order_id,
            expected_version: OrderVersion::INITIAL,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "inventory_unavailable");
    assert_eq!(port.confirm_calls(), 1);

    let (status, version): (String, i32) =
        sqlx::query_as("SELECT status, version FROM ordering.flash_orders WHERE id = $1")
            .bind(order_id.into_inner())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "pending_payment");
    assert_eq!(version, 1);
    assert_eq!(event_bus.snapshot().len(), 0);
}

#[sqlx::test(migrations = "../../../../migrations")]
async fn handler_serializes_concurrent_occ_and_idempotency_races(pool: PgPool) {
    let now = base_time();
    let order_occ = OrderId::new(Uuid::from_u128(0xEEEE_0003));
    let res_occ = ReservationId::new(Uuid::from_u128(0xFFFF_0003));
    insert_order_row(
        &pool,
        &sample_pending_order(order_occ, res_occ, "occ-buyer", ItemKind::Digital, now),
    )
    .await;

    let order_idem = OrderId::new(Uuid::from_u128(0xEEEE_0004));
    let res_idem = ReservationId::new(Uuid::from_u128(0xFFFF_0004));
    insert_order_row(
        &pool,
        &sample_pending_order(order_idem, res_idem, "idem-buyer", ItemKind::Physical, now),
    )
    .await;

    let port = Arc::new(MockConfirmPort::with_reservation(res_occ));
    port.inner
        .lock()
        .unwrap()
        .reservations
        .insert(res_idem, false);
    let event_bus = Arc::new(InMemoryEventBus::new());
    let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(now + Duration::minutes(1)));
    let id_gen = Arc::new(SequenceIdGenerator::new(20));
    let state = AppState::new(
        pool.clone(),
        clock,
        id_gen,
        port.clone() as Arc<dyn InventoryPort>,
        event_bus,
        Duration::minutes(15),
    );

    // 1. 并发 4 个不同幂等键竞争同一订单 version=1 -> 仅 1 个成功，其余 3 个 OCC 冲突
    let mut occ_handles = Vec::new();
    for idx in 0..4 {
        let state = state.clone();
        occ_handles.push(tokio::spawn(async move {
            handler::handle(
                &state,
                PayOrderCommand {
                    buyer_id: BuyerId::try_new("occ-buyer").unwrap(),
                    idempotency_key: IdempotencyKey::try_new(format!("occ-k-{idx}")).unwrap(),
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
                assert_eq!(view.status, OrderLifecycleStatus::Paid);
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

    // 2. 并发 4 个相同幂等键竞争同一订单 -> 全部返回一致的 PaidOrderView 且仅写入 1 条 Outbox
    let mut idem_handles = Vec::new();
    for _ in 0..4 {
        let state = state.clone();
        idem_handles.push(tokio::spawn(async move {
            handler::handle(
                &state,
                PayOrderCommand {
                    buyer_id: BuyerId::try_new("idem-buyer").unwrap(),
                    idempotency_key: IdempotencyKey::try_new("same-pay-key").unwrap(),
                    order_id: order_idem,
                    expected_version: OrderVersion::INITIAL,
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

    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ordering.outbox_events WHERE aggregate_id = $1 AND event_type = 'flash_order_paid'",
    )
    .bind(order_idem.into_inner())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(outbox_count, 1);
}
