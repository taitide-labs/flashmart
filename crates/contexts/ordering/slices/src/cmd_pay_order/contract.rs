use chrono::{DateTime, Utc};
use flashmart_contracts::OrderLifecycleStatus;
use flashmart_infra::{ApiProblem, FieldValidationError, ProblemDetails};
use flashmart_primitives::{
    BuyerId, Currency, IdempotencyKey, ItemId, ItemKind, MoneyCents, OrderId, OrderVersion,
    Quantity,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PayOrderRequest {
    pub version: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayOrderCommand {
    pub buyer_id: BuyerId,
    pub idempotency_key: IdempotencyKey,
    pub order_id: OrderId,
    pub expected_version: OrderVersion,
}

impl PayOrderCommand {
    pub fn parse(
        buyer_id: BuyerId,
        idempotency_key: IdempotencyKey,
        raw_order_id: &str,
        req: PayOrderRequest,
        instance: &str,
    ) -> Result<Self, ApiProblem> {
        let mut errors = Vec::new();
        let order_id = Uuid::parse_str(raw_order_id.trim()).ok().map(OrderId::new);
        if order_id.is_none() {
            errors.push(FieldValidationError {
                field: "order_id".to_owned(),
                message: "订单编号必须为合法 UUID".to_owned(),
            });
        }
        let expected_version = OrderVersion::try_new(req.version).ok();
        if expected_version.is_none() {
            errors.push(FieldValidationError {
                field: "version".to_owned(),
                message: "版本号必须为正整数".to_owned(),
            });
        }

        if let (Some(order_id), Some(expected_version)) = (order_id, expected_version) {
            Ok(Self {
                buyer_id,
                idempotency_key,
                order_id,
                expected_version,
            })
        } else {
            Err(ProblemDetails::validation(
                instance,
                "支付参数无效",
                "请传入合法订单编号与正整数 version。",
                errors,
            )
            .into())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PaidOrderView {
    pub id: OrderId,
    pub buyer_id: BuyerId,
    pub item_id: ItemId,
    pub item_title: String,
    pub item_kind: ItemKind,
    pub quantity: Quantity,
    pub unit_price_cents: MoneyCents,
    pub total_amount_cents: MoneyCents,
    pub currency: Currency,
    pub status: OrderLifecycleStatus,
    pub version: OrderVersion,
    pub payment_deadline: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub paid_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub expired_at: Option<DateTime<Utc>>,
    pub fulfillment_hint: Option<String>,
}
