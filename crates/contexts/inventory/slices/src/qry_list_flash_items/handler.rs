use chrono::{DateTime, Utc};
use flashmart_contracts::{FlashItemSnapshot, InventoryPortError, WindowStatus};
use flashmart_infra::{ApiProblem, AppState, ProblemDetails};
use flashmart_primitives::{Currency, ItemId, ItemKind, MoneyCents};
use http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

use super::contract::{FlashItemListResponse, FlashItemSummary};

#[derive(Debug, sqlx::FromRow)]
pub(super) struct FlashItemRow {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub item_kind: String,
    pub price_cents: i64,
    pub currency: String,
    pub total_stock: i32,
    pub available_stock: i32,
    pub per_user_limit: i32,
    pub sale_starts_at: DateTime<Utc>,
    pub sale_ends_at: DateTime<Utc>,
}

async fn fetch_rows(pool: &PgPool) -> Result<Vec<FlashItemRow>, sqlx::Error> {
    sqlx::query_as::<_, FlashItemRow>(
        r#"
        SELECT
            id, title, description, item_kind, price_cents, currency,
            total_stock, available_stock, per_user_limit, sale_starts_at, sale_ends_at
        FROM inventory.flash_items
        ORDER BY sale_starts_at ASC, id ASC
        "#,
    )
    .fetch_all(pool)
    .await
}

pub(super) fn row_to_summary(
    row: FlashItemRow,
    now: DateTime<Utc>,
) -> Result<FlashItemSummary, String> {
    let item_kind = ItemKind::parse(&row.item_kind).map_err(|e| e.to_string())?;
    let price_cents = MoneyCents::try_new(row.price_cents).map_err(|e| e.to_string())?;
    let currency = Currency::parse(&row.currency).map_err(|e| e.to_string())?;
    let window_status = WindowStatus::at(now, row.sale_starts_at, row.sale_ends_at);

    Ok(FlashItemSummary {
        id: ItemId::new(row.id),
        title: row.title,
        description: row.description,
        item_kind,
        price_cents,
        currency,
        total_stock: row.total_stock,
        available_stock: row.available_stock,
        per_user_limit: row.per_user_limit,
        sale_starts_at: row.sale_starts_at,
        sale_ends_at: row.sale_ends_at,
        window_status,
        server_time: now,
    })
}

pub async fn query_list(state: &AppState) -> Result<FlashItemListResponse, ApiProblem> {
    let now = state.clock.now();
    let rows = fetch_rows(&state.pool).await.map_err(|err| {
        ApiProblem::from(ProblemDetails::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "inventory_query_failed",
            "加载限量商品列表失败",
            format!("数据库查询异常：{err}"),
            "/api/flash-items",
        ))
    })?;

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let summary = row_to_summary(row, now).map_err(|err| {
            ApiProblem::from(ProblemDetails::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "inventory_corrupted_row",
                "商品数据映射失败",
                err,
                "/api/flash-items",
            ))
        })?;
        items.push(summary);
    }

    Ok(FlashItemListResponse {
        items,
        server_time: now,
    })
}

pub async fn list_snapshots(
    pool: &PgPool,
    now: DateTime<Utc>,
) -> Result<Vec<FlashItemSnapshot>, InventoryPortError> {
    let rows = fetch_rows(pool)
        .await
        .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?;

    let mut snapshots = Vec::with_capacity(rows.len());
    for row in rows {
        let summary = row_to_summary(row, now).map_err(InventoryPortError::Unavailable)?;
        snapshots.push(FlashItemSnapshot {
            id: summary.id,
            title: summary.title,
            description: summary.description,
            item_kind: summary.item_kind,
            price_cents: summary.price_cents,
            currency: summary.currency,
            total_stock: summary.total_stock,
            available_stock: summary.available_stock,
            per_user_limit: summary.per_user_limit,
            sale_starts_at: summary.sale_starts_at,
            sale_ends_at: summary.sale_ends_at,
            window_status: summary.window_status,
        });
    }
    Ok(snapshots)
}
