use flashmart_contracts::WindowStatus;
use thiserror::Error;

use crate::ReservationStatus;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum InventoryDomainError {
    #[error("抢购时间窗结束时刻必须晚于开始时刻")]
    InvalidSaleWindowRange,
    #[error("单人限购上限必须大于等于 1")]
    InvalidPerUserLimit,
    #[error("库存账本配额不守恒或存在负值")]
    LedgerInvariantViolated,
    #[error("当前不在抢购时间窗内：{status:?}")]
    SaleWindowNotActive { status: WindowStatus },
    #[error("剩余可售库存不足")]
    InsufficientStock,
    #[error("库存预占状态 {current:?} 不允许执行 {action}")]
    InvalidReservationTransition {
        current: ReservationStatus,
        action: &'static str,
    },
}
