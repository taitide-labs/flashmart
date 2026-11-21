use flashmart_contracts::{Clock, IdGenerator, InventoryPortError, ReleaseStockOutcome};
use flashmart_infra::EventBus;
use sqlx::PgPool;

use super::contract::ReleaseReservedStockCommand;
use super::workflow::{ReleaseReservedStockDecision, decide_release_reserved_stock};
use crate::port::{TransitionDecision, execute_reservation_transition};

pub async fn release_reserved_stock(
    pool: &PgPool,
    clock: &dyn Clock,
    id_gen: &dyn IdGenerator,
    event_bus: &dyn EventBus,
    cmd: ReleaseReservedStockCommand,
) -> Result<ReleaseStockOutcome, InventoryPortError> {
    execute_reservation_transition(
        pool,
        clock,
        id_gen,
        event_bus,
        cmd.reservation_id,
        |item, reservation, now| match decide_release_reserved_stock(&cmd, item, reservation, now)?
        {
            ReleaseReservedStockDecision::Released { event, outcome, .. } => {
                Ok(TransitionDecision::Transitioned { event, outcome })
            }
            ReleaseReservedStockDecision::IdempotentReplay { outcome } => {
                Ok(TransitionDecision::IdempotentReplay { outcome })
            }
        },
    )
    .await
}
