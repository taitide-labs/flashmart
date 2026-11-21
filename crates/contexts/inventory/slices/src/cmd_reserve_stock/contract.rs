use flashmart_contracts::{ReserveStockOutcome, ReserveStockRequest};
use flashmart_primitives::{BuyerId, IdempotencyKey, ItemId, Quantity, ReservationId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReserveStockCommand {
    pub reservation_id: ReservationId,
    pub item_id: ItemId,
    pub buyer_id: BuyerId,
    pub quantity: Quantity,
    pub idempotency_key: IdempotencyKey,
}

impl From<ReserveStockRequest> for ReserveStockCommand {
    fn from(req: ReserveStockRequest) -> Self {
        Self {
            reservation_id: req.reservation_id,
            item_id: req.item_id,
            buyer_id: req.buyer_id,
            quantity: req.quantity,
            idempotency_key: req.idempotency_key,
        }
    }
}

pub type ReserveStockResult = ReserveStockOutcome;
