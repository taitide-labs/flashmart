use chrono::{DateTime, Utc};
use flashmart_contracts::{IntegrationEvent, InventoryPortError, ReturnStockOutcome};
use flashmart_inventory_domain::{FlashItem, InventoryDomainError, StockReservation};

use super::contract::ReturnDeductedStockCommand;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReturnDeductedStockDecision {
    Returned {
        restocked_to_available: bool,
        event: IntegrationEvent,
        outcome: ReturnStockOutcome,
    },
    IdempotentReplay {
        outcome: ReturnStockOutcome,
    },
}

pub fn decide_return_deducted_stock(
    _cmd: &ReturnDeductedStockCommand,
    item: &mut FlashItem,
    reservation: &mut StockReservation,
    now: DateTime<Utc>,
) -> Result<ReturnDeductedStockDecision, InventoryPortError> {
    if reservation.return_deducted().map_err(map_domain_error)? {
        return Ok(ReturnDeductedStockDecision::IdempotentReplay {
            outcome: ReturnStockOutcome {
                reservation_id: reservation.id(),
                restocked_to_available: false,
                idempotent_replay: true,
            },
        });
    }

    let window_status = item.sale_window().status_at(now);
    let restocked_to_available = item
        .ledger_mut()
        .return_deducted(reservation.quantity(), window_status)
        .map_err(map_domain_error)?;

    let event = IntegrationEvent::DeductedStockReturned {
        reservation_id: reservation.id(),
        item_id: item.id(),
        quantity: reservation.quantity(),
        restocked_to_available,
        occurred_at: now,
    };
    let outcome = ReturnStockOutcome {
        reservation_id: reservation.id(),
        restocked_to_available,
        idempotent_replay: false,
    };

    Ok(ReturnDeductedStockDecision::Returned {
        restocked_to_available,
        event,
        outcome,
    })
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
