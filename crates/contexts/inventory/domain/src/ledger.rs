use flashmart_contracts::WindowStatus;
use flashmart_primitives::Quantity;

use crate::InventoryDomainError;

/// 库存账本：严守 `available + reserved + confirmed + returned_after_close == total_stock`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InventoryLedger {
    total_stock: i32,
    available_stock: i32,
    reserved_stock: i32,
    confirmed_stock: i32,
    returned_after_close_stock: i32,
}

impl InventoryLedger {
    pub fn try_new(total_stock: i32) -> Result<Self, InventoryDomainError> {
        Self::rehydrate(total_stock, total_stock, 0, 0, 0)
    }

    pub fn rehydrate(
        total_stock: i32,
        available_stock: i32,
        reserved_stock: i32,
        confirmed_stock: i32,
        returned_after_close_stock: i32,
    ) -> Result<Self, InventoryDomainError> {
        if total_stock < 0
            || available_stock < 0
            || reserved_stock < 0
            || confirmed_stock < 0
            || returned_after_close_stock < 0
        {
            return Err(InventoryDomainError::LedgerInvariantViolated);
        }

        let sum = i64::from(available_stock)
            + i64::from(reserved_stock)
            + i64::from(confirmed_stock)
            + i64::from(returned_after_close_stock);
        if sum != i64::from(total_stock) {
            return Err(InventoryDomainError::LedgerInvariantViolated);
        }

        Ok(Self {
            total_stock,
            available_stock,
            reserved_stock,
            confirmed_stock,
            returned_after_close_stock,
        })
    }

    pub fn total_stock(self) -> i32 {
        self.total_stock
    }

    pub fn available_stock(self) -> i32 {
        self.available_stock
    }

    pub fn reserved_stock(self) -> i32 {
        self.reserved_stock
    }

    pub fn confirmed_stock(self) -> i32 {
        self.confirmed_stock
    }

    pub fn returned_after_close_stock(self) -> i32 {
        self.returned_after_close_stock
    }

    pub fn reserve(
        &mut self,
        quantity: Quantity,
        window_status: WindowStatus,
    ) -> Result<(), InventoryDomainError> {
        if window_status != WindowStatus::Active {
            return Err(InventoryDomainError::SaleWindowNotActive {
                status: window_status,
            });
        }
        let qty = quantity.get();
        if self.available_stock < qty {
            return Err(InventoryDomainError::InsufficientStock);
        }
        self.available_stock -= qty;
        self.reserved_stock += qty;
        Ok(())
    }

    pub fn confirm_deduction(&mut self, quantity: Quantity) -> Result<(), InventoryDomainError> {
        let qty = quantity.get();
        if self.reserved_stock < qty {
            return Err(InventoryDomainError::LedgerInvariantViolated);
        }
        self.reserved_stock -= qty;
        self.confirmed_stock += qty;
        Ok(())
    }

    /// 释放预占库存；仅当仍处于进行中时间窗时恢复为可售库存，否则记账为闭窗后归还
    pub fn release_reserved(
        &mut self,
        quantity: Quantity,
        window_status: WindowStatus,
    ) -> Result<bool, InventoryDomainError> {
        let qty = quantity.get();
        if self.reserved_stock < qty {
            return Err(InventoryDomainError::LedgerInvariantViolated);
        }
        self.reserved_stock -= qty;
        if window_status == WindowStatus::Active {
            self.available_stock += qty;
            Ok(true)
        } else {
            self.returned_after_close_stock += qty;
            Ok(false)
        }
    }

    /// 退回已确认扣减库存；仅当仍处于进行中时间窗时恢复为可售库存，否则记账为闭窗后归还
    pub fn return_deducted(
        &mut self,
        quantity: Quantity,
        window_status: WindowStatus,
    ) -> Result<bool, InventoryDomainError> {
        let qty = quantity.get();
        if self.confirmed_stock < qty {
            return Err(InventoryDomainError::LedgerInvariantViolated);
        }
        self.confirmed_stock -= qty;
        if window_status == WindowStatus::Active {
            self.available_stock += qty;
            Ok(true)
        } else {
            self.returned_after_close_stock += qty;
            Ok(false)
        }
    }
}
