pub mod contract;
pub mod handler;
pub mod workflow;

#[cfg(test)]
mod tests;

use flashmart_infra::AppState;

use contract::{ExpireUnpaidOrdersCommand, ExpireUnpaidOrdersError, ExpireUnpaidOrdersOutcome};

/// 超时未付订单自动过期批处理入口：扫描超过支付保留期的待支付订单并按 OCC 流转为过期，释放预占库存
pub async fn execute(
    state: &AppState,
    cmd: ExpireUnpaidOrdersCommand,
) -> Result<ExpireUnpaidOrdersOutcome, ExpireUnpaidOrdersError> {
    handler::handle(state, cmd).await
}
