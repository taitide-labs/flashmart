use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flashmart_contracts::{
    Clock, ConfirmStockDeductionRequest, ConfirmStockOutcome, FlashItemSnapshot, IdGenerator,
    IntegrationEvent, InventoryPort, InventoryPortError, ReleaseReservedStockRequest,
    ReleaseStockOutcome, ReserveStockOutcome, ReserveStockRequest, ReturnDeductedStockRequest,
    ReturnStockOutcome,
};
use flashmart_infra::EventBus;
use flashmart_inventory_domain::{
    FlashItem, InventoryLedger, ReservationStatus, SaleWindow, StockReservation,
};
use flashmart_primitives::{
    BuyerId, Currency, IdempotencyKey, ItemId, ItemKind, MoneyCents, Quantity, ReservationId,
};
use sqlx::PgPool;
use uuid::Uuid;

/// 基于 PostgreSQL 的库存同步端口适配器（阶段 4 仅冻结连接骨架，业务实现由工单交付）
#[derive(Clone)]
pub struct PgInventoryPort {
    pub pool: PgPool,
    pub clock: Arc<dyn Clock>,
    pub id_gen: Arc<dyn IdGenerator>,
    pub event_bus: Arc<dyn EventBus>,
}

impl PgInventoryPort {
    pub fn new(
        pool: PgPool,
        clock: Arc<dyn Clock>,
        id_gen: Arc<dyn IdGenerator>,
        event_bus: Arc<dyn EventBus>,
    ) -> Self {
        Self {
            pool,
            clock,
            id_gen,
            event_bus,
        }
    }
}

#[async_trait]
impl InventoryPort for PgInventoryPort {
    async fn list_flash_items(&self) -> Result<Vec<FlashItemSnapshot>, InventoryPortError> {
        crate::qry_list_flash_items::handler::list_snapshots(&self.pool, self.clock.now()).await
    }

    async fn get_flash_item(
        &self,
        item_id: ItemId,
    ) -> Result<Option<FlashItemSnapshot>, InventoryPortError> {
        crate::qry_get_flash_item::handler::get_snapshot(&self.pool, item_id, self.clock.now())
            .await
    }

    async fn reserve_stock(
        &self,
        req: ReserveStockRequest,
    ) -> Result<ReserveStockOutcome, InventoryPortError> {
        crate::cmd_reserve_stock::handler::reserve_stock(
            &self.pool,
            self.clock.as_ref(),
            self.id_gen.as_ref(),
            self.event_bus.as_ref(),
            req.into(),
        )
        .await
    }

    async fn confirm_stock_deduction(
        &self,
        req: ConfirmStockDeductionRequest,
    ) -> Result<ConfirmStockOutcome, InventoryPortError> {
        crate::cmd_confirm_stock_deduction::handler::confirm_stock_deduction(
            &self.pool,
            self.clock.as_ref(),
            self.id_gen.as_ref(),
            self.event_bus.as_ref(),
            req.into(),
        )
        .await
    }

    async fn release_reserved_stock(
        &self,
        req: ReleaseReservedStockRequest,
    ) -> Result<ReleaseStockOutcome, InventoryPortError> {
        crate::cmd_release_reserved_stock::handler::release_reserved_stock(
            &self.pool,
            self.clock.as_ref(),
            self.id_gen.as_ref(),
            self.event_bus.as_ref(),
            req.into(),
        )
        .await
    }

    async fn return_deducted_stock(
        &self,
        req: ReturnDeductedStockRequest,
    ) -> Result<ReturnStockOutcome, InventoryPortError> {
        crate::cmd_return_deducted_stock::handler::return_deducted_stock(
            &self.pool,
            self.clock.as_ref(),
            self.id_gen.as_ref(),
            self.event_bus.as_ref(),
            req.into(),
        )
        .await
    }
}

pub(crate) enum TransitionDecision<O> {
    Transitioned { event: IntegrationEvent, outcome: O },
    IdempotentReplay { outcome: O },
}

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

pub(crate) async fn execute_reservation_transition<O, F>(
    pool: &PgPool,
    clock: &dyn Clock,
    id_gen: &dyn IdGenerator,
    event_bus: &dyn EventBus,
    reservation_id: ReservationId,
    decide: F,
) -> Result<O, InventoryPortError>
where
    F: FnOnce(
        &mut FlashItem,
        &mut StockReservation,
        DateTime<Utc>,
    ) -> Result<TransitionDecision<O>, InventoryPortError>,
{
    let now = clock.now();
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;

    let reservation_row = sqlx::query_as::<_, ReservationRow>(
        r#"
        SELECT id, item_id, buyer_id, quantity, status, idempotency_key
        FROM inventory.stock_reservations
        WHERE id = $1
        FOR UPDATE
        "#,
    )
    .bind(reservation_id.into_inner())
    .fetch_optional(&mut *tx)
    .await
    .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;

    let Some(reservation_row) = reservation_row else {
        return Err(InventoryPortError::ReservationNotFound(reservation_id));
    };
    let mut reservation = rehydrate_reservation(reservation_row)?;

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
    .bind(reservation.item_id().into_inner())
    .fetch_optional(&mut *tx)
    .await
    .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;

    let Some(item_row) = item_row else {
        return Err(InventoryPortError::ItemNotFound(reservation.item_id()));
    };
    let mut item = rehydrate_flash_item(item_row)?;

    match decide(&mut item, &mut reservation, now)? {
        TransitionDecision::IdempotentReplay { outcome } => {
            tx.commit()
                .await
                .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;
            Ok(outcome)
        }
        TransitionDecision::Transitioned { event, outcome } => {
            sqlx::query(
                r#"
                UPDATE inventory.flash_items
                SET available_stock = $2,
                    reserved_stock = $3,
                    confirmed_stock = $4,
                    returned_after_close_stock = $5,
                    updated_at = $6
                WHERE id = $1
                "#,
            )
            .bind(item.id().into_inner())
            .bind(item.ledger().available_stock())
            .bind(item.ledger().reserved_stock())
            .bind(item.ledger().confirmed_stock())
            .bind(item.ledger().returned_after_close_stock())
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;

            sqlx::query(
                r#"
                UPDATE inventory.stock_reservations
                SET status = $2,
                    updated_at = $3
                WHERE id = $1
                "#,
            )
            .bind(reservation.id().into_inner())
            .bind(reservation.status().as_str())
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;

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
