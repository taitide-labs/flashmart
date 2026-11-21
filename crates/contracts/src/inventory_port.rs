use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flashmart_primitives::{
    BuyerId, Currency, IdempotencyKey, ItemId, ItemKind, MoneyCents, Quantity, ReservationId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::WindowStatus;

/// 上游商品库存上下文对外暴露的防腐快照视图
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlashItemSnapshot {
    pub id: ItemId,
    pub title: String,
    pub description: String,
    pub item_kind: ItemKind,
    pub price_cents: MoneyCents,
    pub currency: Currency,
    pub total_stock: i32,
    pub available_stock: i32,
    pub per_user_limit: i32,
    pub sale_starts_at: DateTime<Utc>,
    pub sale_ends_at: DateTime<Utc>,
    pub window_status: WindowStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReserveStockRequest {
    pub reservation_id: ReservationId,
    pub item_id: ItemId,
    pub buyer_id: BuyerId,
    pub quantity: Quantity,
    pub idempotency_key: IdempotencyKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReserveStockOutcome {
    pub reservation_id: ReservationId,
    pub item_snapshot: FlashItemSnapshot,
    pub idempotent_replay: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmStockDeductionRequest {
    pub reservation_id: ReservationId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmStockOutcome {
    pub reservation_id: ReservationId,
    pub idempotent_replay: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseReservedStockRequest {
    pub reservation_id: ReservationId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseStockOutcome {
    pub reservation_id: ReservationId,
    pub restocked_to_available: bool,
    pub idempotent_replay: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnDeductedStockRequest {
    pub reservation_id: ReservationId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnStockOutcome {
    pub reservation_id: ReservationId,
    pub restocked_to_available: bool,
    pub idempotent_replay: bool,
}

/// 同步端口封闭错误枚举：调用方必须在切片边界穷尽映射全部变体
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum InventoryPortError {
    #[error("限量商品 {0} 不存在")]
    ItemNotFound(ItemId),
    #[error("库存预占记录 {0} 不存在")]
    ReservationNotFound(ReservationId),
    #[error("商品当前不在抢购时间窗内：{status:?}")]
    SaleWindowNotActive { status: WindowStatus },
    #[error("剩余可售库存不足")]
    InsufficientStock,
    #[error("库存预占状态不允许当前操作：{current}")]
    InvalidReservationState { current: String },
    #[error("预占幂等键与已有预占参数冲突")]
    IdempotencyConflict,
    #[error("库存端口尚未启用：{0}")]
    NotImplemented(&'static str),
    #[error("库存服务内部错误：{0}")]
    Unavailable(String),
}

/// 商品库存上下文 OHS/PL 同步端口；严禁在活跃数据库事务期间调用
#[async_trait]
pub trait InventoryPort: Send + Sync {
    async fn list_flash_items(&self) -> Result<Vec<FlashItemSnapshot>, InventoryPortError>;

    async fn get_flash_item(
        &self,
        item_id: ItemId,
    ) -> Result<Option<FlashItemSnapshot>, InventoryPortError>;

    async fn reserve_stock(
        &self,
        req: ReserveStockRequest,
    ) -> Result<ReserveStockOutcome, InventoryPortError>;

    async fn confirm_stock_deduction(
        &self,
        req: ConfirmStockDeductionRequest,
    ) -> Result<ConfirmStockOutcome, InventoryPortError>;

    async fn release_reserved_stock(
        &self,
        req: ReleaseReservedStockRequest,
    ) -> Result<ReleaseStockOutcome, InventoryPortError>;

    async fn return_deducted_stock(
        &self,
        req: ReturnDeductedStockRequest,
    ) -> Result<ReturnStockOutcome, InventoryPortError>;
}
