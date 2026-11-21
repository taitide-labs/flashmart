use chrono::{DateTime, Utc};
use flashmart_contracts::{IntegrationEvent, InventoryPortError, ReserveStockOutcome};
use flashmart_inventory_domain::{FlashItem, InventoryDomainError, StockReservation};

use super::contract::ReserveStockCommand;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReserveStockDecision {
    Reserved {
        reservation: StockReservation,
        event: IntegrationEvent,
        outcome: ReserveStockOutcome,
    },
    IdempotentReplay {
        outcome: ReserveStockOutcome,
    },
}

pub fn decide_reserve_stock(
    cmd: &ReserveStockCommand,
    item: &mut FlashItem,
    existing_reservation: Option<&StockReservation>,
    now: DateTime<Utc>,
) -> Result<ReserveStockDecision, InventoryPortError> {
    if let Some(existing) = existing_reservation {
        if existing.item_id() != cmd.item_id
            || existing.buyer_id() != &cmd.buyer_id
            || existing.quantity() != cmd.quantity
        {
            return Err(InventoryPortError::IdempotencyConflict);
        }

        return Ok(ReserveStockDecision::IdempotentReplay {
            outcome: ReserveStockOutcome {
                reservation_id: existing.id(),
                item_snapshot: item.to_snapshot(now),
                idempotent_replay: true,
            },
        });
    }

    let window_status = item.sale_window().status_at(now);
    item.ledger_mut()
        .reserve(cmd.quantity, window_status)
        .map_err(map_domain_error)?;

    let reservation = StockReservation::new_reserved(
        cmd.reservation_id,
        cmd.item_id,
        cmd.buyer_id.clone(),
        cmd.quantity,
        cmd.idempotency_key.clone(),
    );
    let event = IntegrationEvent::StockReserved {
        reservation_id: cmd.reservation_id,
        item_id: cmd.item_id,
        buyer_id: cmd.buyer_id.clone(),
        quantity: cmd.quantity,
        occurred_at: now,
    };
    let outcome = ReserveStockOutcome {
        reservation_id: cmd.reservation_id,
        item_snapshot: item.to_snapshot(now),
        idempotent_replay: false,
    };

    Ok(ReserveStockDecision::Reserved {
        reservation,
        event,
        outcome,
    })
}

fn map_domain_error(err: InventoryDomainError) -> InventoryPortError {
    match err {
        InventoryDomainError::SaleWindowNotActive { status } => {
            InventoryPortError::SaleWindowNotActive { status }
        }
        InventoryDomainError::InsufficientStock => InventoryPortError::InsufficientStock,
        other => InventoryPortError::Unavailable(other.to_string()),
    }
}
