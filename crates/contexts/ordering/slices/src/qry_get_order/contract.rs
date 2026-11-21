use chrono::{DateTime, Utc};
use flashmart_contracts::OrderLifecycleStatus;
use flashmart_primitives::{
    BuyerId, Currency, ItemId, ItemKind, MoneyCents, OrderId, OrderVersion, Quantity,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FlashOrderDetailView {
    pub id: OrderId,
    pub buyer_id: BuyerId,
    pub item_id: ItemId,
    pub item_title: String,
    pub item_kind: ItemKind,
    pub quantity: Quantity,
    pub unit_price_cents: MoneyCents,
    pub total_amount_cents: MoneyCents,
    pub currency: Currency,
    pub status: OrderLifecycleStatus,
    pub version: OrderVersion,
    pub payment_deadline: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub paid_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub expired_at: Option<DateTime<Utc>>,
    pub fulfillment_hint: Option<String>,
}
