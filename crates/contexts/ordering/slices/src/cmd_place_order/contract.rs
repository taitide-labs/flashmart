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
pub struct PlaceOrderRequest {
    pub item_id: String,
    pub quantity: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceOrderCommand {
    pub buyer_id: BuyerId,
    pub idempotency_key: IdempotencyKey,
    pub item_id: ItemId,
    pub quantity: Quantity,
}

impl PlaceOrderCommand {
    pub fn parse(
        buyer_id: BuyerId,
        idempotency_key: IdempotencyKey,
        req: PlaceOrderRequest,
        instance: &str,
    ) -> Result<Self, ApiProblem> {
        let mut errors = Vec::new();
        let parsed_item_id = Uuid::parse_str(req.item_id.trim()).ok().map(ItemId::new);
        if parsed_item_id.is_none() {
            errors.push(FieldValidationError {
                field: "item_id".to_owned(),
                message: "商品编号必须为合法 UUID".to_owned(),
            });
        }
        let parsed_qty = Quantity::try_new(req.quantity).ok();
        if parsed_qty.is_none() {
            errors.push(FieldValidationError {
                field: "quantity".to_owned(),
                message: "购买数量必须大于等于 1".to_owned(),
            });
        }

        if let (Some(item_id), Some(quantity)) = (parsed_item_id, parsed_qty) {
            Ok(Self {
                buyer_id,
                idempotency_key,
                item_id,
                quantity,
            })
        } else {
            Err(ProblemDetails::validation(
                instance,
                "下单参数无效",
                "请检查商品编号与购买数量（需为正整数）。",
                errors,
            )
            .into())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FlashOrderView {
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
