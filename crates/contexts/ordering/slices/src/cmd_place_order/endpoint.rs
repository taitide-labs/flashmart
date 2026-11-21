use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{MethodRouter, post};
use flashmart_infra::{
    ApiProblem, AppState, ProblemDetails, SliceDescriptor, SliceMethod, SliceStatus,
    extract_buyer_id, extract_idempotency_key,
};
use http::HeaderMap;

use super::contract::{FlashOrderView, PlaceOrderCommand, PlaceOrderRequest};
use super::handler;

#[utoipa::path(
    post,
    path = "/api/orders",
    tag = "ordering",
    params(
        ("X-Buyer-Id" = String, Header, description = "轻量买家标识"),
        ("Idempotency-Key" = String, Header, description = "请求幂等键")
    ),
    request_body = PlaceOrderRequest,
    responses(
        (status = 201, description = "预占库存成功并创建待支付订单", body = FlashOrderView),
        (status = 400, description = "缺少必要请求头", body = ProblemDetails),
        (status = 404, description = "商品不存在", body = ProblemDetails),
        (status = 409, description = "库存不足、不在时间窗、超限购或幂等冲突", body = ProblemDetails),
        (status = 422, description = "下单参数无效", body = ProblemDetails)
    )
)]
pub async fn handle(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PlaceOrderRequest>,
) -> Result<(StatusCode, Json<FlashOrderView>), ApiProblem> {
    let buyer_id = extract_buyer_id(&headers, "/api/orders")?;
    let idempotency_key = extract_idempotency_key(&headers, "/api/orders")?;
    let cmd = PlaceOrderCommand::parse(buyer_id, idempotency_key, req, "/api/orders")?;
    let (status, view) = handler::handle(&state, cmd).await?;
    Ok((status, Json(view)))
}

fn route() -> MethodRouter<AppState> {
    post(handle)
}

inventory::submit! {
    SliceDescriptor {
        id: "cmd_place_order",
        context: "ordering",
        method: SliceMethod::Post,
        path: "/api/orders",
        status: SliceStatus::Active,
        route,
    }
}
