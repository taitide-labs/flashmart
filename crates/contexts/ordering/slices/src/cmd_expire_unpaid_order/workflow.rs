use chrono::{DateTime, Utc};
use flashmart_contracts::IntegrationEvent;
use flashmart_ordering_domain::{FlashOrder, OrderState, OrderingDomainError};
use flashmart_primitives::OrderVersion;

/// 将批处理上限规范化为有效的 SQL LIMIT；非正数表示本轮不扫描
pub fn normalize_batch_limit(batch_limit: i64) -> Option<i64> {
    (batch_limit > 0).then_some(batch_limit)
}

#[derive(Debug, Clone)]
pub struct PreparedExpiration {
    pub order: FlashOrder,
    pub expected_version: OrderVersion,
    pub expired_at: DateTime<Utc>,
    pub event: IntegrationEvent,
}

/// 零副作用领域编排：校验待支付订单是否已超过支付保留期，并流转为已过期状态
pub fn prepare_expiration(
    mut order: FlashOrder,
    now: DateTime<Utc>,
) -> Result<PreparedExpiration, OrderingDomainError> {
    let expected_version = order.version();
    let event = order.expire(now)?;
    let expired_at = match order.state() {
        OrderState::Expired { expired_at, .. } => expired_at,
        _ => now,
    };

    Ok(PreparedExpiration {
        order,
        expected_version,
        expired_at,
        event,
    })
}
