use chrono::{DateTime, Utc};
use flashmart_contracts::{IntegrationEvent, OrderLifecycleStatus};
use flashmart_primitives::{
    BuyerId, Currency, ItemId, ItemKind, MoneyCents, OrderId, OrderVersion, Quantity,
    ReservationId,
};

use crate::{OrderingDomainError, PaymentHoldDuration};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderState {
    PendingPayment {
        payment_deadline: DateTime<Utc>,
    },
    Paid {
        payment_deadline: DateTime<Utc>,
        paid_at: DateTime<Utc>,
    },
    Cancelled {
        payment_deadline: DateTime<Utc>,
        cancelled_at: DateTime<Utc>,
        was_paid: bool,
    },
    Expired {
        payment_deadline: DateTime<Utc>,
        expired_at: DateTime<Utc>,
    },
}

impl OrderState {
    pub fn lifecycle_status(self) -> OrderLifecycleStatus {
        match self {
            Self::PendingPayment { .. } => OrderLifecycleStatus::PendingPayment,
            Self::Paid { .. } => OrderLifecycleStatus::Paid,
            Self::Cancelled { .. } => OrderLifecycleStatus::Cancelled,
            Self::Expired { .. } => OrderLifecycleStatus::Expired,
        }
    }

    pub fn payment_deadline(self) -> DateTime<Utc> {
        match self {
            Self::PendingPayment { payment_deadline }
            | Self::Paid { payment_deadline, .. }
            | Self::Cancelled { payment_deadline, .. }
            | Self::Expired { payment_deadline, .. } => payment_deadline,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashOrder {
    id: OrderId,
    reservation_id: ReservationId,
    buyer_id: BuyerId,
    item_id: ItemId,
    item_title: String,
    item_kind: ItemKind,
    quantity: Quantity,
    unit_price_cents: MoneyCents,
    total_amount_cents: MoneyCents,
    currency: Currency,
    state: OrderState,
    version: OrderVersion,
    created_at: DateTime<Utc>,
    fulfillment_hint: String,
}

impl FlashOrder {
    #[allow(clippy::too_many_arguments)]
    pub fn place(
        id: OrderId,
        reservation_id: ReservationId,
        buyer_id: BuyerId,
        item_id: ItemId,
        item_title: String,
        item_kind: ItemKind,
        quantity: Quantity,
        unit_price_cents: MoneyCents,
        currency: Currency,
        hold_duration: PaymentHoldDuration,
        now: DateTime<Utc>,
    ) -> Result<(Self, IntegrationEvent), OrderingDomainError> {
        let total_amount_cents = unit_price_cents
            .checked_mul_quantity(quantity)
            .map_err(|_| OrderingDomainError::AmountOverflow)?;
        let payment_deadline = now + hold_duration.as_duration();
        let fulfillment_hint = pending_hint(item_kind);

        let order = Self {
            id,
            reservation_id,
            buyer_id: buyer_id.clone(),
            item_id,
            item_title,
            item_kind,
            quantity,
            unit_price_cents,
            total_amount_cents,
            currency,
            state: OrderState::PendingPayment { payment_deadline },
            version: OrderVersion::INITIAL,
            created_at: now,
            fulfillment_hint,
        };

        let event = IntegrationEvent::FlashOrderPlaced {
            order_id: id,
            reservation_id,
            buyer_id,
            item_id,
            quantity,
            total_amount_cents,
            occurred_at: now,
        };

        Ok((order, event))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rehydrate(
        id: OrderId,
        reservation_id: ReservationId,
        buyer_id: BuyerId,
        item_id: ItemId,
        item_title: String,
        item_kind: ItemKind,
        quantity: Quantity,
        unit_price_cents: MoneyCents,
        total_amount_cents: MoneyCents,
        currency: Currency,
        state: OrderState,
        version: OrderVersion,
        created_at: DateTime<Utc>,
        fulfillment_hint: String,
    ) -> Self {
        Self {
            id,
            reservation_id,
            buyer_id,
            item_id,
            item_title,
            item_kind,
            quantity,
            unit_price_cents,
            total_amount_cents,
            currency,
            state,
            version,
            created_at,
            fulfillment_hint,
        }
    }

    pub fn id(&self) -> OrderId {
        self.id
    }

    pub fn reservation_id(&self) -> ReservationId {
        self.reservation_id
    }

    pub fn buyer_id(&self) -> &BuyerId {
        &self.buyer_id
    }

    pub fn item_id(&self) -> ItemId {
        self.item_id
    }

    pub fn item_title(&self) -> &str {
        &self.item_title
    }

    pub fn item_kind(&self) -> ItemKind {
        self.item_kind
    }

    pub fn quantity(&self) -> Quantity {
        self.quantity
    }

    pub fn unit_price_cents(&self) -> MoneyCents {
        self.unit_price_cents
    }

    pub fn total_amount_cents(&self) -> MoneyCents {
        self.total_amount_cents
    }

    pub fn currency(&self) -> Currency {
        self.currency
    }

    pub fn state(&self) -> OrderState {
        self.state
    }

    pub fn version(&self) -> OrderVersion {
        self.version
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub fn fulfillment_hint(&self) -> &str {
        &self.fulfillment_hint
    }

    pub fn pay(
        &mut self,
        caller: &BuyerId,
        expected_version: OrderVersion,
        now: DateTime<Utc>,
    ) -> Result<IntegrationEvent, OrderingDomainError> {
        self.ensure_owned_and_version(caller, expected_version)?;
        match self.state {
            OrderState::PendingPayment { payment_deadline } => {
                if now > payment_deadline {
                    return Err(OrderingDomainError::OrderExpired);
                }
                self.state = OrderState::Paid {
                    payment_deadline,
                    paid_at: now,
                };
                self.version = self.version.next();
                self.fulfillment_hint = build_fulfillment_hint(self.item_kind, self.id);
                Ok(IntegrationEvent::FlashOrderPaid {
                    order_id: self.id,
                    reservation_id: self.reservation_id,
                    buyer_id: self.buyer_id.clone(),
                    item_id: self.item_id,
                    quantity: self.quantity,
                    paid_at: now,
                })
            }
            OrderState::Expired { .. } => Err(OrderingDomainError::OrderExpired),
            other => Err(OrderingDomainError::InvalidOrderState {
                status: other.lifecycle_status(),
                action: "pay",
            }),
        }
    }

    pub fn cancel(
        &mut self,
        caller: &BuyerId,
        expected_version: OrderVersion,
        now: DateTime<Utc>,
    ) -> Result<(bool, IntegrationEvent), OrderingDomainError> {
        self.ensure_owned_and_version(caller, expected_version)?;
        match self.state {
            OrderState::PendingPayment { payment_deadline } => {
                self.state = OrderState::Cancelled {
                    payment_deadline,
                    cancelled_at: now,
                    was_paid: false,
                };
                self.version = self.version.next();
                self.fulfillment_hint = "已主动取消，预占库存已释放。".to_owned();
                Ok((
                    false,
                    IntegrationEvent::FlashOrderCancelled {
                        order_id: self.id,
                        reservation_id: self.reservation_id,
                        buyer_id: self.buyer_id.clone(),
                        item_id: self.item_id,
                        quantity: self.quantity,
                        was_paid: false,
                        cancelled_at: now,
                    },
                ))
            }
            OrderState::Paid {
                payment_deadline, ..
            } => {
                self.state = OrderState::Cancelled {
                    payment_deadline,
                    cancelled_at: now,
                    was_paid: true,
                };
                self.version = self.version.next();
                self.fulfillment_hint = "已取消并完成模拟原路退款，库存已退回。".to_owned();
                Ok((
                    true,
                    IntegrationEvent::FlashOrderCancelled {
                        order_id: self.id,
                        reservation_id: self.reservation_id,
                        buyer_id: self.buyer_id.clone(),
                        item_id: self.item_id,
                        quantity: self.quantity,
                        was_paid: true,
                        cancelled_at: now,
                    },
                ))
            }
            other => Err(OrderingDomainError::InvalidOrderState {
                status: other.lifecycle_status(),
                action: "cancel",
            }),
        }
    }

    pub fn expire(&mut self, now: DateTime<Utc>) -> Result<IntegrationEvent, OrderingDomainError> {
        match self.state {
            OrderState::PendingPayment { payment_deadline } => {
                if now <= payment_deadline {
                    return Err(OrderingDomainError::NotDueForExpiration);
                }
                self.state = OrderState::Expired {
                    payment_deadline,
                    expired_at: now,
                };
                self.version = self.version.next();
                self.fulfillment_hint = "超过支付保留期未付款，预占库存已自动回流。".to_owned();
                Ok(IntegrationEvent::FlashOrderExpired {
                    order_id: self.id,
                    reservation_id: self.reservation_id,
                    buyer_id: self.buyer_id.clone(),
                    item_id: self.item_id,
                    quantity: self.quantity,
                    expired_at: now,
                })
            }
            other => Err(OrderingDomainError::InvalidOrderState {
                status: other.lifecycle_status(),
                action: "expire",
            }),
        }
    }

    fn ensure_owned_and_version(
        &self,
        caller: &BuyerId,
        expected_version: OrderVersion,
    ) -> Result<(), OrderingDomainError> {
        if &self.buyer_id != caller {
            return Err(OrderingDomainError::OwnershipMismatch);
        }
        if self.version != expected_version {
            return Err(OrderingDomainError::VersionConflict {
                expected: expected_version.get(),
                actual: self.version.get(),
            });
        }
        Ok(())
    }
}

fn pending_hint(kind: ItemKind) -> String {
    match kind {
        ItemKind::Digital => "完成模拟结算后即刻生成数字兑换凭证".to_owned(),
        ItemKind::Physical => "完成模拟结算后进入限量实体待配货队列".to_owned(),
    }
}

pub fn build_fulfillment_hint(kind: ItemKind, order_id: OrderId) -> String {
    let hex = order_id.as_uuid().simple().to_string();
    let suffix = hex[hex.len() - 6..].to_ascii_uppercase();
    match kind {
        ItemKind::Digital => format!("模拟数字兑换码：FM-DIGI-{suffix}"),
        ItemKind::Physical => format!("模拟配货单号：FM-PKG-{suffix}（等待出库）"),
    }
}
