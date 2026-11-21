use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{MethodRouter, get};
use flashmart_infra::{
    ApiProblem, AppState, ProblemDetails, SliceDescriptor, SliceMethod, SliceStatus,
    extract_buyer_id,
};
use http::HeaderMap;

use super::contract::FlashOrderDetailView;

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
    State(_state): State<AppState>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
) -> Result<axum::Json<FlashOrderDetailView>, ApiProblem> {
    let instance = format!("/api/orders/{order_id}");
    let _buyer_id = extract_buyer_id(&headers, &instance)?;
    Err(ProblemDetails::new(
        StatusCode::NOT_IMPLEMENTED,
        "slice_not_implemented",
        "接口尚未激活",
        "qry_get_order 切片尚处于 Planned 基线状态。",
        instance,
    )
    .into())
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
        status: SliceStatus::Planned,
        route,
    }
}
