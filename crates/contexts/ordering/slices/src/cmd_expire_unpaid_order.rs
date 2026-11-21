pub mod contract;

use flashmart_infra::AppState;

use contract::{ExpireUnpaidOrdersCommand, ExpireUnpaidOrdersError, ExpireUnpaidOrdersOutcome};

/// 冻结命令入口（阶段 4 仅返回 NotImplemented，业务编排由工单交付）
pub async fn execute(
    _state: &AppState,
    _cmd: ExpireUnpaidOrdersCommand,
) -> Result<ExpireUnpaidOrdersOutcome, ExpireUnpaidOrdersError> {
    Err(ExpireUnpaidOrdersError::NotImplemented)
}
