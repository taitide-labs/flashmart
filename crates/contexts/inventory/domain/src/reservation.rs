use flashmart_primitives::{BuyerId, IdempotencyKey, ItemId, Quantity, ReservationId};

use crate::InventoryDomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReservationStatus {
    Reserved,
    Confirmed,
    Released,
    Returned,
}

impl ReservationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Confirmed => "confirmed",
            Self::Released => "released",
            Self::Returned => "returned",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockReservation {
    id: ReservationId,
    item_id: ItemId,
    buyer_id: BuyerId,
    quantity: Quantity,
    status: ReservationStatus,
    idempotency_key: IdempotencyKey,
}

impl StockReservation {
    pub fn new_reserved(
        id: ReservationId,
        item_id: ItemId,
        buyer_id: BuyerId,
        quantity: Quantity,
        idempotency_key: IdempotencyKey,
    ) -> Self {
        Self {
            id,
            item_id,
            buyer_id,
            quantity,
            status: ReservationStatus::Reserved,
            idempotency_key,
        }
    }

    pub fn rehydrate(
        id: ReservationId,
        item_id: ItemId,
        buyer_id: BuyerId,
        quantity: Quantity,
        status: ReservationStatus,
        idempotency_key: IdempotencyKey,
    ) -> Self {
        Self {
            id,
            item_id,
            buyer_id,
            quantity,
            status,
            idempotency_key,
        }
    }

    pub fn id(&self) -> ReservationId {
        self.id
    }

    pub fn item_id(&self) -> ItemId {
        self.item_id
    }

    pub fn buyer_id(&self) -> &BuyerId {
        &self.buyer_id
    }

    pub fn quantity(&self) -> Quantity {
        self.quantity
    }

    pub fn status(&self) -> ReservationStatus {
        self.status
    }

    pub fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }

    /// 幂等确认扣减：Reserved -> Confirmed；已 Confirmed 返回 idempotent_replay = true
    pub fn confirm(&mut self) -> Result<bool, InventoryDomainError> {
        match self.status {
            ReservationStatus::Reserved => {
                self.status = ReservationStatus::Confirmed;
                Ok(false)
            }
            ReservationStatus::Confirmed => Ok(true),
            other => Err(InventoryDomainError::InvalidReservationTransition {
                current: other,
                action: "confirm_deduction",
            }),
        }
    }

    /// 幂等释放预占：Reserved -> Released；已 Released 返回 idempotent_replay = true
    pub fn release(&mut self) -> Result<bool, InventoryDomainError> {
        match self.status {
            ReservationStatus::Reserved => {
                self.status = ReservationStatus::Released;
                Ok(false)
            }
            ReservationStatus::Released => Ok(true),
            other => Err(InventoryDomainError::InvalidReservationTransition {
                current: other,
                action: "release_reserved",
            }),
        }
    }

    /// 幂等退回已扣库存：Confirmed -> Returned；已 Returned 返回 idempotent_replay = true
    pub fn return_deducted(&mut self) -> Result<bool, InventoryDomainError> {
        match self.status {
            ReservationStatus::Confirmed => {
                self.status = ReservationStatus::Returned;
                Ok(false)
            }
            ReservationStatus::Returned => Ok(true),
            other => Err(InventoryDomainError::InvalidReservationTransition {
                current: other,
                action: "return_deducted",
            }),
        }
    }
}
