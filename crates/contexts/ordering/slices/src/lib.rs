pub mod cmd_cancel_order;
pub mod cmd_expire_unpaid_order;
pub mod cmd_pay_order;
pub mod cmd_place_order;
pub mod qry_get_order;
pub mod qry_list_orders;

/// 链接锚点：宿主组合根显式调用一次，确保静态链接保留 `inventory::submit!` 注册描述符
pub fn link() {}
