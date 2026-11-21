# FlashMart Web 设计规范

面向桌面端与移动端 Web 浏览器（Desktop & Mobile Web），服务限时限量好物抢购的下单结算与个人订单管理。整体以 Teenage Engineering 与 Vercel Store 的精密工具感为视觉参照：依托中性灰表面明暗分层、等宽数字（tabular numerals）与单一高饱和抢购强调色建立信息秩序，避免装饰性渐变与营销弹窗噪音。

## 设计原则

- **秩序优先于装饰**：依赖字阶对比、1px 细分割线与表面明暗层级划分信息区域，不使用大面积纯色背景块或花哨插画填充。
- **抢购信号高信噪比**：倒计时、剩余库存、限购件数与支付保留期统一采用等宽数字与强对比状态色，确保买家在 1 秒内看清「能否买、还剩几件、剩多久」。
- **单强调色克制**：全站仅保留单一抢购主强调色（用于核心主按钮与进行中状态焦点），其余信息保持中性灰度体系，语义状态色（成功/警告/危险）仅用于订单状态与错误提示。
- **可控交互与防误触**：写操作按钮在请求期间显式进入加载禁用态，下单、支付与取消等关键状态转换提供明确反馈，取消已支付或待支付订单前需经二次确认。

## 色彩与表面材质

- 令牌事实源：[tokens.css](file:///Users/sony/RustroverProjects/taitide-labs/flashmart/web/src/styles/tokens.css) 与 [tokens.ts](file:///Users/sony/RustroverProjects/taitide-labs/flashmart/web/src/ui/tokens.ts)；下文仅引用语义令牌名。
- 色阶与强调色：
  - 表面层级从底层至浮层依次递进：`--bg-canvas`（画布底色） -> `--bg-surface`（卡片主表面） -> `--bg-subtle`（内凹参数槽与次级区） -> `--bg-elevated`（浮层/弹窗），亮暗双主题保持同构键值。
  - 文字三级对比：`--text-primary`（主标题与关键数值）、`--text-secondary`（辅助说明与标签）、`--text-muted`（占位与次要元信息）。
  - 主强调色 `--accent-primary` 仅用于「立即抢购 / 确认结算 / 当前选中导航」；状态色 `--status-success`、`--status-warning`、`--status-danger` 仅作用于库存告警、订单状态标签与 RFC 9457 错误提示。
- 表面材质：
  - 亮色主题采用高通透冷调中性底色与 1px `--border-default` 锐利描边，配合微弱顶光高光 `--shadow-surface` 呈现精密硬件面板质感；
  - 暗色主题采用深炭黑画布搭配抬升灰阶卡片与半透明 1px 顶部高光边框 `--border-highlight`，浮层启用 `--bg-glass` 毛玻璃背景与 `--shadow-elevated`。

## 排版、空间与多端适配

- 字体与排版：正文采用无衬线系统字体族 `--font-sans`；商品单价、剩余库存、倒计时与订单号/版本号统一使用 `--font-mono` 并开启 `font-variant-numeric: tabular-nums`，杜绝数字跳动导致布局抖动。中英文混排通过字号微调与 `letter-spacing` 保持基线对齐。
- 空间与对齐：采用 4px 基准空间步进令牌（`--space-1` 至 `--space-8`）。严格遵守**内边距归组件自身、元素间距归父容器**，杜绝双倍留白；卡片与内部按钮采用同心圆角（外层 `--radius-lg`，内层控件 `--radius-md` / `--radius-sm`）。
- 跨平台家族映射：
  - 桌面端（`>= 768px`）：顶部常驻单行导航壳层（品牌标识 + `限时抢购` / `我的订单` + 买家切换器 + 主题切换），商品网格采用 2–3 列自适应排布，控件保持紧凑桌面指针密度。
  - 移动端（`< 768px`）：商品网格收敛为单列全宽卡片，可交互按钮与输入框最小高度满足 `44px` 触控热区，顶栏导航自动折叠为双层或紧凑横排标签，弹窗在小屏视口内保持安全边距与可滚动内容区。

## 共享控件目录

所有受控组件统一沉淀于 [ui](file:///Users/sony/RustroverProjects/taitide-labs/flashmart/web/src/ui) 目录，业务页面严禁硬编码样式或绕过 `ui/` 编写原生控件：

- 布局原语：`Container`、`Stack`、`Cluster`、`Grid`、`Surface`（仅通过 `gap` / `padding` 语义属性控制间距）
- 基础交互：`Button`（`variant`: `primary` | `secondary` | `ghost` | `danger`；`size`: `sm` | `md` | `lg`；支持 `loading` / `disabled` 与统一外置 `:focus-visible` 焦点环）
- 表单与输入：`Input`（内置 `label`、`hint`、`error` 与 `aria-invalid` 关联）、`NumberStepper`（受控加减步进器，遵循 min/max 边界禁用态）
- 展示与反馈：`Badge`（`neutral` | `accent` | `success` | `warning` | `danger`）、`InlineAlert`（呈现 RFC 9457 错误与操作反馈）、`EmptyState`、`Skeleton`
- 导航与浮层：`AppShell`（顶栏导航 + 买家标识入口 + 主题切换）、`Tabs`、`Dialog`（支持 `Esc` 关闭、遮罩关闭、ARIA `role="dialog"` 与焦点保持）

## 页面拓扑、命令与文案

- 壳层与页面地图：
  - `/drops`（默认首页 `/`）：限时抢购商品大厅，支持按活动状态筛选（全部 / 抢购中 / 即将开抢 / 已结束）、查看实时库存与倒计时、选品进入确认下单弹窗或详情页 `/drops/:itemId`。
  - `/drops/:itemId`：单品抢购详情页，展示完整描述、品类标记、单人限购说明、倒计时与直接抢购操作。
  - `/orders`：我的订单列表与状态页，按创建时间倒序展示当前 `BuyerIdentity` 名下全部抢购订单，支持在支付保留期内点击「确认结算…」、对「待支付 / 已支付」订单执行「取消订单…」（实时触发退库回流）。
  - `/gallery`：开发与验收用全状态展厅，集中呈现全部 `ui/` 控件变体、亮暗双主题与边界态快照。
- 命令与文案规范：
  - 同义动作文案全站统一（「立即抢购」「确认结算」「取消订单」），会打开二次确认弹窗或切换浮层的按钮统一带省略号（如 `切换买家…`、`取消订单…`）。
  - 界面文案保持经济克制：直接呈现业务事实与明确下一步动作（如库存不足时提示"库存已抢光，可关注取消回流"）。

## 动效清单

- 微交互反馈：`Button` 悬停/按压与表面边框过渡使用 `--duration-fast`（`120ms`）+ `--ease-out`；
- 浮层进出：`Dialog` 遮罩与面板入场使用 `--duration-normal`（`180ms`）+ `--ease-out`；
- 加载占位：`Skeleton` 使用低对比度脉冲动画；
- 降级策略：在 `@media (prefers-reduced-motion: reduce)` 下将所有过渡与动画时长收敛为 `0ms` / 禁用位移。

## 推荐与禁止

- **推荐**：在价格、余量、倒计时与订单版本号上统一启用 `tabular-nums` 等宽数字；在每次下单/支付/取消后主动同步商品库存视图，让退库回流即时可见。
- **禁止**：禁止在页面层硬编码十六进制色值或内联 `style`；禁止使用虚假占位图或无意义的装饰徽章；禁止把错误原因隐藏为模糊的「操作失败」，必须完整透传 RFC 9457 业务原因。
