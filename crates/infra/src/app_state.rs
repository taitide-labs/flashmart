use std::sync::Arc;

use chrono::Duration;
use flashmart_contracts::{Clock, IdGenerator, InventoryPort};
use sqlx::PgPool;

use crate::EventBus;

/// 运行时共享组合根状态，供切片处理器与后台任务共享
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub clock: Arc<dyn Clock>,
    pub id_gen: Arc<dyn IdGenerator>,
    pub inventory_port: Arc<dyn InventoryPort>,
    pub event_bus: Arc<dyn EventBus>,
    pub payment_hold_duration: Duration,
}

impl AppState {
    pub fn new(
        pool: PgPool,
        clock: Arc<dyn Clock>,
        id_gen: Arc<dyn IdGenerator>,
        inventory_port: Arc<dyn InventoryPort>,
        event_bus: Arc<dyn EventBus>,
        payment_hold_duration: Duration,
    ) -> Self {
        Self {
            pool,
            clock,
            id_gen,
            inventory_port,
            event_bus,
            payment_hold_duration,
        }
    }
}
