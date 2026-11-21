use chrono::{DateTime, Utc};
use flashmart_contracts::WindowStatus;

use crate::InventoryDomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaleWindow {
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
}

impl SaleWindow {
    pub fn try_new(
        starts_at: DateTime<Utc>,
        ends_at: DateTime<Utc>,
    ) -> Result<Self, InventoryDomainError> {
        if ends_at <= starts_at {
            return Err(InventoryDomainError::InvalidSaleWindowRange);
        }
        Ok(Self { starts_at, ends_at })
    }

    pub fn starts_at(self) -> DateTime<Utc> {
        self.starts_at
    }

    pub fn ends_at(self) -> DateTime<Utc> {
        self.ends_at
    }

    pub fn status_at(self, now: DateTime<Utc>) -> WindowStatus {
        WindowStatus::at(now, self.starts_at, self.ends_at)
    }
}
