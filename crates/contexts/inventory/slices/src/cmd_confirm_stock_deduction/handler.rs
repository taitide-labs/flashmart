use flashmart_contracts::{Clock, ConfirmStockOutcome, IdGenerator, InventoryPortError};
use flashmart_infra::EventBus;
use sqlx::PgPool;

use super::contract::ConfirmStockDeductionCommand;
use super::workflow::{ConfirmStockDeductionDecision, decide_confirm_stock_deduction};
use crate::port::{TransitionDecision, execute_reservation_transition};

pub async fn confirm_stock_deduction(
    pool: &PgPool,
    clock: &dyn Clock,
    id_gen: &dyn IdGenerator,
    event_bus: &dyn EventBus,
    cmd: ConfirmStockDeductionCommand,
) -> Result<ConfirmStockOutcome, InventoryPortError> {
    execute_reservation_transition(
        pool,
        clock,
        id_gen,
        event_bus,
        cmd.reservation_id,
        |item, reservation, now| match decide_confirm_stock_deduction(&cmd, item, reservation, now)?
        {
            ConfirmStockDeductionDecision::Confirmed { event, outcome } => {
                Ok(TransitionDecision::Transitioned { event, outcome })
            }
            ConfirmStockDeductionDecision::IdempotentReplay { outcome } => {
                Ok(TransitionDecision::IdempotentReplay { outcome })
            }
        },
    )
    .await
}
