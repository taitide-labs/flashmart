use chrono::{DateTime, Utc};
use flashmart_contracts::{ConfirmStockOutcome, IntegrationEvent, InventoryPortError};
use flashmart_inventory_domain::{FlashItem, InventoryDomainError, StockReservation};

use super::contract::ConfirmStockDeductionCommand;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmStockDeductionDecision {
    Confirmed {
        event: IntegrationEvent,
        outcome: ConfirmStockOutcome,
    },
    IdempotentReplay {
        outcome: ConfirmStockOutcome,
    },
}

pub fn decide_confirm_stock_deduction(
    _cmd: &ConfirmStockDeductionCommand,
    item: &mut FlashItem,
    reservation: &mut StockReservation,
    now: DateTime<Utc>,
) -> Result<ConfirmStockDeductionDecision, InventoryPortError> {
    if reservation.confirm().map_err(map_domain_error)? {
        return Ok(ConfirmStockDeductionDecision::IdempotentReplay {
            outcome: ConfirmStockOutcome {
                reservation_id: reservation.id(),
                idempotent_replay: true,
            },
        });
    }

    item.ledger_mut()
        .confirm_deduction(reservation.quantity())
        .map_err(map_domain_error)?;

    let event = IntegrationEvent::StockDeductionConfirmed {
        reservation_id: reservation.id(),
        item_id: item.id(),
        quantity: reservation.quantity(),
        occurred_at: now,
    };
    let outcome = ConfirmStockOutcome {
        reservation_id: reservation.id(),
        idempotent_replay: false,
    };

    Ok(ConfirmStockDeductionDecision::Confirmed { event, outcome })
}

fn map_domain_error(err: InventoryDomainError) -> InventoryPortError {
    match err {
        InventoryDomainError::InvalidReservationTransition { current, .. } => {
            InventoryPortError::InvalidReservationState {
                current: current.as_str().to_owned(),
            }
        }
        other => InventoryPortError::Unavailable(other.to_string()),
    }
}
