use chrono::{DateTime, Utc};
use flashmart_primitives::{BuyerId, ItemId, MoneyCents, OrderId, Quantity, ReservationId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// 跨上下文开放枚举：限量商品抢购时间窗状态
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WindowStatus {
    Upcoming,
    Active,
    Ended,
}

impl WindowStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Upcoming => "upcoming",
            Self::Active => "active",
            Self::Ended => "ended",
        }
    }
}

/// 跨上下文开放枚举：订单生命周期状态
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OrderLifecycleStatus {
    PendingPayment,
    Paid,
    Cancelled,
    Expired,
}

impl OrderLifecycleStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PendingPayment => "pending_payment",
            Self::Paid => "paid",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        }
    }
}

/// 跨上下文集成事件（开放枚举，下游匹配必须保留兜底分支）
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum IntegrationEvent {
    StockReserved {
        reservation_id: ReservationId,
        item_id: ItemId,
        buyer_id: BuyerId,
        quantity: Quantity,
        occurred_at: DateTime<Utc>,
    },
    StockDeductionConfirmed {
        reservation_id: ReservationId,
        item_id: ItemId,
        quantity: Quantity,
        occurred_at: DateTime<Utc>,
    },
    ReservedStockReleased {
        reservation_id: ReservationId,
        item_id: ItemId,
        quantity: Quantity,
        restocked_to_available: bool,
        occurred_at: DateTime<Utc>,
    },
    DeductedStockReturned {
        reservation_id: ReservationId,
        item_id: ItemId,
        quantity: Quantity,
        restocked_to_available: bool,
        occurred_at: DateTime<Utc>,
    },
    FlashOrderPlaced {
        order_id: OrderId,
        reservation_id: ReservationId,
        buyer_id: BuyerId,
        item_id: ItemId,
        quantity: Quantity,
        total_amount_cents: MoneyCents,
        occurred_at: DateTime<Utc>,
    },
    FlashOrderPaid {
        order_id: OrderId,
        reservation_id: ReservationId,
        buyer_id: BuyerId,
        item_id: ItemId,
        quantity: Quantity,
        paid_at: DateTime<Utc>,
    },
    FlashOrderCancelled {
        order_id: OrderId,
        reservation_id: ReservationId,
        buyer_id: BuyerId,
        item_id: ItemId,
        quantity: Quantity,
        was_paid: bool,
        cancelled_at: DateTime<Utc>,
    },
    FlashOrderExpired {
        order_id: OrderId,
        reservation_id: ReservationId,
        buyer_id: BuyerId,
        item_id: ItemId,
        quantity: Quantity,
        expired_at: DateTime<Utc>,
    },
}

impl IntegrationEvent {
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::StockReserved { .. } => "stock_reserved",
            Self::StockDeductionConfirmed { .. } => "stock_deduction_confirmed",
            Self::ReservedStockReleased { .. } => "reserved_stock_released",
            Self::DeductedStockReturned { .. } => "deducted_stock_returned",
            Self::FlashOrderPlaced { .. } => "flash_order_placed",
            Self::FlashOrderPaid { .. } => "flash_order_paid",
            Self::FlashOrderCancelled { .. } => "flash_order_cancelled",
            Self::FlashOrderExpired { .. } => "flash_order_expired",
        }
    }
}
