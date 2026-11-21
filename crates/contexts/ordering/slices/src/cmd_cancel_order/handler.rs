use chrono::{DateTime, Utc};
use flashmart_contracts::{ReleaseReservedStockRequest, ReturnDeductedStockRequest};
use flashmart_infra::{ApiProblem, AppState, ProblemDetails};
use flashmart_ordering_domain::{FlashOrder, OrderState};
use flashmart_primitives::{
    BuyerId, Currency, IdempotencyKey, ItemId, ItemKind, MoneyCents, OrderId, OrderVersion,
    Quantity, ReservationId,
};
use http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

use super::contract::{CancelOrderCommand, CancelledOrderView};
use super::workflow::{self, PreparedCancellation};

#[derive(Debug, sqlx::FromRow)]
struct IdempotencyRecordRow {
    request_hash: String,
    response_body: serde_json::Value,
}

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
    Cancelled(Box<PreparedCancellation>),
    IdempotencyRace,
}

pub async fn handle(
    state: &AppState,
    cmd: CancelOrderCommand,
) -> Result<CancelledOrderView, ApiProblem> {
    let endpoint = workflow::cancel_order_endpoint(cmd.order_id);
    let instance = workflow::cancel_order_instance(cmd.order_id);
    let request_hash = workflow::compute_request_hash(&cmd);

    // 1. 预检幂等记录，避免重放或参数冲突触发重复释放/退回库存
    if let Some(existing) = fetch_idempotency_record(
        &state.pool,
        &cmd.buyer_id,
        &endpoint,
        &cmd.idempotency_key,
        &instance,
    )
    .await?
    {
        return resolve_idempotency_record(existing, &request_hash, &instance);
    }

    let now = state.clock.now();

    // 2. 在开启写事务与调用上游端口前完成只读预检（存在性、权属、OCC 版本与可取消状态）
    let preflight_row = fetch_order_row(&state.pool, cmd.order_id, &instance)
        .await?
        .ok_or_else(|| workflow::order_not_found_problem(cmd.order_id, &instance))?;
    let preflight_order = rehydrate_order(preflight_row, &instance)?;
    let preflight_prepared =
        workflow::prepare_cancellation(preflight_order, &cmd.buyer_id, cmd.expected_version, now)
            .map_err(|err| workflow::map_domain_error(err, &instance))?;

    // 3. 严格在活跃数据库事务外调用同步库存端口：待支付释放预占，已支付退回已扣减库存
    let reservation_id = preflight_prepared.order.reservation_id();
    if preflight_prepared.was_paid {
        state
            .inventory_port
            .return_deducted_stock(ReturnDeductedStockRequest { reservation_id })
            .await
            .map_err(|err| workflow::map_inventory_error(err, &instance))?;
    } else {
        state
            .inventory_port
            .release_reserved_stock(ReleaseReservedStockRequest { reservation_id })
            .await
            .map_err(|err| workflow::map_inventory_error(err, &instance))?;
    }

    // 4. 开启本地短事务：行锁加锁二次校验状态与 OCC，原子落库订单变更、幂等记录与 Outbox
    match persist_cancellation_tx(state, &cmd, &endpoint, &request_hash, now, &instance).await? {
        PersistOutcome::Cancelled(prepared) => {
            state.event_bus.publish(prepared.event).await;
            Ok(prepared.view)
        }
        PersistOutcome::IdempotencyRace => {
            let existing = fetch_idempotency_record(
                &state.pool,
                &cmd.buyer_id,
                &endpoint,
                &cmd.idempotency_key,
                &instance,
            )
            .await?
            .ok_or_else(|| {
                internal_problem(
                    "ordering_idempotency_missing",
                    "幂等记录读取失败",
                    "并发幂等冲突后未能读取已提交记录。".to_owned(),
                    &instance,
                )
            })?;
            resolve_idempotency_record(existing, &request_hash, &instance)
        }
    }
}

async fn fetch_idempotency_record(
    pool: &PgPool,
    buyer_id: &BuyerId,
    endpoint: &str,
    idempotency_key: &IdempotencyKey,
    instance: &str,
) -> Result<Option<IdempotencyRecordRow>, ApiProblem> {
    sqlx::query_as::<_, IdempotencyRecordRow>(
        r#"
        SELECT request_hash, response_body
        FROM ordering.idempotency_records
        WHERE buyer_id = $1
          AND endpoint = $2
          AND idempotency_key = $3
        "#,
    )
    .bind(buyer_id.as_str())
    .bind(endpoint)
    .bind(idempotency_key.as_str())
    .fetch_optional(pool)
    .await
    .map_err(|err| {
        internal_problem(
            "ordering_query_failed",
            "读取幂等记录失败",
            format!("数据库查询异常：{err}"),
            instance,
        )
    })
}

fn resolve_idempotency_record(
    record: IdempotencyRecordRow,
    request_hash: &str,
    instance: &str,
) -> Result<CancelledOrderView, ApiProblem> {
    if record.request_hash != request_hash {
        return Err(workflow::idempotency_conflict_problem(instance));
    }
    serde_json::from_value(record.response_body).map_err(|err| {
        internal_problem(
            "ordering_corrupted_idempotency",
            "解析幂等缓存响应失败",
            err.to_string(),
            instance,
        )
    })
}

async fn fetch_order_row(
    pool: &PgPool,
    order_id: OrderId,
    instance: &str,
) -> Result<Option<FlashOrderRow>, ApiProblem> {
    sqlx::query_as::<_, FlashOrderRow>(
        r#"
        SELECT
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
        FROM ordering.flash_orders
        WHERE id = $1
        "#,
    )
    .bind(order_id.into_inner())
    .fetch_optional(pool)
    .await
    .map_err(|err| {
        internal_problem(
            "ordering_query_failed",
            "查询订单信息失败",
            format!("数据库查询异常：{err}"),
            instance,
        )
    })
}

async fn persist_cancellation_tx(
    state: &AppState,
    cmd: &CancelOrderCommand,
    endpoint: &str,
    request_hash: &str,
    now: DateTime<Utc>,
    instance: &str,
) -> Result<PersistOutcome, ApiProblem> {
    let mut tx = state.pool.begin().await.map_err(|err| {
        internal_problem(
            "ordering_tx_begin_failed",
            "开启取消事务失败",
            format!("数据库事务异常：{err}"),
            instance,
        )
    })?;

    // 对目标订单加排他行锁，串行化同一订单上的支付、取消与过期操作
    let locked_row = sqlx::query_as::<_, FlashOrderRow>(
        r#"
        SELECT
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
        FROM ordering.flash_orders
        WHERE id = $1
        FOR UPDATE
        "#,
    )
    .bind(cmd.order_id.into_inner())
    .fetch_optional(&mut *tx)
    .await
    .map_err(|err| {
        internal_problem(
            "ordering_query_failed",
            "锁定订单记录失败",
            format!("数据库查询异常：{err}"),
            instance,
        )
    })?;

    let existing_idem = sqlx::query_as::<_, IdempotencyRecordRow>(
        r#"
        SELECT request_hash, response_body
        FROM ordering.idempotency_records
        WHERE buyer_id = $1
          AND endpoint = $2
          AND idempotency_key = $3
        "#,
    )
    .bind(cmd.buyer_id.as_str())
    .bind(endpoint)
    .bind(cmd.idempotency_key.as_str())
    .fetch_optional(&mut *tx)
    .await
    .map_err(|err| {
        internal_problem(
            "ordering_query_failed",
            "校验幂等记录失败",
            format!("数据库查询异常：{err}"),
            instance,
        )
    })?;

    if existing_idem.is_some() {
        flashmart_infra::rollback_tx(tx, "cmd_cancel_order").await;
        return Ok(PersistOutcome::IdempotencyRace);
    }

    let Some(row) = locked_row else {
        flashmart_infra::rollback_tx(tx, "cmd_cancel_order").await;
        return Err(workflow::order_not_found_problem(cmd.order_id, instance));
    };

    let order = rehydrate_order(row, instance)?;
    let prepared =
        match workflow::prepare_cancellation(order, &cmd.buyer_id, cmd.expected_version, now) {
            Ok(prepared) => prepared,
            Err(domain_err) => {
                flashmart_infra::rollback_tx(tx, "cmd_cancel_order").await;
                return Err(workflow::map_domain_error(domain_err, instance));
            }
        };

    let response_json = serde_json::to_value(&prepared.view).map_err(|err| {
        internal_problem(
            "ordering_serialize_failed",
            "序列化取消响应失败",
            err.to_string(),
            instance,
        )
    })?;

    let idem_insert = sqlx::query(
        r#"
        INSERT INTO ordering.idempotency_records (
            buyer_id, endpoint, idempotency_key, request_hash, status_code, response_body, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (buyer_id, endpoint, idempotency_key) DO NOTHING
        "#,
    )
    .bind(cmd.buyer_id.as_str())
    .bind(endpoint)
    .bind(cmd.idempotency_key.as_str())
    .bind(request_hash)
    .bind(StatusCode::OK.as_u16() as i16)
    .bind(response_json)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|err| {
        internal_problem(
            "ordering_persist_failed",
            "写入幂等记录失败",
            format!("数据库写入异常：{err}"),
            instance,
        )
    })?;

    if idem_insert.rows_affected() == 0 {
        flashmart_infra::rollback_tx(tx, "cmd_cancel_order").await;
        return Ok(PersistOutcome::IdempotencyRace);
    }

    let updated = sqlx::query(
        r#"
        UPDATE ordering.flash_orders
        SET status = $2,
            version = $3,
            cancelled_at = $4,
            fulfillment_hint = $5
        WHERE id = $1
          AND version = $6
        "#,
    )
    .bind(prepared.order.id().into_inner())
    .bind(prepared.order.state().lifecycle_status().as_str())
    .bind(prepared.order.version().get())
    .bind(prepared.view.cancelled_at)
    .bind(Some(prepared.order.fulfillment_hint()))
    .bind(cmd.expected_version.get())
    .execute(&mut *tx)
    .await
    .map_err(|err| {
        internal_problem(
            "ordering_persist_failed",
            "更新订单取消状态失败",
            format!("数据库写入异常：{err}"),
            instance,
        )
    })?;

    if updated.rows_affected() != 1 {
        flashmart_infra::rollback_tx(tx, "cmd_cancel_order").await;
        return Err(ProblemDetails::new(
            StatusCode::CONFLICT,
            "order_version_conflict",
            "订单状态已发生并发变更",
            "订单版本号已过期，请刷新订单最新状态后再试。",
            instance,
        )
        .into());
    }

    let outbox_id = state.id_gen.next_uuid();
    let event_payload = serde_json::to_value(&prepared.event).map_err(|err| {
        internal_problem(
            "ordering_serialize_failed",
            "序列化 Outbox 事件失败",
            err.to_string(),
            instance,
        )
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
    .map_err(|err| {
        internal_problem(
            "ordering_persist_failed",
            "写入 Outbox 事件失败",
            format!("数据库写入异常：{err}"),
            instance,
        )
    })?;

    tx.commit().await.map_err(|err| {
        internal_problem(
            "ordering_tx_commit_failed",
            "提交取消事务失败",
            format!("数据库提交异常：{err}"),
            instance,
        )
    })?;

    Ok(PersistOutcome::Cancelled(Box::new(prepared)))
}

fn rehydrate_order(row: FlashOrderRow, instance: &str) -> Result<FlashOrder, ApiProblem> {
    parse_order_row(row).map_err(|err| {
        internal_problem("ordering_corrupted_row", "订单数据映射失败", err, instance)
    })
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

fn internal_problem(code: &str, title: &str, detail: String, instance: &str) -> ApiProblem {
    ProblemDetails::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        code,
        title,
        detail,
        instance,
    )
    .into()
}
