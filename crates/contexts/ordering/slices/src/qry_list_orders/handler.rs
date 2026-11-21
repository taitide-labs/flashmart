use chrono::{DateTime, Utc};
use flashmart_contracts::OrderLifecycleStatus;
use flashmart_infra::{ApiProblem, AppState, ProblemDetails};
use flashmart_primitives::{
    BuyerId, Currency, ItemId, ItemKind, MoneyCents, OrderId, OrderVersion, Quantity,
};
use http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

use super::contract::{FlashOrderListResponse, FlashOrderSummaryView};

#[derive(Debug, sqlx::FromRow)]
pub(super) struct FlashOrderRow {
    pub id: Uuid,
    pub buyer_id: String,
    pub item_id: Uuid,
    pub item_title: String,
    pub item_kind: String,
    pub quantity: i32,
    pub unit_price_cents: i64,
    pub total_amount_cents: i64,
    pub currency: String,
    pub status: String,
    pub version: i32,
    pub payment_deadline: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub paid_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub expired_at: Option<DateTime<Utc>>,
    pub fulfillment_hint: Option<String>,
}

pub(super) fn parse_order_status(raw: &str) -> Result<OrderLifecycleStatus, String> {
    match raw {
        "pending_payment" => Ok(OrderLifecycleStatus::PendingPayment),
        "paid" => Ok(OrderLifecycleStatus::Paid),
        "cancelled" => Ok(OrderLifecycleStatus::Cancelled),
        "expired" => Ok(OrderLifecycleStatus::Expired),
        other => Err(format!("未知订单状态：{other}")),
    }
}

pub(super) fn row_to_summary(row: FlashOrderRow) -> Result<FlashOrderSummaryView, String> {
    let buyer_id = BuyerId::try_new(row.buyer_id).map_err(|e| e.to_string())?;
    let item_kind = ItemKind::parse(&row.item_kind).map_err(|e| e.to_string())?;
    let quantity = Quantity::try_new(row.quantity).map_err(|e| e.to_string())?;
    let unit_price_cents = MoneyCents::try_new(row.unit_price_cents).map_err(|e| e.to_string())?;
    let total_amount_cents =
        MoneyCents::try_new(row.total_amount_cents).map_err(|e| e.to_string())?;
    let currency = Currency::parse(&row.currency).map_err(|e| e.to_string())?;
    let status = parse_order_status(&row.status)?;
    let version = OrderVersion::try_new(row.version).map_err(|e| e.to_string())?;

    Ok(FlashOrderSummaryView {
        id: OrderId::new(row.id),
        buyer_id,
        item_id: ItemId::new(row.item_id),
        item_title: row.item_title,
        item_kind,
        quantity,
        unit_price_cents,
        total_amount_cents,
        currency,
        status,
        version,
        payment_deadline: row.payment_deadline,
        created_at: row.created_at,
        paid_at: row.paid_at,
        cancelled_at: row.cancelled_at,
        expired_at: row.expired_at,
        fulfillment_hint: row.fulfillment_hint,
    })
}

async fn fetch_rows(pool: &PgPool, buyer_id: &str) -> Result<Vec<FlashOrderRow>, sqlx::Error> {
    sqlx::query_as::<_, FlashOrderRow>(
        r#"
        SELECT
            id,
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
        WHERE buyer_id = $1
        ORDER BY created_at DESC, id DESC
        "#,
    )
    .bind(buyer_id)
    .fetch_all(pool)
    .await
}

/// 执行买家个人订单列表的 CQRS 只读视图投影
pub async fn query_list(
    state: &AppState,
    buyer_id: &BuyerId,
) -> Result<FlashOrderListResponse, ApiProblem> {
    let now = state.clock.now();
    let rows = fetch_rows(&state.pool, buyer_id.as_str())
        .await
        .map_err(|err| {
            ApiProblem::from(ProblemDetails::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ordering_query_failed",
                "加载个人订单列表失败",
                format!("数据库查询异常：{err}"),
                "/api/orders",
            ))
        })?;

    let mut orders = Vec::with_capacity(rows.len());
    for row in rows {
        let summary = row_to_summary(row).map_err(|err| {
            ApiProblem::from(ProblemDetails::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ordering_corrupted_row",
                "订单数据映射失败",
                err,
                "/api/orders",
            ))
        })?;
        orders.push(summary);
    }

    Ok(FlashOrderListResponse {
        orders,
        server_time: now,
    })
}
