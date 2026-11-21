use flashmart_contracts::{ReleaseReservedStockRequest, ReleaseStockOutcome};
use flashmart_primitives::ReservationId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseReservedStockCommand {
    pub reservation_id: ReservationId,
}

impl From<ReleaseReservedStockRequest> for ReleaseReservedStockCommand {
    fn from(req: ReleaseReservedStockRequest) -> Self {
        Self {
            reservation_id: req.reservation_id,
        }
    }
}

pub type ReleaseReservedStockResult = ReleaseStockOutcome;
