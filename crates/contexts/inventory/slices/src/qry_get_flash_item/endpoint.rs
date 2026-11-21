use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{MethodRouter, get};
use flashmart_infra::{
    ApiProblem, AppState, ProblemDetails, SliceDescriptor, SliceMethod, SliceStatus,
};

use super::contract::FlashItemDetailView;

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
    State(_state): State<AppState>,
    Path(item_id): Path<String>,
) -> Result<axum::Json<FlashItemDetailView>, ApiProblem> {
    Err(ProblemDetails::new(
        StatusCode::NOT_IMPLEMENTED,
        "slice_not_implemented",
        "接口尚未激活",
        "qry_get_flash_item 切片尚处于 Planned 基线状态。",
        format!("/api/flash-items/{item_id}"),
    )
    .into())
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
        status: SliceStatus::Planned,
        route,
    }
}
