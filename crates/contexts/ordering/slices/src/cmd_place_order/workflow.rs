use chrono::{DateTime, Duration, Utc};
use flashmart_contracts::{FlashItemSnapshot, IntegrationEvent, InventoryPortError, WindowStatus};
use flashmart_infra::{ApiProblem, ProblemDetails};
use flashmart_ordering_domain::{
    ActiveQuota, FlashOrder, OrderingDomainError, PaymentHoldDuration,
};
use flashmart_primitives::{BuyerId, OrderId, Quantity, ReservationId};
use http::StatusCode;

use super::contract::{FlashOrderView, PlaceOrderCommand};

pub const PLACE_ORDER_ENDPOINT: &str = "POST /api/orders";
pub const PLACE_ORDER_INSTANCE: &str = "/api/orders";

#[derive(Debug, Clone)]
pub struct PlaceOrderInput {
    pub order_id: OrderId,
    pub reservation_id: ReservationId,
    pub buyer_id: BuyerId,
    pub quantity: Quantity,
    pub snapshot: FlashItemSnapshot,
    pub current_active_qty: i32,
    pub payment_hold_duration: Duration,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct PreparedOrder {
    pub order: FlashOrder,
    pub event: IntegrationEvent,
    pub view: FlashOrderView,
}

/// 生成用于幂等载荷冲突检测的确定性请求指纹
pub fn compute_request_hash(cmd: &PlaceOrderCommand) -> String {
    format!("item_id={}&quantity={}", cmd.item_id, cmd.quantity.get())
}

/// 零副作用领域编排：核验限购占用并构造 `FlashOrder` 与集成事件
pub fn prepare_order(input: PlaceOrderInput) -> Result<PreparedOrder, OrderingDomainError> {
    let hold_duration = PaymentHoldDuration::try_new(input.payment_hold_duration)?;
    let quota = ActiveQuota::try_new(input.snapshot.per_user_limit, input.current_active_qty)?;
    quota.ensure_can_place(input.quantity)?;

    let (order, event) = FlashOrder::place(
        input.order_id,
        input.reservation_id,
        input.buyer_id,
        input.snapshot.id,
        input.snapshot.title,
        input.snapshot.item_kind,
        input.quantity,
        input.snapshot.price_cents,
        input.snapshot.currency,
        hold_duration,
        input.now,
    )?;

    let view = order_to_view(&order);
    Ok(PreparedOrder { order, event, view })
}

pub fn order_to_view(order: &FlashOrder) -> FlashOrderView {
    FlashOrderView {
        id: order.id(),
        buyer_id: order.buyer_id().clone(),
        item_id: order.item_id(),
        item_title: order.item_title().to_owned(),
        item_kind: order.item_kind(),
        quantity: order.quantity(),
        unit_price_cents: order.unit_price_cents(),
        total_amount_cents: order.total_amount_cents(),
        currency: order.currency(),
        status: order.state().lifecycle_status(),
        version: order.version(),
        payment_deadline: order.state().payment_deadline(),
        created_at: order.created_at(),
        paid_at: None,
        cancelled_at: None,
        expired_at: None,
        fulfillment_hint: Some(order.fulfillment_hint().to_owned()),
    }
}

pub fn idempotency_conflict_problem(instance: &str) -> ApiProblem {
    ProblemDetails::new(
        StatusCode::CONFLICT,
        "idempotency_key_conflict",
        "幂等键参数冲突",
        "相同的 Idempotency-Key 不能用于不同的请求参数。",
        instance,
    )
    .into()
}

pub fn map_inventory_error(err: InventoryPortError, instance: &str) -> ApiProblem {
    match err {
        InventoryPortError::ItemNotFound(item_id) => ProblemDetails::new(
            StatusCode::NOT_FOUND,
            "item_not_found",
            "商品不存在",
            format!("限量商品 {item_id} 不存在或已下架。"),
            instance,
        ),
        InventoryPortError::SaleWindowNotActive { status } => {
            let detail = match status {
                WindowStatus::Upcoming => "该商品尚未到开抢时间，请耐心等待倒计时结束。",
                WindowStatus::Ended => "本场限时抢购已结束，暂不可继续下单。",
                WindowStatus::Active | _ => "当前不在有效抢购时间窗内。",
            };
            ProblemDetails::new(
                StatusCode::CONFLICT,
                "sale_window_not_active",
                "不在抢购时间窗内",
                detail,
                instance,
            )
        }
        InventoryPortError::InsufficientStock => ProblemDetails::new(
            StatusCode::CONFLICT,
            "insufficient_stock",
            "剩余库存不足",
            "手慢一步，当前可售库存不足；如有买家取消或超时未付，库存将实时回流。",
            instance,
        ),
        InventoryPortError::IdempotencyConflict => ProblemDetails::new(
            StatusCode::CONFLICT,
            "idempotency_key_conflict",
            "幂等键参数冲突",
            "相同的 Idempotency-Key 不能用于不同的请求参数。",
            instance,
        ),
        InventoryPortError::ReservationNotFound(reservation_id) => ProblemDetails::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "inventory_unexpected_state",
            "库存预占状态异常",
            format!("未找到库存预占记录 {reservation_id}。"),
            instance,
        ),
        InventoryPortError::InvalidReservationState { current } => ProblemDetails::new(
            StatusCode::CONFLICT,
            "invalid_reservation_state",
            "库存预占状态不允许当前操作",
            format!("当前库存预占状态为 {current}。"),
            instance,
        ),
        InventoryPortError::NotImplemented(slice) => ProblemDetails::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "inventory_not_implemented",
            "库存端口尚未启用",
            format!("库存端口 {slice} 尚未启用。"),
            instance,
        ),
        InventoryPortError::Unavailable(detail) => ProblemDetails::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "inventory_unavailable",
            "库存服务暂不可用",
            detail,
            instance,
        ),
    }
    .into()
}

pub fn map_domain_error(err: OrderingDomainError, instance: &str) -> ApiProblem {
    match err {
        OrderingDomainError::PurchaseLimitExceeded { limit, active_qty } => ProblemDetails::new(
            StatusCode::CONFLICT,
            "purchase_limit_exceeded",
            "超出单人限购上限",
            format!(
                "每人限购 {limit} 件，您当前已持有 {active_qty} 件有效订单（取消或过期后可释放限购名额）。"
            ),
            instance,
        ),
        OrderingDomainError::AmountOverflow => ProblemDetails::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "amount_overflow",
            "订单金额溢出",
            "购买数量与单价乘积超出可表示范围。",
            instance,
        ),
        OrderingDomainError::InvalidPaymentHoldDuration
        | OrderingDomainError::InvalidPerUserLimit => ProblemDetails::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid_ordering_policy",
            "订单策略配置无效",
            err.to_string(),
            instance,
        ),
        OrderingDomainError::OwnershipMismatch
        | OrderingDomainError::VersionConflict { .. }
        | OrderingDomainError::OrderExpired
        | OrderingDomainError::InvalidOrderState { .. }
        | OrderingDomainError::NotDueForExpiration => ProblemDetails::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected_ordering_state",
            "订单领域状态异常",
            err.to_string(),
            instance,
        ),
    }
    .into()
}
