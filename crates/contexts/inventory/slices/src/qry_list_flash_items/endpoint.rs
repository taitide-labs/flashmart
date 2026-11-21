use axum::extract::State;
use axum::routing::{MethodRouter, get};
use flashmart_infra::{
    ApiProblem, AppState, ProblemDetails, SliceDescriptor, SliceMethod, SliceStatus,
};

use super::contract::FlashItemListResponse;
use super::handler;

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
    State(state): State<AppState>,
) -> Result<axum::Json<FlashItemListResponse>, ApiProblem> {
    let response = handler::query_list(&state).await?;
    Ok(axum::Json(response))
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
        status: SliceStatus::Active,
        route,
    }
}
