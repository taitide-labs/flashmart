# 采用按上下文隔离的 PostgreSQL Schema 与事务外同步端口补偿编排

Status: accepted

在 Modular Monolith 中，商品库存与订单结算共享同一 PostgreSQL 实例，但各自拥有独立的数据库 Schema（`inventory` 与 `ordering`），严禁跨 Schema 外键与 SQL 联表；下单、支付与退库编排严格在本地数据库事务外调用 `InventoryPort`，通过成对幂等释放/退回与 Transactional Outbox 保证最终一致。

## 备选方案

- 单一 `public` Schema 共享事务：可借助数据库外键与单事务同时更新库存表与订单表，实现最简强一致；但会破坏上下文自治边界，导致后续拆库或复用库存能力时无法解耦。

## 影响

- 下单主流程在执行 `reserve_stock` 后若本地事务落库失败（含并发限购冲突），必须执行 `release_reserved_stock` 逆向补偿，并辅以 Outbox 兜底。
- `ordering` 上下文在本地只读投影中保存商品标题、品类与单价快照，不联表查询 `inventory.flash_items`。
