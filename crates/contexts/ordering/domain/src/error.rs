use flashmart_contracts::OrderLifecycleStatus;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OrderingDomainError {
    #[error("支付保留期必须大于 0 秒")]
    InvalidPaymentHoldDuration,
    #[error("单人限购上限必须大于等于 1")]
    InvalidPerUserLimit,
    #[error("金额计算溢出")]
    AmountOverflow,
    #[error("超出单人限购上限（上限 {limit} 件，当前已占用 {active_qty} 件）")]
    PurchaseLimitExceeded { limit: i32, active_qty: i32 },
    #[error("当前买家无权操作该订单")]
    OwnershipMismatch,
    #[error("订单版本号冲突（期望 {expected}，实际 {actual}）")]
    VersionConflict { expected: i32, actual: i32 },
    #[error("订单已超过支付保留期")]
    OrderExpired,
    #[error("订单当前状态 {status:?} 不允许执行 {action}")]
    InvalidOrderState {
        status: OrderLifecycleStatus,
        action: &'static str,
    },
    #[error("订单尚未超过支付保留期，暂不可过期")]
    NotDueForExpiration,
}
