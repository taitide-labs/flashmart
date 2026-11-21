use flashmart_contracts::{ReturnDeductedStockRequest, ReturnStockOutcome};
use flashmart_primitives::ReservationId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnDeductedStockCommand {
    pub reservation_id: ReservationId,
}

impl From<ReturnDeductedStockRequest> for ReturnDeductedStockCommand {
    fn from(req: ReturnDeductedStockRequest) -> Self {
        Self {
            reservation_id: req.reservation_id,
        }
    }
}

pub type ReturnDeductedStockResult = ReturnStockOutcome;
