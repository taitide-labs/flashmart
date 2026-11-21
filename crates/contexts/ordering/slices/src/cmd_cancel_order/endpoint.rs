use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{MethodRouter, post};
use flashmart_infra::{
    ApiProblem, AppState, ProblemDetails, SliceDescriptor, SliceMethod, SliceStatus,
    extract_buyer_id, extract_idempotency_key,
};
use http::HeaderMap;

use super::contract::{CancelOrderCommand, CancelOrderRequest, CancelledOrderView};

#[utoipa::path(
    post,
    path = "/api/orders/{order_id}/cancel",
    tag = "ordering",
    params(
        ("order_id" = String, Path, description = "抢购订单 UUID"),
        ("X-Buyer-Id" = String, Header, description = "轻量买家标识"),
        ("Idempotency-Key" = String, Header, description = "请求幂等键")
    ),
    request_body = CancelOrderRequest,
    responses(
        (status = 200, description = "取消订单并按状态触发释放/退回库存", body = CancelledOrderView),
        (status = 400, description = "缺少必要请求头", body = ProblemDetails),
        (status = 403, description = "无权操作他人订单", body = ProblemDetails),
        (status = 404, description = "订单不存在", body = ProblemDetails),
        (status = 409, description = "版本冲突、不可取消状态或幂等冲突", body = ProblemDetails),
        (status = 422, description = "请求参数无效", body = ProblemDetails)
    )
)]
pub async fn handle(
    State(_state): State<AppState>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
    Json(req): Json<CancelOrderRequest>,
) -> Result<Json<CancelledOrderView>, ApiProblem> {
    let instance = format!("/api/orders/{order_id}/cancel");
    let buyer_id = extract_buyer_id(&headers, &instance)?;
    let idempotency_key = extract_idempotency_key(&headers, &instance)?;
    let _cmd = CancelOrderCommand::parse(buyer_id, idempotency_key, &order_id, req, &instance)?;
    Err(ProblemDetails::new(
        StatusCode::NOT_IMPLEMENTED,
        "slice_not_implemented",
        "接口尚未激活",
        "cmd_cancel_order 切片尚处于 Planned 基线状态。",
        instance,
    )
    .into())
}

fn route() -> MethodRouter<AppState> {
    post(handle)
}

inventory::submit! {
    SliceDescriptor {
        id: "cmd_cancel_order",
        context: "ordering",
        method: SliceMethod::Post,
        path: "/api/orders/{order_id}/cancel",
        status: SliceStatus::Planned,
        route,
    }
}
