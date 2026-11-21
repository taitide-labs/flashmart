use axum::Json;
use axum::Router;
use axum::routing::get;
use chrono::{DateTime, Duration, Utc};
use flashmart_infra::{AppState, collect_active_routes};
use flashmart_ordering_slices::cmd_expire_unpaid_order;
use flashmart_ordering_slices::cmd_expire_unpaid_order::contract::ExpireUnpaidOrdersCommand;
use serde::Serialize;
use sqlx::PgPool;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub listen_addr: String,
    pub database_url: String,
    pub payment_hold_secs: i64,
    pub seed_demo_items: bool,
    pub expire_scan_interval_secs: u64,
}

impl ServerConfig {
    pub fn from_env() -> Self {
        let listen_addr =
            std::env::var("FLASHMART_LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_owned());
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
        let expire_scan_interval_secs = std::env::var("FLASHMART_EXPIRE_SCAN_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(5);

        Self {
            listen_addr,
            database_url,
            payment_hold_secs,
            seed_demo_items,
            expire_scan_interval_secs,
        }
    }
}

struct DemoItemSeed {
    id: Uuid,
    title: &'static str,
    description: &'static str,
    item_kind: &'static str,
    price_cents: i64,
    total_stock: i32,
    available_stock: i32,
    confirmed_stock: i32,
    per_user_limit: i32,
    starts_offset_mins: i64,
    ends_offset_mins: i64,
}

const DEMO_ITEM_SEEDS: &[DemoItemSeed] = &[
    DemoItemSeed {
        id: Uuid::from_u128(0x1111_1111_1111_4111_8111_1111_1111_1101),
        title: "FlashMart 创世数字徽章 · Pro Pass",
        description: "含独立编号与社区抢先体验资格的限量数字通行证，下单预占库存，结算后即刻交付凭证。",
        item_kind: "digital",
        price_cents: 9900,
        total_stock: 10,
        available_stock: 10,
        confirmed_stock: 0,
        per_user_limit: 1,
        starts_offset_mins: -15,
        ends_offset_mins: 120,
    },
    DemoItemSeed {
        id: Uuid::from_u128(0x1111_1111_1111_4111_8111_1111_1111_1102),
        title: "CNC 阳极氧化铝机械旋钮宏键盘",
        description: "实体好物限量批次，工业橙旋钮搭配静音线性轴，支持单人限购 1 台与取消退库回流。",
        item_kind: "physical",
        price_cents: 49900,
        total_stock: 5,
        available_stock: 5,
        confirmed_stock: 0,
        per_user_limit: 1,
        starts_offset_mins: -8,
        ends_offset_mins: 90,
    },
    DemoItemSeed {
        id: Uuid::from_u128(0x1111_1111_1111_4111_8111_1111_1111_1103),
        title: "午夜合成器 WAV 采样母带包",
        description: "数字无损音频采样合集，本场首批已售罄，若有待支付超时或主动取消将实时回流。",
        item_kind: "digital",
        price_cents: 5900,
        total_stock: 12,
        available_stock: 0,
        confirmed_stock: 12,
        per_user_limit: 1,
        starts_offset_mins: -30,
        ends_offset_mins: 60,
    },
    DemoItemSeed {
        id: Uuid::from_u128(0x1111_1111_1111_4111_8111_1111_1111_1104),
        title: "联名钛金属 EDC 随身钥匙扣",
        description: "整块五级钛 CNC 铣削成型，即将开放限时抢购，倒计时结束后可发起原子预占。",
        item_kind: "physical",
        price_cents: 18900,
        total_stock: 8,
        available_stock: 8,
        confirmed_stock: 0,
        per_user_limit: 1,
        starts_offset_mins: 25,
        ends_offset_mins: 145,
    },
    DemoItemSeed {
        id: Uuid::from_u128(0x1111_1111_1111_4111_8111_1111_1111_1105),
        title: "首发工业贴纸与编号卡册套装",
        description: "首场测试批次已于早些时候截单，仅保留账面展示不再开放抢购。",
        item_kind: "physical",
        price_cents: 3900,
        total_stock: 15,
        available_stock: 1,
        confirmed_stock: 14,
        per_user_limit: 1,
        starts_offset_mins: -180,
        ends_offset_mins: -20,
    },
];

/// 幂等写入演示限量商品，受 `FLASHMART_SEED_DEMO` 控制；已存在记录不覆盖实时库存
pub async fn seed_demo_items(pool: &PgPool, now: DateTime<Utc>) -> Result<u64, sqlx::Error> {
    let mut inserted = 0_u64;
    for item in DEMO_ITEM_SEEDS {
        let starts_at = now + Duration::minutes(item.starts_offset_mins);
        let ends_at = now + Duration::minutes(item.ends_offset_mins);
        let result = sqlx::query(
            r#"
            INSERT INTO inventory.flash_items (
                id, title, description, item_kind, price_cents, currency,
                total_stock, available_stock, reserved_stock, confirmed_stock,
                returned_after_close_stock, per_user_limit, sale_starts_at, sale_ends_at
            )
            VALUES ($1, $2, $3, $4, $5, 'CNY', $6, $7, 0, $8, 0, $9, $10, $11)
            ON CONFLICT (id) DO NOTHING
            "#,
        )
        .bind(item.id)
        .bind(item.title)
        .bind(item.description)
        .bind(item.item_kind)
        .bind(item.price_cents)
        .bind(item.total_stock)
        .bind(item.available_stock)
        .bind(item.confirmed_stock)
        .bind(item.per_user_limit)
        .bind(starts_at)
        .bind(ends_at)
        .execute(pool)
        .await?;
        inserted += result.rows_affected();
    }
    Ok(inserted)
}

/// 执行单次超时未付订单过期巡检，返回本轮成功过期并释放库存的订单数
pub async fn run_expiration_sweep_once(state: &AppState) -> usize {
    match cmd_expire_unpaid_order::execute(state, ExpireUnpaidOrdersCommand::default()).await {
        Ok(outcome) => {
            let count = outcome.expired_order_ids.len();
            if count > 0 {
                tracing::info!(
                    expired_count = count,
                    "expired overdue unpaid flash orders and released reserved stock"
                );
            }
            count
        }
        Err(err) => {
            tracing::warn!(error = %err, "background unpaid order expiration sweep failed");
            0
        }
    }
}

/// 启动后台超时未付订单定时巡检任务
pub fn spawn_expiration_sweeper(
    state: AppState,
    scan_interval: std::time::Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(scan_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        // 跳过启动瞬间首次即时 tick，等待首个完整巡检周期后再扫描
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let _ = run_expiration_sweep_once(&state).await;
        }
    })
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
