use flashmart_contracts::{Clock, IdGenerator, InventoryPortError, ReturnStockOutcome};
use flashmart_infra::EventBus;
use sqlx::PgPool;

use super::contract::ReturnDeductedStockCommand;
use super::workflow::{ReturnDeductedStockDecision, decide_return_deducted_stock};
use crate::port::{TransitionDecision, execute_reservation_transition};

pub async fn return_deducted_stock(
    pool: &PgPool,
    clock: &dyn Clock,
    id_gen: &dyn IdGenerator,
    event_bus: &dyn EventBus,
    cmd: ReturnDeductedStockCommand,
) -> Result<ReturnStockOutcome, InventoryPortError> {
    execute_reservation_transition(
        pool,
        clock,
        id_gen,
        event_bus,
        cmd.reservation_id,
        |item, reservation, now| match decide_return_deducted_stock(&cmd, item, reservation, now)? {
            ReturnDeductedStockDecision::Returned { event, outcome, .. } => {
                Ok(TransitionDecision::Transitioned { event, outcome })
            }
            ReturnDeductedStockDecision::IdempotentReplay { outcome } => {
                Ok(TransitionDecision::IdempotentReplay { outcome })
            }
        },
    )
    .await
}
