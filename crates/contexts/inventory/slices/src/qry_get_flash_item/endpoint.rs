use axum::extract::{Path, State};
use axum::routing::{MethodRouter, get};
use flashmart_infra::{
    ApiProblem, AppState, ProblemDetails, SliceDescriptor, SliceMethod, SliceStatus,
};

use super::contract::FlashItemDetailView;
use super::handler;

#[utoipa::path(
    get,
    path = "/api/flash-items/{item_id}",
    tag = "inventory",
    params(
        ("item_id" = String, Path, description = "限量商品 UUID")
    ),
    responses(
        (status = 200, description = "返回单个限量商品详情与实时库存", body = FlashItemDetailView),
        (status = 404, description = "商品不存在", body = ProblemDetails),
        (status = 422, description = "商品编号格式无效", body = ProblemDetails)
    )
)]
pub async fn handle(
    State(state): State<AppState>,
    Path(item_id): Path<String>,
) -> Result<axum::Json<FlashItemDetailView>, ApiProblem> {
    let view = handler::query_detail(&state, &item_id).await?;
    Ok(axum::Json(view))
}

fn route() -> MethodRouter<AppState> {
    get(handle)
}

inventory::submit! {
    SliceDescriptor {
        id: "qry_get_flash_item",
        context: "inventory",
        method: SliceMethod::Get,
        path: "/api/flash-items/{item_id}",
        status: SliceStatus::Active,
        route,
    }
}
