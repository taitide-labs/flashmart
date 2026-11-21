# 商品库存上下文

负责限量好物的基础信息、抢购时间窗及防超卖库存账本（预占、确认扣减、释放与退回），作为上游开放能力可复用于后续各类营销与售卖活动；纳入扁平商品目录默认支撑方案，暂不包含多规格属性矩阵与复杂类目树。

## 术语

**限量商品**：参与限时抢购的独立售卖标的，持有名称、品类标记（数字好物或实体好物）、售价、单人限购上限与抢购时间窗。
- 等价：`FlashItem`
- 避免：`Product`、`SKU`、`Goods`

**抢购时间窗**：限量商品开放购买的起止时间区间，决定商品处于未开始、进行中或已结束状态，仅进行中允许新的库存预占。
- 等价：`SaleWindow`
- 避免：`Schedule`、`Cron`

**库存账本**：记录某限量商品总配额、可售库存、已预占库存与已确认扣减库存的强一致配额记录，任意时刻可售库存不得为负。
- 等价：`InventoryLedger`
- 避免：`StockCache`、`Counter`

**库存预占**：为单笔抢购请求在库存账本中原子锁定的指定件数临时配额凭证，在确认扣减或释放之前该部分库存不可被其他买家抢占。
- 等价：`StockReservation`
- 避免：`Lock`、`CartItem`

**库存回流**：因下单失败补偿、待支付取消、超时过期或已支付取消而释放/退回库存时，将配额归还库存账本；若此时仍处于抢购时间窗内，归还件数实时恢复为可售库存，否则仅记账归还不再开售。
- 等价：`StockRestock`
- 避免：`Rollback`、`Retry`

## Workflow

- `ReserveStock`：`ReserveStockCommand` -> `StockReserved` / `StockReservationRejected`
- `ConfirmStockDeduction`：`ConfirmStockDeductionCommand` -> `StockDeductionConfirmed`
- `ReleaseReservedStock`：`ReleaseReservedStockCommand` -> `ReservedStockReleased`
- `ReturnDeductedStock`：`ReturnDeductedStockCommand` -> `DeductedStockReturned`
