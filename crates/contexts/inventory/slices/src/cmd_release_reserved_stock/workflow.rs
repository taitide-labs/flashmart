use chrono::{DateTime, Utc};
use flashmart_contracts::{IntegrationEvent, InventoryPortError, ReleaseStockOutcome};
use flashmart_inventory_domain::{FlashItem, InventoryDomainError, StockReservation};

use super::contract::ReleaseReservedStockCommand;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseReservedStockDecision {
    Released {
        restocked_to_available: bool,
        event: IntegrationEvent,
        outcome: ReleaseStockOutcome,
    },
    IdempotentReplay {
        outcome: ReleaseStockOutcome,
    },
}

pub fn decide_release_reserved_stock(
    _cmd: &ReleaseReservedStockCommand,
    item: &mut FlashItem,
    reservation: &mut StockReservation,
    now: DateTime<Utc>,
) -> Result<ReleaseReservedStockDecision, InventoryPortError> {
    if reservation.release().map_err(map_domain_error)? {
        return Ok(ReleaseReservedStockDecision::IdempotentReplay {
            outcome: ReleaseStockOutcome {
                reservation_id: reservation.id(),
                restocked_to_available: false,
                idempotent_replay: true,
            },
        });
    }

    let window_status = item.sale_window().status_at(now);
    let restocked_to_available = item
        .ledger_mut()
        .release_reserved(reservation.quantity(), window_status)
        .map_err(map_domain_error)?;

    let event = IntegrationEvent::ReservedStockReleased {
        reservation_id: reservation.id(),
        item_id: item.id(),
        quantity: reservation.quantity(),
        restocked_to_available,
        occurred_at: now,
    };
    let outcome = ReleaseStockOutcome {
        reservation_id: reservation.id(),
        restocked_to_available,
        idempotent_replay: false,
    };

    Ok(ReleaseReservedStockDecision::Released {
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
