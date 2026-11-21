use flashmart_contracts::{ConfirmStockDeductionRequest, ConfirmStockOutcome};
use flashmart_primitives::ReservationId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmStockDeductionCommand {
    pub reservation_id: ReservationId,
}

impl From<ConfirmStockDeductionRequest> for ConfirmStockDeductionCommand {
    fn from(req: ConfirmStockDeductionRequest) -> Self {
        Self {
            reservation_id: req.reservation_id,
        }
    }
}

pub type ConfirmStockDeductionResult = ConfirmStockOutcome;
