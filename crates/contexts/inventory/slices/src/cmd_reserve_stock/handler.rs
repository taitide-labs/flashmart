use chrono::{DateTime, Utc};
use flashmart_contracts::{Clock, IdGenerator, InventoryPortError, ReserveStockOutcome};
use flashmart_infra::EventBus;
use flashmart_inventory_domain::{
    FlashItem, InventoryLedger, ReservationStatus, SaleWindow, StockReservation,
};
use flashmart_primitives::{
    BuyerId, Currency, IdempotencyKey, ItemId, ItemKind, MoneyCents, Quantity, ReservationId,
};
use sqlx::PgPool;
use uuid::Uuid;

use super::contract::ReserveStockCommand;
use super::workflow::{ReserveStockDecision, decide_reserve_stock};

#[derive(Debug, sqlx::FromRow)]
struct FlashItemLockRow {
    id: Uuid,
    title: String,
    description: String,
    item_kind: String,
    price_cents: i64,
    currency: String,
    total_stock: i32,
    available_stock: i32,
    reserved_stock: i32,
    confirmed_stock: i32,
    returned_after_close_stock: i32,
    per_user_limit: i32,
    sale_starts_at: DateTime<Utc>,
    sale_ends_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct ReservationRow {
    id: Uuid,
    item_id: Uuid,
    buyer_id: String,
    quantity: i32,
    status: String,
    idempotency_key: String,
}

pub async fn reserve_stock(
    pool: &PgPool,
    clock: &dyn Clock,
    id_gen: &dyn IdGenerator,
    event_bus: &dyn EventBus,
    cmd: ReserveStockCommand,
) -> Result<ReserveStockOutcome, InventoryPortError> {
    let now = clock.now();
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;

    // 先对目标商品行加排他锁，再读取幂等记录，确保同 item_id + 同 idempotency_key 并发到达时串行可见已提交记录
    let item_row = sqlx::query_as::<_, FlashItemLockRow>(
        r#"
        SELECT
            id, title, description, item_kind, price_cents, currency,
            total_stock, available_stock, reserved_stock, confirmed_stock,
            returned_after_close_stock, per_user_limit, sale_starts_at, sale_ends_at
        FROM inventory.flash_items
        WHERE id = $1
        FOR UPDATE
        "#,
    )
    .bind(cmd.item_id.into_inner())
    .fetch_optional(&mut *tx)
    .await
    .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;

    let existing_row = sqlx::query_as::<_, ReservationRow>(
        r#"
        SELECT id, item_id, buyer_id, quantity, status, idempotency_key
        FROM inventory.stock_reservations
        WHERE idempotency_key = $1
        "#,
    )
    .bind(cmd.idempotency_key.as_str())
    .fetch_optional(&mut *tx)
    .await
    .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;

    let Some(item_row) = item_row else {
        if existing_row.is_some() {
            return Err(InventoryPortError::IdempotencyConflict);
        }
        return Err(InventoryPortError::ItemNotFound(cmd.item_id));
    };

    let mut item = rehydrate_flash_item(item_row)?;
    let existing = existing_row.map(rehydrate_reservation).transpose()?;

    match decide_reserve_stock(&cmd, &mut item, existing.as_ref(), now)? {
        ReserveStockDecision::IdempotentReplay { outcome } => {
            tx.commit()
                .await
                .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;
            Ok(outcome)
        }
        ReserveStockDecision::Reserved {
            reservation,
            event,
            outcome,
        } => {
            sqlx::query(
                r#"
                UPDATE inventory.flash_items
                SET available_stock = $2,
                    reserved_stock = $3,
                    updated_at = $4
                WHERE id = $1
                "#,
            )
            .bind(item.id().into_inner())
            .bind(item.ledger().available_stock())
            .bind(item.ledger().reserved_stock())
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;

            if let Err(err) = sqlx::query(
                r#"
                INSERT INTO inventory.stock_reservations (
                    id, item_id, buyer_id, quantity, status, idempotency_key, created_at, updated_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $7)
                "#,
            )
            .bind(reservation.id().into_inner())
            .bind(reservation.item_id().into_inner())
            .bind(reservation.buyer_id().as_str())
            .bind(reservation.quantity().get())
            .bind(reservation.status().as_str())
            .bind(reservation.idempotency_key().as_str())
            .bind(now)
            .execute(&mut *tx)
            .await
            {
                return Err(map_insert_reservation_error(err));
            }

            let payload = serde_json::to_value(&event)
                .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;
            sqlx::query(
                r#"
                INSERT INTO inventory.outbox_events (
                    id, event_type, aggregate_id, payload, occurred_at, published_at
                )
                VALUES ($1, $2, $3, $4, $5, NULL)
                "#,
            )
            .bind(id_gen.next_uuid())
            .bind(event.event_type())
            .bind(item.id().into_inner())
            .bind(payload)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;

            tx.commit()
                .await
                .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;

            event_bus.publish(event).await;
            Ok(outcome)
        }
    }
}

fn map_insert_reservation_error(err: sqlx::Error) -> InventoryPortError {
    if let sqlx::Error::Database(db_err) = &err
        && db_err.code().as_deref() == Some("23505")
        && db_err.constraint() == Some("stock_reservations_idempotency_key_key")
    {
        return InventoryPortError::IdempotencyConflict;
    }
    InventoryPortError::Unavailable(err.to_string())
}

fn rehydrate_flash_item(row: FlashItemLockRow) -> Result<FlashItem, InventoryPortError> {
    let item_kind = ItemKind::parse(&row.item_kind)
        .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;
    let price_cents = MoneyCents::try_new(row.price_cents)
        .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;
    let currency = Currency::parse(&row.currency)
        .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;
    let sale_window = SaleWindow::try_new(row.sale_starts_at, row.sale_ends_at)
        .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;
    let ledger = InventoryLedger::rehydrate(
        row.total_stock,
        row.available_stock,
        row.reserved_stock,
        row.confirmed_stock,
        row.returned_after_close_stock,
    )
    .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;

    FlashItem::try_new(
        ItemId::new(row.id),
        row.title,
        row.description,
        item_kind,
        price_cents,
        currency,
        row.per_user_limit,
        sale_window,
        ledger,
    )
    .map_err(|err| InventoryPortError::Unavailable(err.to_string()))
}

fn rehydrate_reservation(row: ReservationRow) -> Result<StockReservation, InventoryPortError> {
    let buyer_id = BuyerId::try_new(row.buyer_id)
        .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;
    let quantity = Quantity::try_new(row.quantity)
        .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;
    let status = parse_reservation_status(&row.status)?;
    let idempotency_key = IdempotencyKey::try_new(row.idempotency_key)
        .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;

    Ok(StockReservation::rehydrate(
        ReservationId::new(row.id),
        ItemId::new(row.item_id),
        buyer_id,
        quantity,
        status,
        idempotency_key,
    ))
}

fn parse_reservation_status(raw: &str) -> Result<ReservationStatus, InventoryPortError> {
    match raw {
        "reserved" => Ok(ReservationStatus::Reserved),
        "confirmed" => Ok(ReservationStatus::Confirmed),
        "released" => Ok(ReservationStatus::Released),
        "returned" => Ok(ReservationStatus::Returned),
        other => Err(InventoryPortError::Unavailable(format!(
            "未知的预占状态：{other}"
        ))),
    }
}
