use axum::Json;
use axum::extract::{Path, State};
use axum::routing::{MethodRouter, post};
use flashmart_infra::{
    ApiProblem, AppState, ProblemDetails, SliceDescriptor, SliceMethod, SliceStatus,
    extract_buyer_id, extract_idempotency_key,
};
use http::HeaderMap;

use super::contract::{PaidOrderView, PayOrderCommand, PayOrderRequest};
use super::handler;

#[utoipa::path(
    post,
    path = "/api/orders/{order_id}/pay",
    tag = "ordering",
    params(
        ("order_id" = String, Path, description = "抢购订单 UUID"),
        ("X-Buyer-Id" = String, Header, description = "轻量买家标识"),
        ("Idempotency-Key" = String, Header, description = "请求幂等键")
    ),
    request_body = PayOrderRequest,
    responses(
        (status = 200, description = "模拟结算成功并确认库存扣减", body = PaidOrderView),
        (status = 400, description = "缺少必要请求头", body = ProblemDetails),
        (status = 403, description = "无权操作他人订单", body = ProblemDetails),
        (status = 404, description = "订单不存在", body = ProblemDetails),
        (status = 409, description = "版本冲突、已过期、状态不允许或幂等冲突", body = ProblemDetails),
        (status = 422, description = "请求参数无效", body = ProblemDetails)
    )
)]
pub async fn handle(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
    Json(req): Json<PayOrderRequest>,
) -> Result<Json<PaidOrderView>, ApiProblem> {
    let instance = format!("/api/orders/{order_id}/pay");
    let buyer_id = extract_buyer_id(&headers, &instance)?;
    let idempotency_key = extract_idempotency_key(&headers, &instance)?;
    let cmd = PayOrderCommand::parse(buyer_id, idempotency_key, &order_id, req, &instance)?;
    let view = handler::handle(&state, cmd).await?;
    Ok(Json(view))
}

fn route() -> MethodRouter<AppState> {
    post(handle)
}

inventory::submit! {
    SliceDescriptor {
        id: "cmd_pay_order",
        context: "ordering",
        method: SliceMethod::Post,
        path: "/api/orders/{order_id}/pay",
        status: SliceStatus::Active,
        route,
    }
}
