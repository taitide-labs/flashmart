use chrono::{DateTime, Utc};
use flashmart_contracts::FlashItemSnapshot;
use flashmart_primitives::{Currency, ItemId, ItemKind, MoneyCents};

use crate::{InventoryDomainError, InventoryLedger, SaleWindow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashItem {
    id: ItemId,
    title: String,
    description: String,
    item_kind: ItemKind,
    price_cents: MoneyCents,
    currency: Currency,
    per_user_limit: i32,
    sale_window: SaleWindow,
    ledger: InventoryLedger,
}

impl FlashItem {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        id: ItemId,
        title: String,
        description: String,
        item_kind: ItemKind,
        price_cents: MoneyCents,
        currency: Currency,
        per_user_limit: i32,
        sale_window: SaleWindow,
        ledger: InventoryLedger,
    ) -> Result<Self, InventoryDomainError> {
        if per_user_limit < 1 {
            return Err(InventoryDomainError::InvalidPerUserLimit);
        }
        Ok(Self {
            id,
            title,
            description,
            item_kind,
            price_cents,
            currency,
            per_user_limit,
            sale_window,
            ledger,
        })
    }

    pub fn id(&self) -> ItemId {
        self.id
    }

    pub fn sale_window(&self) -> SaleWindow {
        self.sale_window
    }

    pub fn ledger(&self) -> InventoryLedger {
        self.ledger
    }

    pub fn ledger_mut(&mut self) -> &mut InventoryLedger {
        &mut self.ledger
    }

    pub fn to_snapshot(&self, now: DateTime<Utc>) -> FlashItemSnapshot {
        FlashItemSnapshot {
            id: self.id,
            title: self.title.clone(),
            description: self.description.clone(),
            item_kind: self.item_kind,
            price_cents: self.price_cents,
            currency: self.currency,
            total_stock: self.ledger.total_stock(),
            available_stock: self.ledger.available_stock(),
            per_user_limit: self.per_user_limit,
            sale_starts_at: self.sale_window.starts_at(),
            sale_ends_at: self.sale_window.ends_at(),
            window_status: self.sale_window.status_at(now),
        }
    }
}
