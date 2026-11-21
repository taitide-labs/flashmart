use chrono::{DateTime, Utc};
use flashmart_contracts::{IdGenerator, InventoryPortError, ReleaseReservedStockRequest};
use flashmart_infra::AppState;
use flashmart_ordering_domain::{FlashOrder, OrderState, OrderingDomainError};
use flashmart_primitives::{
    BuyerId, Currency, ItemId, ItemKind, MoneyCents, OrderId, OrderVersion, Quantity, ReservationId,
};
use sqlx::PgPool;
use uuid::Uuid;

use super::contract::{
    ExpireUnpaidOrdersCommand, ExpireUnpaidOrdersError, ExpireUnpaidOrdersOutcome,
};
use super::workflow::{self, PreparedExpiration};

const ORDER_SELECT_COLUMNS: &str = r#"
    id,
    reservation_id,
    buyer_id,
    item_id,
    item_title,
    item_kind,
    quantity,
    unit_price_cents,
    total_amount_cents,
    currency,
    status,
    version,
    payment_deadline,
    created_at,
    paid_at,
    cancelled_at,
    expired_at,
    fulfillment_hint
"#;

#[derive(Debug, sqlx::FromRow)]
struct FlashOrderRow {
    id: Uuid,
    reservation_id: Uuid,
    buyer_id: String,
    item_id: Uuid,
    item_title: String,
    item_kind: String,
    quantity: i32,
    unit_price_cents: i64,
    total_amount_cents: i64,
    currency: String,
    status: String,
    version: i32,
    payment_deadline: DateTime<Utc>,
    created_at: DateTime<Utc>,
    paid_at: Option<DateTime<Utc>>,
    cancelled_at: Option<DateTime<Utc>>,
    expired_at: Option<DateTime<Utc>>,
    fulfillment_hint: Option<String>,
}

enum PersistOutcome {
    Expired(Box<PreparedExpiration>),
    Skipped,
}

pub async fn handle(
    state: &AppState,
    cmd: ExpireUnpaidOrdersCommand,
) -> Result<ExpireUnpaidOrdersOutcome, ExpireUnpaidOrdersError> {
    let Some(limit) = workflow::normalize_batch_limit(cmd.batch_limit) else {
        return Ok(ExpireUnpaidOrdersOutcome {
            expired_order_ids: Vec::new(),
        });
    };

    let now = state.clock.now();
    let candidate_rows = fetch_due_pending_orders(&state.pool, now, limit).await?;
    let mut expired_order_ids = Vec::with_capacity(candidate_rows.len());

    for row in candidate_rows {
        let candidate_order = rehydrate_order(row)?;
        let preflight = match workflow::prepare_expiration(candidate_order, now) {
            Ok(prepared) => prepared,
            Err(
                OrderingDomainError::NotDueForExpiration
                | OrderingDomainError::InvalidOrderState { .. },
            ) => continue,
            Err(err) => return Err(ExpireUnpaidOrdersError::Database(err.to_string())),
        };

        let order_id = preflight.order.id();
        let reservation_id = preflight.order.reservation_id();

        // 严格在活跃数据库事务外调用同步库存端口释放预占库存与限购名额
        match state
            .inventory_port
            .release_reserved_stock(ReleaseReservedStockRequest { reservation_id })
            .await
        {
            Ok(_) => {}
            Err(InventoryPortError::InvalidReservationState { current }) => {
                // 若扫描后订单已被并发支付或取消，跳过该笔并继续处理剩余批次
                if !is_order_still_pending(&state.pool, order_id).await? {
                    continue;
                }
                return Err(ExpireUnpaidOrdersError::Inventory(
                    InventoryPortError::InvalidReservationState { current }.to_string(),
                ));
            }
            Err(err) => {
                return Err(ExpireUnpaidOrdersError::Inventory(err.to_string()));
            }
        }

        match persist_expiration_tx(
            &state.pool,
            state.id_gen.as_ref(),
            order_id,
            preflight.expected_version,
            now,
        )
        .await?
        {
            PersistOutcome::Expired(prepared) => {
                state.event_bus.publish(prepared.event).await;
                expired_order_ids.push(prepared.order.id());
            }
            PersistOutcome::Skipped => {}
        }
    }

    Ok(ExpireUnpaidOrdersOutcome { expired_order_ids })
}

async fn fetch_due_pending_orders(
    pool: &PgPool,
    now: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<FlashOrderRow>, ExpireUnpaidOrdersError> {
    let sql = format!(
        r#"
        SELECT {ORDER_SELECT_COLUMNS}
        FROM ordering.flash_orders
        WHERE status = 'pending_payment'
          AND payment_deadline < $1
        ORDER BY payment_deadline ASC, id ASC
        LIMIT $2
        "#
    );

    sqlx::query_as::<_, FlashOrderRow>(&sql)
        .bind(now)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(|err| ExpireUnpaidOrdersError::Database(format!("扫描到期未付订单失败：{err}")))
}

async fn is_order_still_pending(
    pool: &PgPool,
    order_id: OrderId,
) -> Result<bool, ExpireUnpaidOrdersError> {
    let status: Option<String> = sqlx::query_scalar(
        r#"
        SELECT status
        FROM ordering.flash_orders
        WHERE id = $1
        "#,
    )
    .bind(order_id.into_inner())
    .fetch_optional(pool)
    .await
    .map_err(|err| ExpireUnpaidOrdersError::Database(format!("查询订单状态失败：{err}")))?;

    Ok(matches!(status.as_deref(), Some("pending_payment")))
}

async fn persist_expiration_tx(
    pool: &PgPool,
    id_gen: &dyn IdGenerator,
    order_id: OrderId,
    expected_version: OrderVersion,
    now: DateTime<Utc>,
) -> Result<PersistOutcome, ExpireUnpaidOrdersError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| ExpireUnpaidOrdersError::Database(format!("开启过期事务失败：{err}")))?;

    // 加排他行锁串行化同一订单上的支付、取消与过期操作
    let lock_sql = format!(
        r#"
        SELECT {ORDER_SELECT_COLUMNS}
        FROM ordering.flash_orders
        WHERE id = $1
        FOR UPDATE
        "#
    );
    let locked_row = sqlx::query_as::<_, FlashOrderRow>(&lock_sql)
        .bind(order_id.into_inner())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| ExpireUnpaidOrdersError::Database(format!("锁定订单记录失败：{err}")))?;

    let Some(row) = locked_row else {
        flashmart_infra::rollback_tx(tx, "cmd_expire_unpaid_order").await;
        return Ok(PersistOutcome::Skipped);
    };

    let order = rehydrate_order(row)?;
    if order.version() != expected_version {
        flashmart_infra::rollback_tx(tx, "cmd_expire_unpaid_order").await;
        return Ok(PersistOutcome::Skipped);
    }

    let prepared = match workflow::prepare_expiration(order, now) {
        Ok(prepared) => prepared,
        Err(
            OrderingDomainError::NotDueForExpiration
            | OrderingDomainError::InvalidOrderState { .. },
        ) => {
            flashmart_infra::rollback_tx(tx, "cmd_expire_unpaid_order").await;
            return Ok(PersistOutcome::Skipped);
        }
        Err(err) => {
            flashmart_infra::rollback_tx(tx, "cmd_expire_unpaid_order").await;
            return Err(ExpireUnpaidOrdersError::Database(err.to_string()));
        }
    };

    let updated = sqlx::query(
        r#"
        UPDATE ordering.flash_orders
        SET status = $2,
            version = $3,
            expired_at = $4,
            fulfillment_hint = $5
        WHERE id = $1
          AND version = $6
        "#,
    )
    .bind(prepared.order.id().into_inner())
    .bind(prepared.order.state().lifecycle_status().as_str())
    .bind(prepared.order.version().get())
    .bind(prepared.expired_at)
    .bind(Some(prepared.order.fulfillment_hint()))
    .bind(prepared.expected_version.get())
    .execute(&mut *tx)
    .await
    .map_err(|err| ExpireUnpaidOrdersError::Database(format!("更新订单过期状态失败：{err}")))?;

    if updated.rows_affected() != 1 {
        flashmart_infra::rollback_tx(tx, "cmd_expire_unpaid_order").await;
        return Ok(PersistOutcome::Skipped);
    }

    let outbox_id = id_gen.next_uuid();
    let event_payload = serde_json::to_value(&prepared.event).map_err(|err| {
        ExpireUnpaidOrdersError::Database(format!("序列化 Outbox 事件失败：{err}"))
    })?;

    sqlx::query(
        r#"
        INSERT INTO ordering.outbox_events (
            id, event_type, aggregate_id, payload, occurred_at, published_at
        )
        VALUES ($1, $2, $3, $4, $5, NULL)
        "#,
    )
    .bind(outbox_id)
    .bind(prepared.event.event_type())
    .bind(prepared.order.id().into_inner())
    .bind(event_payload)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|err| ExpireUnpaidOrdersError::Database(format!("写入 Outbox 事件失败：{err}")))?;

    tx.commit()
        .await
        .map_err(|err| ExpireUnpaidOrdersError::Database(format!("提交过期事务失败：{err}")))?;

    Ok(PersistOutcome::Expired(Box::new(prepared)))
}

fn rehydrate_order(row: FlashOrderRow) -> Result<FlashOrder, ExpireUnpaidOrdersError> {
    parse_order_row(row)
        .map_err(|err| ExpireUnpaidOrdersError::Database(format!("订单数据映射失败：{err}")))
}

fn parse_order_row(row: FlashOrderRow) -> Result<FlashOrder, String> {
    let buyer_id = BuyerId::try_new(row.buyer_id).map_err(|e| e.to_string())?;
    let item_kind = ItemKind::parse(&row.item_kind).map_err(|e| e.to_string())?;
    let quantity = Quantity::try_new(row.quantity).map_err(|e| e.to_string())?;
    let unit_price_cents = MoneyCents::try_new(row.unit_price_cents).map_err(|e| e.to_string())?;
    let total_amount_cents =
        MoneyCents::try_new(row.total_amount_cents).map_err(|e| e.to_string())?;
    let currency = Currency::parse(&row.currency).map_err(|e| e.to_string())?;
    let version = OrderVersion::try_new(row.version).map_err(|e| e.to_string())?;

    let state = match row.status.as_str() {
        "pending_payment" => OrderState::PendingPayment {
            payment_deadline: row.payment_deadline,
        },
        "paid" => OrderState::Paid {
            payment_deadline: row.payment_deadline,
            paid_at: row
                .paid_at
                .ok_or_else(|| "已支付订单缺少 paid_at 时间戳".to_owned())?,
        },
        "cancelled" => OrderState::Cancelled {
            payment_deadline: row.payment_deadline,
            cancelled_at: row
                .cancelled_at
                .ok_or_else(|| "已取消订单缺少 cancelled_at 时间戳".to_owned())?,
            was_paid: row.paid_at.is_some(),
        },
        "expired" => OrderState::Expired {
            payment_deadline: row.payment_deadline,
            expired_at: row
                .expired_at
                .ok_or_else(|| "已过期订单缺少 expired_at 时间戳".to_owned())?,
        },
        other => return Err(format!("未知订单状态：{other}")),
    };

    Ok(FlashOrder::rehydrate(
        OrderId::new(row.id),
        ReservationId::new(row.reservation_id),
        buyer_id,
        ItemId::new(row.item_id),
        row.item_title,
        item_kind,
        quantity,
        unit_price_cents,
        total_amount_cents,
        currency,
        state,
        version,
        row.created_at,
        row.fulfillment_hint.unwrap_or_default(),
    ))
}
