use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{MethodRouter, get};
use flashmart_infra::{
    ApiProblem, AppState, ProblemDetails, SliceDescriptor, SliceMethod, SliceStatus,
};

use super::contract::FlashItemListResponse;

#[utoipa::path(
    get,
    path = "/api/flash-items",
    tag = "inventory",
    responses(
        (status = 200, description = "返回限量商品目录与实时剩余库存", body = FlashItemListResponse),
        (status = 500, description = "服务端异常", body = ProblemDetails)
    )
)]
pub async fn handle(
    State(_state): State<AppState>,
) -> Result<axum::Json<FlashItemListResponse>, ApiProblem> {
    Err(ProblemDetails::new(
        StatusCode::NOT_IMPLEMENTED,
        "slice_not_implemented",
        "接口尚未激活",
        "qry_list_flash_items 切片尚处于 Planned 基线状态。",
        "/api/flash-items",
    )
    .into())
}

fn route() -> MethodRouter<AppState> {
    get(handle)
}

inventory::submit! {
    SliceDescriptor {
        id: "qry_list_flash_items",
        context: "inventory",
        method: SliceMethod::Get,
        path: "/api/flash-items",
        status: SliceStatus::Planned,
        route,
    }
}
