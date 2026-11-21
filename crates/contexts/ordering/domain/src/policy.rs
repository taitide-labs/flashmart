use chrono::Duration;
use flashmart_primitives::Quantity;

use crate::OrderingDomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentHoldDuration(Duration);

impl PaymentHoldDuration {
    pub const DEFAULT_SECS: i64 = 15 * 60;

    pub fn try_new(duration: Duration) -> Result<Self, OrderingDomainError> {
        if duration <= Duration::zero() {
            return Err(OrderingDomainError::InvalidPaymentHoldDuration);
        }
        Ok(Self(duration))
    }

    pub fn as_duration(self) -> Duration {
        self.0
    }
}

impl Default for PaymentHoldDuration {
    fn default() -> Self {
        Self(Duration::seconds(Self::DEFAULT_SECS))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveQuota {
    per_user_limit: i32,
    current_active_qty: i32,
}

impl ActiveQuota {
    pub fn try_new(per_user_limit: i32, current_active_qty: i32) -> Result<Self, OrderingDomainError> {
        if per_user_limit < 1 {
            return Err(OrderingDomainError::InvalidPerUserLimit);
        }
        Ok(Self {
            per_user_limit,
            current_active_qty: current_active_qty.max(0),
        })
    }

    pub fn ensure_can_place(self, requested: Quantity) -> Result<(), OrderingDomainError> {
        let next_total = self.current_active_qty.saturating_add(requested.get());
        if next_total > self.per_user_limit {
            return Err(OrderingDomainError::PurchaseLimitExceeded {
                limit: self.per_user_limit,
                active_qty: self.current_active_qty,
            });
        }
        Ok(())
    }
}
