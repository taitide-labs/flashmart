use std::collections::BTreeMap;

use axum::Router;
use axum::routing::MethodRouter;

use crate::AppState;

/// 垂直切片交付状态；新建切片冻结时固定为 `Planned`，避免生产路由暴露未实现入口
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceStatus {
    Planned,
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceMethod {
    Get,
    Post,
}

/// 零冲突自注册路由描述符，由各切片 `endpoint.rs` 通过 `inventory::submit!` 提交
#[derive(Debug, Clone, Copy)]
pub struct SliceDescriptor {
    pub id: &'static str,
    pub context: &'static str,
    pub method: SliceMethod,
    pub path: &'static str,
    pub status: SliceStatus,
    pub route: fn() -> MethodRouter<AppState>,
}

inventory::collect!(SliceDescriptor);

/// 收集并挂载所有处于 `Active` 状态的切片路由；`Planned` 切片在生产构建中自动屏蔽
pub fn collect_active_routes(state: AppState) -> Router {
    let mut by_path: BTreeMap<&'static str, MethodRouter<AppState>> = BTreeMap::new();
    for descriptor in inventory::iter::<SliceDescriptor> {
        if descriptor.status == SliceStatus::Active {
            let method_router = (descriptor.route)();
            by_path
                .entry(descriptor.path)
                .and_modify(|existing| {
                    *existing = std::mem::take(existing).merge(method_router.clone());
                })
                .or_insert(method_router);
        }
    }

    let mut router = Router::new();
    for (path, method_router) in by_path {
        router = router.route(path, method_router);
    }
    router.with_state(state)
}
