use axum::extract::State;
use axum::routing::{MethodRouter, get};
use flashmart_infra::{
    ApiProblem, AppState, ProblemDetails, SliceDescriptor, SliceMethod, SliceStatus,
    extract_buyer_id,
};
use http::HeaderMap;

use super::contract::FlashOrderListResponse;
use super::handler;

#[utoipa::path(
    get,
    path = "/api/orders",
    tag = "ordering",
    params(
        ("X-Buyer-Id" = String, Header, description = "轻量买家标识")
    ),
    responses(
        (status = 200, description = "返回当前买家订单列表", body = FlashOrderListResponse),
        (status = 400, description = "缺少 X-Buyer-Id 请求头", body = ProblemDetails),
        (status = 422, description = "买家标识格式无效", body = ProblemDetails)
    )
)]
pub async fn handle(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<axum::Json<FlashOrderListResponse>, ApiProblem> {
    let buyer_id = extract_buyer_id(&headers, "/api/orders")?;
    let response = handler::query_list(&state, &buyer_id).await?;
    Ok(axum::Json(response))
}

fn route() -> MethodRouter<AppState> {
    get(handle)
}

inventory::submit! {
    SliceDescriptor {
        id: "qry_list_orders",
        context: "ordering",
        method: SliceMethod::Get,
        path: "/api/orders",
        status: SliceStatus::Active,
        route,
    }
}
