mod clock;
mod events;
mod inventory_port;

#[cfg(test)]
mod tests;

pub use clock::{Clock, IdGenerator};
pub use events::{IntegrationEvent, OrderLifecycleStatus, WindowStatus};
pub use inventory_port::{
    ConfirmStockDeductionRequest, ConfirmStockOutcome, FlashItemSnapshot, InventoryPort,
    InventoryPortError, ReleaseReservedStockRequest, ReleaseStockOutcome, ReserveStockOutcome,
    ReserveStockRequest, ReturnDeductedStockRequest, ReturnStockOutcome,
};
