pub mod cmd_confirm_stock_deduction;
pub mod cmd_release_reserved_stock;
pub mod cmd_reserve_stock;
pub mod cmd_return_deducted_stock;
pub mod port;
pub mod qry_get_flash_item;
pub mod qry_list_flash_items;

/// 链接锚点：宿主组合根显式调用一次，确保静态链接保留 `inventory::submit!` 注册描述符
pub fn link() {}
