use chrono::{DateTime, Utc};
use flashmart_contracts::{FlashItemSnapshot, InventoryPortError, WindowStatus};
use flashmart_infra::{ApiProblem, AppState, FieldValidationError, ProblemDetails};
use flashmart_primitives::{Currency, ItemId, ItemKind, MoneyCents};
use http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

use super::contract::FlashItemDetailView;

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct FlashItemRow {
    pub(crate) id: Uuid,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) item_kind: String,
    pub(crate) price_cents: i64,
    pub(crate) currency: String,
    pub(crate) total_stock: i32,
    pub(crate) available_stock: i32,
    pub(crate) per_user_limit: i32,
    pub(crate) sale_starts_at: DateTime<Utc>,
    pub(crate) sale_ends_at: DateTime<Utc>,
}

pub(crate) fn parse_item_id(raw_item_id: &str, instance: &str) -> Result<ItemId, ApiProblem> {
    Uuid::parse_str(raw_item_id.trim())
        .map(ItemId::new)
        .map_err(|_| {
            ProblemDetails::validation(
                instance,
                "商品编号格式无效",
                "商品编号必须为合法 UUID。",
                vec![FieldValidationError {
                    field: "item_id".to_owned(),
                    message: "商品编号必须为合法 UUID".to_owned(),
                }],
            )
            .into()
        })
}

async fn fetch_row(pool: &PgPool, item_id: Uuid) -> Result<Option<FlashItemRow>, sqlx::Error> {
    sqlx::query_as::<_, FlashItemRow>(
        r#"
        SELECT
            id, title, description, item_kind, price_cents, currency,
            total_stock, available_stock, per_user_limit, sale_starts_at, sale_ends_at
        FROM inventory.flash_items
        WHERE id = $1
        "#,
    )
    .bind(item_id)
    .fetch_optional(pool)
    .await
}

pub(crate) fn row_to_snapshot(
    row: FlashItemRow,
    now: DateTime<Utc>,
) -> Result<FlashItemSnapshot, String> {
    let item_kind = ItemKind::parse(&row.item_kind).map_err(|e| e.to_string())?;
    let price_cents = MoneyCents::try_new(row.price_cents).map_err(|e| e.to_string())?;
    let currency = Currency::parse(&row.currency).map_err(|e| e.to_string())?;
    let window_status = WindowStatus::at(now, row.sale_starts_at, row.sale_ends_at);

    Ok(FlashItemSnapshot {
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
    })
}

pub(crate) fn snapshot_to_detail_view(
    snapshot: FlashItemSnapshot,
    server_time: DateTime<Utc>,
) -> FlashItemDetailView {
    FlashItemDetailView {
        id: snapshot.id,
        title: snapshot.title,
        description: snapshot.description,
        item_kind: snapshot.item_kind,
        price_cents: snapshot.price_cents,
        currency: snapshot.currency,
        total_stock: snapshot.total_stock,
        available_stock: snapshot.available_stock,
        per_user_limit: snapshot.per_user_limit,
        sale_starts_at: snapshot.sale_starts_at,
        sale_ends_at: snapshot.sale_ends_at,
        window_status: snapshot.window_status,
        server_time,
    }
}

pub async fn query_detail(
    state: &AppState,
    raw_item_id: &str,
) -> Result<FlashItemDetailView, ApiProblem> {
    let instance = format!("/api/flash-items/{raw_item_id}");
    let item_id = parse_item_id(raw_item_id, &instance)?;
    let now = state.clock.now();

    let row = fetch_row(&state.pool, item_id.into_inner())
        .await
        .map_err(|err| {
            ApiProblem::from(ProblemDetails::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "inventory_query_failed",
                "加载限量商品详情失败",
                format!("数据库查询异常：{err}"),
                &instance,
            ))
        })?
        .ok_or_else(|| {
            ApiProblem::from(ProblemDetails::new(
                StatusCode::NOT_FOUND,
                "item_not_found",
                "商品不存在",
                format!("未找到商品 {item_id}。"),
                &instance,
            ))
        })?;

    let snapshot = row_to_snapshot(row, now).map_err(|err| {
        ApiProblem::from(ProblemDetails::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "inventory_corrupted_row",
            "商品数据映射失败",
            err,
            &instance,
        ))
    })?;

    Ok(snapshot_to_detail_view(snapshot, now))
}

pub async fn get_snapshot(
    pool: &PgPool,
    item_id: ItemId,
    now: DateTime<Utc>,
) -> Result<Option<FlashItemSnapshot>, InventoryPortError> {
    let Some(row) = fetch_row(pool, item_id.into_inner())
        .await
        .map_err(|err| InventoryPortError::Unavailable(err.to_string()))?
    else {
        return Ok(None);
    };

    row_to_snapshot(row, now)
        .map(Some)
        .map_err(InventoryPortError::Unavailable)
}
