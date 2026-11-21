use chrono::{DateTime, Utc};
use flashmart_contracts::{
    FlashItemSnapshot, InventoryPortError, ReleaseReservedStockRequest, ReleaseStockOutcome,
    ReserveStockRequest,
};
use flashmart_infra::{ApiProblem, AppState, ProblemDetails, rollback_tx};
use flashmart_primitives::{OrderId, ReservationId};
use http::StatusCode;
use sqlx::PgPool;

use super::contract::{FlashOrderView, PlaceOrderCommand};
use super::workflow::{
    self, PLACE_ORDER_ENDPOINT, PLACE_ORDER_INSTANCE, PlaceOrderInput, PreparedOrder,
};

#[derive(Debug, sqlx::FromRow)]
struct IdempotencyRecordRow {
    request_hash: String,
    status_code: i16,
    response_body: serde_json::Value,
}

enum PersistOutcome {
    Created(Box<PreparedOrder>),
    IdempotencyRace,
}

pub async fn handle(
    state: &AppState,
    cmd: PlaceOrderCommand,
) -> Result<(StatusCode, FlashOrderView), ApiProblem> {
    let request_hash = workflow::compute_request_hash(&cmd);

    // 1. 预检幂等记录，避免重放或参数冲突触发库存预占
    if let Some(existing) = fetch_idempotency_record(&state.pool, &cmd).await? {
        return resolve_idempotency_record(existing, &request_hash);
    }

    // 2. 严格在本地数据库事务外调用同步库存端口完成原子预占
    let tentative_reservation_id = ReservationId::new(state.id_gen.next_uuid());
    let reserve_outcome = state
        .inventory_port
        .reserve_stock(ReserveStockRequest {
            reservation_id: tentative_reservation_id,
            item_id: cmd.item_id,
            buyer_id: cmd.buyer_id.clone(),
            quantity: cmd.quantity,
            idempotency_key: cmd.idempotency_key.clone(),
        })
        .await
        .map_err(|err| workflow::map_inventory_error(err, PLACE_ORDER_INSTANCE))?;

    let reserved_id = reserve_outcome.reservation_id;

    // 3. 开启本地短事务：加锁核验有效限购占用并落库订单、幂等记录与 Outbox
    match persist_order_tx(
        state,
        &cmd,
        &request_hash,
        reserved_id,
        reserve_outcome.item_snapshot,
    )
    .await
    {
        Ok(PersistOutcome::Created(prepared)) => {
            state.event_bus.publish(prepared.event).await;
            Ok((StatusCode::CREATED, prepared.view))
        }
        Ok(PersistOutcome::IdempotencyRace) => {
            if !reserve_outcome.idempotent_replay
                && let Err(release_err) = compensate_release(state, reserved_id).await
            {
                tracing::error!(
                    reservation_id = %reserved_id,
                    buyer_id = cmd.buyer_id.as_str(),
                    item_id = %cmd.item_id,
                    error = %release_err,
                    "failed to release reserved stock after idempotency race"
                );
            }
            let existing = fetch_idempotency_record(&state.pool, &cmd)
                .await?
                .ok_or_else(|| {
                    internal_problem(
                        "ordering_idempotency_missing",
                        "幂等记录读取失败",
                        "并发幂等冲突后未能读取已提交记录。".to_owned(),
                    )
                })?;
            resolve_idempotency_record(existing, &request_hash)
        }
        Err(err) => {
            if let Err(release_err) = compensate_release(state, reserved_id).await {
                tracing::error!(
                    reservation_id = %reserved_id,
                    buyer_id = cmd.buyer_id.as_str(),
                    item_id = %cmd.item_id,
                    order_error_code = %err.code,
                    error = %release_err,
                    "failed to release reserved stock during order placement compensation"
                );
            }
            Err(err)
        }
    }
}

pub(super) async fn compensate_release(
    state: &AppState,
    reservation_id: ReservationId,
) -> Result<ReleaseStockOutcome, InventoryPortError> {
    state
        .inventory_port
        .release_reserved_stock(ReleaseReservedStockRequest { reservation_id })
        .await
}

async fn fetch_idempotency_record(
    pool: &PgPool,
    cmd: &PlaceOrderCommand,
) -> Result<Option<IdempotencyRecordRow>, ApiProblem> {
    sqlx::query_as::<_, IdempotencyRecordRow>(
        r#"
        SELECT request_hash, status_code, response_body
        FROM ordering.idempotency_records
        WHERE buyer_id = $1
          AND endpoint = $2
          AND idempotency_key = $3
        "#,
    )
    .bind(cmd.buyer_id.as_str())
    .bind(PLACE_ORDER_ENDPOINT)
    .bind(cmd.idempotency_key.as_str())
    .fetch_optional(pool)
    .await
    .map_err(|err| {
        internal_problem(
            "ordering_query_failed",
            "读取幂等记录失败",
            format!("数据库查询异常：{err}"),
        )
    })
}

fn resolve_idempotency_record(
    record: IdempotencyRecordRow,
    request_hash: &str,
) -> Result<(StatusCode, FlashOrderView), ApiProblem> {
    if record.request_hash != request_hash {
        return Err(workflow::idempotency_conflict_problem(PLACE_ORDER_INSTANCE));
    }

    let status = u16::try_from(record.status_code)
        .ok()
        .and_then(|code| StatusCode::from_u16(code).ok())
        .unwrap_or(StatusCode::CREATED);
    let view: FlashOrderView = serde_json::from_value(record.response_body).map_err(|err| {
        internal_problem(
            "ordering_corrupted_idempotency",
            "解析幂等缓存响应失败",
            err.to_string(),
        )
    })?;
    Ok((status, view))
}

async fn persist_order_tx(
    state: &AppState,
    cmd: &PlaceOrderCommand,
    request_hash: &str,
    reserved_id: ReservationId,
    snapshot: FlashItemSnapshot,
) -> Result<PersistOutcome, ApiProblem> {
    let mut tx = state.pool.begin().await.map_err(|err| {
        internal_problem(
            "ordering_tx_begin_failed",
            "开启订单事务失败",
            format!("数据库事务异常：{err}"),
        )
    })?;

    // 基于 (buyer_id, item_id) 获取事务级咨询锁，避免首单并发穿透限购检查
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1), hashtext($2))")
        .bind(cmd.buyer_id.as_str())
        .bind(cmd.item_id.as_uuid().to_string())
        .execute(&mut *tx)
        .await
        .map_err(|err| {
            internal_problem(
                "ordering_lock_failed",
                "获取限购锁失败",
                format!("数据库锁异常：{err}"),
            )
        })?;

    let existing_idem = sqlx::query_as::<_, IdempotencyRecordRow>(
        r#"
        SELECT request_hash, status_code, response_body
        FROM ordering.idempotency_records
        WHERE buyer_id = $1
          AND endpoint = $2
          AND idempotency_key = $3
        "#,
    )
    .bind(cmd.buyer_id.as_str())
    .bind(PLACE_ORDER_ENDPOINT)
    .bind(cmd.idempotency_key.as_str())
    .fetch_optional(&mut *tx)
    .await
    .map_err(|err| {
        internal_problem(
            "ordering_query_failed",
            "校验幂等记录失败",
            format!("数据库查询异常：{err}"),
        )
    })?;

    if existing_idem.is_some() {
        rollback_tx(tx, "cmd_place_order").await;
        return Ok(PersistOutcome::IdempotencyRace);
    }

    let active_qty_i64: i64 = sqlx::query_scalar(
        r#"
        SELECT COALESCE(SUM(quantity), 0)::BIGINT
        FROM ordering.flash_orders
        WHERE buyer_id = $1
          AND item_id = $2
          AND status IN ('pending_payment', 'paid')
        "#,
    )
    .bind(cmd.buyer_id.as_str())
    .bind(cmd.item_id.into_inner())
    .fetch_one(&mut *tx)
    .await
    .map_err(|err| {
        internal_problem(
            "ordering_query_failed",
            "统计有效限购占用失败",
            format!("数据库查询异常：{err}"),
        )
    })?;

    let current_active_qty = i32::try_from(active_qty_i64).unwrap_or(i32::MAX);
    let now = state.clock.now();
    let order_id = OrderId::new(state.id_gen.next_uuid());

    let prepared = match workflow::prepare_order(PlaceOrderInput {
        order_id,
        reservation_id: reserved_id,
        buyer_id: cmd.buyer_id.clone(),
        quantity: cmd.quantity,
        snapshot,
        current_active_qty,
        payment_hold_duration: state.payment_hold_duration,
        now,
    }) {
        Ok(prepared) => prepared,
        Err(domain_err) => {
            rollback_tx(tx, "cmd_place_order").await;
            return Err(workflow::map_domain_error(domain_err, PLACE_ORDER_INSTANCE));
        }
    };

    let response_json = serde_json::to_value(&prepared.view).map_err(|err| {
        internal_problem(
            "ordering_serialize_failed",
            "序列化订单响应失败",
            err.to_string(),
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
    .bind(PLACE_ORDER_ENDPOINT)
    .bind(cmd.idempotency_key.as_str())
    .bind(request_hash)
    .bind(StatusCode::CREATED.as_u16() as i16)
    .bind(response_json)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|err| {
        internal_problem(
            "ordering_persist_failed",
            "写入幂等记录失败",
            format!("数据库写入异常：{err}"),
        )
    })?;

    if idem_insert.rows_affected() == 0 {
        rollback_tx(tx, "cmd_place_order").await;
        return Ok(PersistOutcome::IdempotencyRace);
    }

    sqlx::query(
        r#"
        INSERT INTO ordering.flash_orders (
            id, reservation_id, buyer_id, item_id, item_title, item_kind,
            quantity, unit_price_cents, total_amount_cents, currency, status,
            version, payment_deadline, created_at, paid_at, cancelled_at,
            expired_at, fulfillment_hint
        )
        VALUES (
            $1, $2, $3, $4, $5, $6,
            $7, $8, $9, $10, $11,
            $12, $13, $14, $15, $16,
            $17, $18
        )
        "#,
    )
    .bind(prepared.order.id().into_inner())
    .bind(prepared.order.reservation_id().into_inner())
    .bind(prepared.order.buyer_id().as_str())
    .bind(prepared.order.item_id().into_inner())
    .bind(prepared.order.item_title())
    .bind(prepared.order.item_kind().as_str())
    .bind(prepared.order.quantity().get())
    .bind(prepared.order.unit_price_cents().get())
    .bind(prepared.order.total_amount_cents().get())
    .bind(prepared.order.currency().as_str())
    .bind(prepared.order.state().lifecycle_status().as_str())
    .bind(prepared.order.version().get())
    .bind(prepared.order.state().payment_deadline())
    .bind(prepared.order.created_at())
    .bind(Option::<DateTime<Utc>>::None)
    .bind(Option::<DateTime<Utc>>::None)
    .bind(Option::<DateTime<Utc>>::None)
    .bind(Some(prepared.order.fulfillment_hint()))
    .execute(&mut *tx)
    .await
    .map_err(|err| {
        internal_problem(
            "ordering_persist_failed",
            "落库抢购订单失败",
            format!("数据库写入异常：{err}"),
        )
    })?;

    let outbox_id = state.id_gen.next_uuid();
    let event_payload = serde_json::to_value(&prepared.event).map_err(|err| {
        internal_problem(
            "ordering_serialize_failed",
            "序列化 Outbox 事件失败",
            err.to_string(),
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
    .bind(prepared.order.created_at())
    .execute(&mut *tx)
    .await
    .map_err(|err| {
        internal_problem(
            "ordering_persist_failed",
            "写入 Outbox 事件失败",
            format!("数据库写入异常：{err}"),
        )
    })?;

    tx.commit().await.map_err(|err| {
        internal_problem(
            "ordering_tx_commit_failed",
            "提交订单事务失败",
            format!("数据库提交异常：{err}"),
        )
    })?;

    Ok(PersistOutcome::Created(Box::new(prepared)))
}

fn internal_problem(code: &str, title: &str, detail: String) -> ApiProblem {
    ProblemDetails::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        code,
        title,
        detail,
        PLACE_ORDER_INSTANCE,
    )
    .into()
}
