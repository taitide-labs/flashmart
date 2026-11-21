use flashmart_primitives::OrderId;
use thiserror::Error;

/// 超时未付订单自动过期批处理命令（由服务端定时调度或验收测试触发）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpireUnpaidOrdersCommand {
    pub batch_limit: i64,
}

impl Default for ExpireUnpaidOrdersCommand {
    fn default() -> Self {
        Self { batch_limit: 50 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpireUnpaidOrdersOutcome {
    pub expired_order_ids: Vec<OrderId>,
}

#[derive(Debug, Error)]
pub enum ExpireUnpaidOrdersError {
    #[error("过期扫描尚未实现")]
    NotImplemented,
    #[error("数据库错误：{0}")]
    Database(String),
    #[error("库存释放失败：{0}")]
    Inventory(String),
}
