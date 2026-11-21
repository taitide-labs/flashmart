# 订单结算上下文

负责面向买家的限时抢购下单资格判定、单品抢购订单生命周期、站内模拟结算以及订单取消/过期后的库存退回协调；纳入轻量买家标识与站内模拟结算通用域默认方案。

## 术语

**买家标识**：用于区分本人订单及判定单人限购占用的轻量访客或免密用户身份凭证。
- 等价：`BuyerIdentity`
- 避免：`Account`、`UserProfile`、`SessionToken`

**抢购订单**：买家针对单一限量商品发起下单后生成的履约单据，持有商品快照、品类标记、购买件数、应付金额、支付截止时刻与订单状态（待支付、已支付、已取消、已过期）。
- 等价：`FlashOrder`
- 避免：`Cart`、`CheckoutSession`

**有效限购占用**：同一买家在同一限量商品下处于「待支付」或「已支付」状态的订单总件数；订单变为「已取消」或「已过期」后立即解除占用，允许买家重新抢购。
- 等价：`ActiveQuotaUsage`
- 避免：`PurchaseHistory`、`Counter`

**支付保留期**：从抢购订单生成起算的固定有效付款时间长度（首期默认 15 分钟）；已在抢购时间窗内成功预占名额的订单，即使活动窗口在此期间结束，仍可在支付保留期内完成结算。
- 等价：`PaymentHoldDuration`
- 避免：`Timeout`、`TTL`

**模拟结算**：买家在支付保留期内触发的站内确认付款动作，将订单流转为「已支付」；当「已支付」订单被主动取消时同步执行模拟退款。
- 等价：`SimulatedSettlement`
- 避免：`PaymentGateway`、`Webhook`

## Workflow

- `PlaceFlashOrder`：`PlaceFlashOrderCommand` -> `FlashOrderPlaced` / `FlashOrderPlacementFailed`
- `PayFlashOrder`：`PayFlashOrderCommand` -> `FlashOrderPaid`
- `CancelFlashOrder`：`CancelFlashOrderCommand` -> `FlashOrderCancelled`
- `ExpireUnpaidOrder`：`ExpireUnpaidOrderCommand` -> `FlashOrderExpired`
