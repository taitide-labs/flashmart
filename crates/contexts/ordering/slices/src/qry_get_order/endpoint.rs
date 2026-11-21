use axum::extract::{Path, State};
use axum::routing::{MethodRouter, get};
use flashmart_infra::{
    ApiProblem, AppState, ProblemDetails, SliceDescriptor, SliceMethod, SliceStatus,
    extract_buyer_id,
};
use http::HeaderMap;

use super::contract::FlashOrderDetailView;
use super::handler;

#[utoipa::path(
    get,
    path = "/api/orders/{order_id}",
    tag = "ordering",
    params(
        ("order_id" = String, Path, description = "抢购订单 UUID"),
        ("X-Buyer-Id" = String, Header, description = "轻量买家标识")
    ),
    responses(
        (status = 200, description = "返回订单详情", body = FlashOrderDetailView),
        (status = 400, description = "缺少 X-Buyer-Id 请求头", body = ProblemDetails),
        (status = 403, description = "无权查看他人订单", body = ProblemDetails),
        (status = 404, description = "订单不存在", body = ProblemDetails)
    )
)]
pub async fn handle(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
) -> Result<axum::Json<FlashOrderDetailView>, ApiProblem> {
    let instance = format!("/api/orders/{order_id}");
    let buyer_id = extract_buyer_id(&headers, &instance)?;
    let view = handler::query_detail(&state, &buyer_id, &order_id, &instance).await?;
    Ok(axum::Json(view))
}

fn route() -> MethodRouter<AppState> {
    get(handle)
}

inventory::submit! {
    SliceDescriptor {
        id: "qry_get_order",
        context: "ordering",
        method: SliceMethod::Get,
        path: "/api/orders/{order_id}",
        status: SliceStatus::Active,
        route,
    }
}
