use flashmart_infra::{FieldValidationError, ProblemDetails};
use flashmart_inventory_slices::{qry_get_flash_item, qry_list_flash_items};
use flashmart_ordering_slices::{
    cmd_cancel_order, cmd_pay_order, cmd_place_order, qry_get_order, qry_list_orders,
};
use utoipa::OpenApi;

use crate::app::{self, HealthResponse};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "FlashMart API",
        version = "0.1.0",
        description = "FlashMart 限时抢购与模拟结算后端冻结契约"
    ),
    paths(
        app::health,
        qry_list_flash_items::endpoint::handle,
        qry_get_flash_item::endpoint::handle,
        qry_list_orders::endpoint::handle,
        qry_get_order::endpoint::handle,
        cmd_place_order::endpoint::handle,
        cmd_pay_order::endpoint::handle,
        cmd_cancel_order::endpoint::handle,
    ),
    components(schemas(
        HealthResponse,
        ProblemDetails,
        FieldValidationError,
        qry_list_flash_items::contract::FlashItemSummary,
        qry_list_flash_items::contract::FlashItemListResponse,
        qry_get_flash_item::contract::FlashItemDetailView,
        qry_list_orders::contract::FlashOrderSummaryView,
        qry_list_orders::contract::FlashOrderListResponse,
        qry_get_order::contract::FlashOrderDetailView,
        cmd_place_order::contract::PlaceOrderRequest,
        cmd_place_order::contract::FlashOrderView,
        cmd_pay_order::contract::PayOrderRequest,
        cmd_pay_order::contract::PaidOrderView,
        cmd_cancel_order::contract::CancelOrderRequest,
        cmd_cancel_order::contract::CancelledOrderView,
    ))
)]
pub struct ApiDoc;
