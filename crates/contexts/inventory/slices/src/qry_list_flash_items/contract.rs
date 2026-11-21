use chrono::{DateTime, Utc};
use flashmart_contracts::WindowStatus;
use flashmart_primitives::{Currency, ItemId, ItemKind, MoneyCents};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FlashItemSummary {
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
    pub server_time: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FlashItemListResponse {
    pub items: Vec<FlashItemSummary>,
    pub server_time: DateTime<Utc>,
}
