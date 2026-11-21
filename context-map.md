# 上下文地图

## 限界上下文

- [商品库存上下文](./crates/contexts/inventory/context.md) — 支撑域与核心域（限量商品目录及防超卖库存账本，可复用于后续营销与售卖活动）
- [订单结算上下文](./crates/contexts/ordering/context.md) — 核心域与通用域默认集成（单品抢购下单、限购校验、模拟结算与取消过期退库协调）

## 关系模式

- 商品库存上下文 [OHS, PL] -> 订单结算上下文 [ACL]
  - 全局边界与非确定性抽象：跨上下文协作与时间/ID 抽象统一收敛于 `contracts` 模块（含 `Clock`、`IdGenerator`），严禁跨上下文直接引用内部领域模型或私有状态
  - 同步端口（`InventoryPort`）：
    - `list_flash_items` / `get_flash_item`：查询商品快照与实时可售余量
    - `reserve_stock` <-> 成对幂等释放 `release_reserved_stock`：下单前在本地数据库事务外原子预占库存；若下游成单落库失败、待支付主动取消或超时过期，调用成对方法幂等释放预占
    - `confirm_stock_deduction` <-> 成对幂等退回 `return_deducted_stock`：支付确认后将预占核销为已售出；若已支付订单主动取消，调用成对方法幂等退回已扣库存
  - 编排与写防线约束：
    - 严禁在持有活跃数据库事务期间调用 `InventoryPort`，下单主流程严格遵循「事务外 `reserve_stock` -> 本地短事务落库（含限购与 Outbox） -> 失败逆向调用 `release_reserved_stock`」
    - 南北向写接口统一要求 `Idempotency-Key`（按 `(buyer_id, endpoint, idempotency_key)` 判重与载荷冲突检测），状态变更统一基于订单 `version` 执行 OCC 乐观锁校验
  - 集成事件（`IntegrationEvent`，开放枚举 + 下游 `Unknown` 兜底 + Transactional Outbox）：
    - 上游发布：`StockReserved`、`StockDeductionConfirmed`、`ReservedStockReleased`、`DeductedStockReturned`
    - 下游发布：`FlashOrderPlaced`、`FlashOrderPaid`、`FlashOrderCancelled`、`FlashOrderExpired`（作为异步补偿与状态确认兜底，驱动库存核销/释放/退回最终一致）

## 首期 Walking Skeleton 闭环

- 预置包含数字好物与实体好物的限量商品及抢购时间窗、初始库存配额
- 买家凭轻量买家标识浏览限量商品、活动状态与实时剩余库存
- 在抢购时间窗内完成单品直购（限购校验 -> 原子预占库存 -> 生成待支付订单），高并发争抢时严禁超卖并给出明确失败反馈
- 买家查看个人订单列表与状态，在支付保留期内完成模拟结算，或主动取消待支付/已支付订单（及超时未付自动过期）并实时验证库存回流
