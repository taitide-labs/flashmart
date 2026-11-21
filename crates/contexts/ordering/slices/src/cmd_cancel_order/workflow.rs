use chrono::{DateTime, Utc};
use flashmart_contracts::{IntegrationEvent, InventoryPortError, WindowStatus};
use flashmart_infra::{ApiProblem, ProblemDetails};
use flashmart_ordering_domain::{FlashOrder, OrderState, OrderingDomainError};
use flashmart_primitives::{BuyerId, OrderId, OrderVersion};
use http::StatusCode;

use super::contract::{CancelOrderCommand, CancelledOrderView};

pub fn cancel_order_endpoint(order_id: OrderId) -> String {
    format!("POST /api/orders/{order_id}/cancel")
}

pub fn cancel_order_instance(order_id: OrderId) -> String {
    format!("/api/orders/{order_id}/cancel")
}

/// 生成用于幂等载荷冲突检测的确定性请求指纹
pub fn compute_request_hash(cmd: &CancelOrderCommand) -> String {
    format!(
        "order_id={}&version={}",
        cmd.order_id,
        cmd.expected_version.get()
    )
}

#[derive(Debug, Clone)]
pub struct PreparedCancellation {
    pub order: FlashOrder,
    pub was_paid: bool,
    pub event: IntegrationEvent,
    pub view: CancelledOrderView,
}

/// 零副作用领域编排：校验权属、OCC 版本与可取消状态，并流转为已取消状态
pub fn prepare_cancellation(
    mut order: FlashOrder,
    caller: &BuyerId,
    expected_version: OrderVersion,
    now: DateTime<Utc>,
) -> Result<PreparedCancellation, OrderingDomainError> {
    let preserved_paid_at = match order.state() {
        OrderState::Paid { paid_at, .. } => Some(paid_at),
        _ => None,
    };
    let (was_paid, event) = order.cancel(caller, expected_version, now)?;
    let view = order_to_cancelled_view(&order, preserved_paid_at);
    Ok(PreparedCancellation {
        order,
        was_paid,
        event,
        view,
    })
}

pub fn order_to_cancelled_view(
    order: &FlashOrder,
    preserved_paid_at: Option<DateTime<Utc>>,
) -> CancelledOrderView {
    let (paid_at, cancelled_at, expired_at) = match order.state() {
        OrderState::PendingPayment { .. } => (None, None, None),
        OrderState::Paid { paid_at, .. } => (Some(paid_at), None, None),
        OrderState::Cancelled { cancelled_at, .. } => (preserved_paid_at, Some(cancelled_at), None),
        OrderState::Expired { expired_at, .. } => (None, None, Some(expired_at)),
    };

    CancelledOrderView {
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
        paid_at,
        cancelled_at,
        expired_at,
        fulfillment_hint: Some(order.fulfillment_hint().to_owned()),
    }
}

pub fn order_not_found_problem(order_id: OrderId, instance: &str) -> ApiProblem {
    ProblemDetails::new(
        StatusCode::NOT_FOUND,
        "order_not_found",
        "订单不存在",
        format!("未找到订单 {order_id}。"),
        instance,
    )
    .into()
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

pub fn map_domain_error(err: OrderingDomainError, instance: &str) -> ApiProblem {
    match err {
        OrderingDomainError::OwnershipMismatch => ProblemDetails::new(
            StatusCode::FORBIDDEN,
            "order_ownership_mismatch",
            "无权操作他人订单",
            "当前买家标识与订单归属不一致。",
            instance,
        ),
        OrderingDomainError::VersionConflict { expected, actual } => ProblemDetails::new(
            StatusCode::CONFLICT,
            "order_version_conflict",
            "订单状态已发生并发变更",
            format!("订单版本号冲突（期望 {expected}，实际 {actual}），请刷新订单最新状态后再试。"),
            instance,
        ),
        OrderingDomainError::InvalidOrderState { status, .. } => ProblemDetails::new(
            StatusCode::CONFLICT,
            "invalid_order_state",
            "订单状态不允许取消",
            format!(
                "当前状态为 {}，仅待支付或已支付订单可执行取消。",
                status.as_str()
            ),
            instance,
        ),
        OrderingDomainError::OrderExpired
        | OrderingDomainError::InvalidPaymentHoldDuration
        | OrderingDomainError::InvalidPerUserLimit
        | OrderingDomainError::AmountOverflow
        | OrderingDomainError::PurchaseLimitExceeded { .. }
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

pub fn map_inventory_error(err: InventoryPortError, instance: &str) -> ApiProblem {
    match err {
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
            format!("当前库存预占状态为 {current}，无法释放或退回库存。"),
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
        InventoryPortError::ItemNotFound(item_id) => ProblemDetails::new(
            StatusCode::NOT_FOUND,
            "item_not_found",
            "商品不存在",
            format!("限量商品 {item_id} 不存在或已下架。"),
            instance,
        ),
        InventoryPortError::SaleWindowNotActive { status } => {
            let detail = match status {
                WindowStatus::Upcoming => "该商品尚未到开抢时间。",
                WindowStatus::Ended => "本场限时抢购已结束。",
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
            "当前可售库存不足。",
            instance,
        ),
        InventoryPortError::IdempotencyConflict => {
            idempotency_conflict_problem(instance).0.as_ref().clone()
        }
    }
    .into()
}
