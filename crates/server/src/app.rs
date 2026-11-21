use axum::Json;
use axum::Router;
use axum::routing::get;
use flashmart_infra::{AppState, collect_active_routes};
use serde::Serialize;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use utoipa::ToSchema;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub listen_addr: String,
    pub database_url: String,
    pub payment_hold_secs: i64,
    pub seed_demo_items: bool,
}

impl ServerConfig {
    pub fn from_env() -> Self {
        let listen_addr = std::env::var("FLASHMART_LISTEN_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_owned());
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postgres".to_owned());
        let payment_hold_secs = std::env::var("FLASHMART_PAYMENT_HOLD_SECS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(900);
        let seed_demo_items = std::env::var("FLASHMART_SEED_DEMO")
            .map(|v| v != "false" && v != "0")
            .unwrap_or(true);

        Self {
            listen_addr,
            database_url,
            payment_hold_secs,
            seed_demo_items,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: &'static str,
}

#[utoipa::path(
    get,
    path = "/api/health",
    tag = "system",
    responses(
        (status = 200, description = "服务健康检查", body = HealthResponse)
    )
)]
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

/// 组装宿主路由：显式链接两上下文切片注册锚点，仅挂载 `Active` 切片路由
pub fn build_app(state: AppState) -> Router {
    flashmart_inventory_slices::link();
    flashmart_ordering_slices::link();

    let slice_router = collect_active_routes(state);

    Router::new()
        .route("/api/health", get(health))
        .merge(slice_router)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}
