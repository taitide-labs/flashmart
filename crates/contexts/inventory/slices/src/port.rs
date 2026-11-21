use std::sync::Arc;

use async_trait::async_trait;
use flashmart_contracts::{
    Clock, ConfirmStockDeductionRequest, ConfirmStockOutcome, FlashItemSnapshot, IdGenerator,
    InventoryPort, InventoryPortError, ReleaseReservedStockRequest, ReleaseStockOutcome,
    ReserveStockOutcome, ReserveStockRequest, ReturnDeductedStockRequest, ReturnStockOutcome,
};
use flashmart_infra::EventBus;
use flashmart_primitives::ItemId;
use sqlx::PgPool;

/// 基于 PostgreSQL 的库存同步端口适配器（阶段 4 仅冻结连接骨架，业务实现由工单交付）
#[derive(Clone)]
pub struct PgInventoryPort {
    pub pool: PgPool,
    pub clock: Arc<dyn Clock>,
    pub id_gen: Arc<dyn IdGenerator>,
    pub event_bus: Arc<dyn EventBus>,
}

impl PgInventoryPort {
    pub fn new(
        pool: PgPool,
        clock: Arc<dyn Clock>,
        id_gen: Arc<dyn IdGenerator>,
        event_bus: Arc<dyn EventBus>,
    ) -> Self {
        Self {
            pool,
            clock,
            id_gen,
            event_bus,
        }
    }
}

#[async_trait]
impl InventoryPort for PgInventoryPort {
    async fn list_flash_items(&self) -> Result<Vec<FlashItemSnapshot>, InventoryPortError> {
        crate::qry_list_flash_items::handler::list_snapshots(&self.pool, self.clock.now()).await
    }

    async fn get_flash_item(
        &self,
        _item_id: ItemId,
    ) -> Result<Option<FlashItemSnapshot>, InventoryPortError> {
        Err(InventoryPortError::NotImplemented("qry_get_flash_item"))
    }

    async fn reserve_stock(
        &self,
        _req: ReserveStockRequest,
    ) -> Result<ReserveStockOutcome, InventoryPortError> {
        Err(InventoryPortError::NotImplemented("cmd_reserve_stock"))
    }

    async fn confirm_stock_deduction(
        &self,
        _req: ConfirmStockDeductionRequest,
    ) -> Result<ConfirmStockOutcome, InventoryPortError> {
        Err(InventoryPortError::NotImplemented(
            "cmd_confirm_stock_deduction",
        ))
    }

    async fn release_reserved_stock(
        &self,
        _req: ReleaseReservedStockRequest,
    ) -> Result<ReleaseStockOutcome, InventoryPortError> {
        Err(InventoryPortError::NotImplemented(
            "cmd_release_reserved_stock",
        ))
    }

    async fn return_deducted_stock(
        &self,
        _req: ReturnDeductedStockRequest,
    ) -> Result<ReturnStockOutcome, InventoryPortError> {
        Err(InventoryPortError::NotImplemented(
            "cmd_return_deducted_stock",
        ))
    }
}
