import type { Transport, TransportRequest } from './client';
import {
  ApiProblemError,
  type FlashItemSummary,
  type FlashOrderView,
  type ItemKind,
  type OrderMutationRequest,
  type PlaceOrderRequest,
  type ProblemDetails,
  type WindowStatus,
} from './types';

export type StubScenario = 'normal' | 'empty' | 'network_error';

interface SeedItem {
  id: string;
  title: string;
  description: string;
  item_kind: ItemKind;
  price_cents: number;
  total_stock: number;
  available_stock: number;
  per_user_limit: number;
  sale_starts_at_ms: number;
  sale_ends_at_ms: number;
}

interface StoredOrder extends FlashOrderView {
  reservation_id: string;
}

interface IdempotencyEntry {
  fingerprint: string;
  response: unknown;
}

export interface StubTransportOptions {
  clock?: () => number;
  scenario?: StubScenario;
  delayMs?: number;
  paymentHoldMs?: number;
}

const BUYER_ID_PATTERN = /^[A-Za-z0-9_-]{1,64}$/;
const DEFAULT_PAYMENT_HOLD_MS = 15 * 60 * 1000;

export class StubTransport implements Transport {
  private readonly clock: () => number;
  private readonly paymentHoldMs: number;
  private scenario: StubScenario;
  private delayMs: number;
  private seq = 1;
  private items: SeedItem[];
  private orders: StoredOrder[] = [];
  private idempotency = new Map<string, IdempotencyEntry>();

  constructor(options: StubTransportOptions = {}) {
    this.clock = options.clock ?? (() => Date.now());
    this.scenario = options.scenario ?? 'normal';
    this.delayMs = options.delayMs ?? 0;
    this.paymentHoldMs = options.paymentHoldMs ?? DEFAULT_PAYMENT_HOLD_MS;
    this.items = createDefaultItems(this.clock());
  }

  setScenario(scenario: StubScenario): void {
    this.scenario = scenario;
  }

  setDelayMs(delayMs: number): void {
    this.delayMs = delayMs;
  }

  reset(): void {
    this.seq = 1;
    this.items = createDefaultItems(this.clock());
    this.orders = [];
    this.idempotency.clear();
  }

  async request<T>(req: TransportRequest): Promise<T> {
    if (this.delayMs > 0) {
      await waitWithAbort(this.delayMs, req.signal);
    }
    if (req.signal?.aborted) {
      throw new DOMException('Aborted', 'AbortError');
    }

    if (this.scenario === 'network_error') {
      throw problemError(req.path, 0, 'network_error', '网络连接失败', '当前处于离线故障模拟模式，请恢复网络后重试。');
    }

    this.expireOverdueOrders();

    if (req.method === 'GET' && req.path === '/api/health') {
      return { status: 'ok' } as T;
    }

    if (req.method === 'GET' && req.path === '/api/flash-items') {
      if (this.scenario === 'empty') {
        return {
          items: [],
          server_time: new Date(this.clock()).toISOString(),
        } as T;
      }
      return {
        items: this.items.map((item) => this.toItemSummary(item)),
        server_time: new Date(this.clock()).toISOString(),
      } as T;
    }

    const itemMatch = /^\/api\/flash-items\/([^/]+)$/.exec(req.path);
    if (req.method === 'GET' && itemMatch) {
      const itemId = decodeURIComponent(itemMatch[1]);
      const item = this.items.find((entry) => entry.id === itemId);
      if (!item || this.scenario === 'empty') {
        throw problemError(req.path, 404, 'item_not_found', '商品不存在', `未找到商品 ${itemId}。`);
      }
      return this.toItemSummary(item) as T;
    }

    const buyerId = this.requireBuyerId(req.headers, req.path);

    if (req.method === 'GET' && req.path === '/api/orders') {
      if (this.scenario === 'empty') {
        return {
          orders: [],
          server_time: new Date(this.clock()).toISOString(),
        } as T;
      }
      const list = this.orders
        .filter((order) => order.buyer_id === buyerId)
        .sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at))
        .map(stripInternalOrder);
      return {
        orders: list,
        server_time: new Date(this.clock()).toISOString(),
      } as T;
    }

    const orderMatch = /^\/api\/orders\/([^/]+)$/.exec(req.path);
    if (req.method === 'GET' && orderMatch) {
      const orderId = decodeURIComponent(orderMatch[1]);
      const order = this.requireOwnedOrder(orderId, buyerId, req.path);
      return stripInternalOrder(order) as T;
    }

    const idempotencyKey = this.requireIdempotencyKey(req.headers, req.path);
    const idemScope = `${buyerId}:${req.method}:${req.path}:${idempotencyKey}`;
    const fingerprint = JSON.stringify(req.body ?? null);
    const cached = this.idempotency.get(idemScope);
    if (cached) {
      if (cached.fingerprint !== fingerprint) {
        throw problemError(
          req.path,
          409,
          'idempotency_key_conflict',
          '幂等键参数冲突',
          '相同的 Idempotency-Key 不能用于不同的请求参数。',
        );
      }
      return structuredClone(cached.response) as T;
    }

    if (req.method === 'POST' && req.path === '/api/orders') {
      const body = req.body as Partial<PlaceOrderRequest> | undefined;
      if (!body || typeof body.item_id !== 'string' || typeof body.quantity !== 'number' || !Number.isInteger(body.quantity) || body.quantity < 1) {
        throw problemError(
          req.path,
          422,
          'validation_error',
          '下单参数无效',
          '请检查商品编号与购买数量（需为正整数）。',
          [{ field: 'quantity', message: '购买数量必须大于等于 1' }],
        );
      }

      const item = this.items.find((entry) => entry.id === body.item_id);
      if (!item) {
        throw problemError(req.path, 404, 'item_not_found', '商品不存在', '所选限量商品不存在或已下架。');
      }

      const windowStatus = this.computeWindowStatus(item);
      if (windowStatus !== 'active') {
        throw problemError(
          req.path,
          409,
          'sale_window_not_active',
          '不在抢购时间窗内',
          windowStatus === 'upcoming' ? '该商品尚未到开抢时间，请耐心等待倒计时结束。' : '本场限时抢购已结束，暂不可继续下单。',
        );
      }

      const activeQuantity = this.orders
        .filter(
          (order) =>
            order.buyer_id === buyerId &&
            order.item_id === item.id &&
            (order.status === 'pending_payment' || order.status === 'paid'),
        )
        .reduce((sum, order) => sum + order.quantity, 0);

      if (activeQuantity + body.quantity > item.per_user_limit) {
        throw problemError(
          req.path,
          409,
          'purchase_limit_exceeded',
          '超出单人限购上限',
          `每人限购 ${item.per_user_limit} 件，您当前已持有有效订单（取消或过期后可释放限购名额）。`,
        );
      }

      if (item.available_stock < body.quantity) {
        throw problemError(
          req.path,
          409,
          'insufficient_stock',
          '剩余库存不足',
          '手慢一步，当前可售库存不足；如有买家取消或超时未付，库存将实时回流。',
        );
      }

      item.available_stock -= body.quantity;
      const nowMs = this.clock();
      const orderId = this.nextUuid('ord');
      const created: StoredOrder = {
        id: orderId,
        reservation_id: this.nextUuid('res'),
        buyer_id: buyerId,
        item_id: item.id,
        item_title: item.title,
        item_kind: item.item_kind,
        quantity: body.quantity,
        unit_price_cents: item.price_cents,
        total_amount_cents: item.price_cents * body.quantity,
        currency: 'CNY',
        status: 'pending_payment',
        version: 1,
        payment_deadline: new Date(nowMs + this.paymentHoldMs).toISOString(),
        created_at: new Date(nowMs).toISOString(),
        paid_at: null,
        cancelled_at: null,
        expired_at: null,
        fulfillment_hint: buildFulfillmentHint(item.item_kind, 'pending_payment', orderId),
      };
      this.orders.push(created);
      const view = stripInternalOrder(created);
      this.idempotency.set(idemScope, { fingerprint, response: structuredClone(view) });
      return view as T;
    }

    const payMatch = /^\/api\/orders\/([^/]+)\/pay$/.exec(req.path);
    if (req.method === 'POST' && payMatch) {
      const orderId = decodeURIComponent(payMatch[1]);
      const body = req.body as Partial<OrderMutationRequest> | undefined;
      const order = this.requireOwnedOrder(orderId, buyerId, req.path);
      this.assertValidVersion(body, order, req.path);

      if (order.status === 'expired') {
        throw problemError(req.path, 409, 'order_expired', '订单已超过支付保留期', '该订单超时未支付，预占库存已自动释放。');
      }
      if (order.status !== 'pending_payment') {
        throw problemError(req.path, 409, 'invalid_order_state', '订单状态不允许支付', `当前状态为 ${order.status}，仅待支付订单可执行结算。`);
      }

      const nowIso = new Date(this.clock()).toISOString();
      order.status = 'paid';
      order.version += 1;
      order.paid_at = nowIso;
      order.fulfillment_hint = buildFulfillmentHint(order.item_kind, 'paid', order.id);

      const view = stripInternalOrder(order);
      this.idempotency.set(idemScope, { fingerprint, response: structuredClone(view) });
      return view as T;
    }

    const cancelMatch = /^\/api\/orders\/([^/]+)\/cancel$/.exec(req.path);
    if (req.method === 'POST' && cancelMatch) {
      const orderId = decodeURIComponent(cancelMatch[1]);
      const body = req.body as Partial<OrderMutationRequest> | undefined;
      const order = this.requireOwnedOrder(orderId, buyerId, req.path);
      this.assertValidVersion(body, order, req.path);

      if (order.status !== 'pending_payment' && order.status !== 'paid') {
        throw problemError(req.path, 409, 'invalid_order_state', '订单状态不可取消', `当前状态为 ${order.status}，无法再次取消。`);
      }

      const wasPaid = order.status === 'paid';
      order.status = 'cancelled';
      order.version += 1;
      order.cancelled_at = new Date(this.clock()).toISOString();
      order.fulfillment_hint = wasPaid ? '已取消并完成模拟原路退款，库存已退回。' : '已主动取消，预占库存已释放。';

      this.restockIfWindowActive(order.item_id, order.quantity);

      const view = stripInternalOrder(order);
      this.idempotency.set(idemScope, { fingerprint, response: structuredClone(view) });
      return view as T;
    }

    throw problemError(req.path, 404, 'route_not_found', '接口不存在', `未匹配到契约路由 ${req.method} ${req.path}`);
  }

  private expireOverdueOrders(): void {
    const nowMs = this.clock();
    for (const order of this.orders) {
      if (order.status === 'pending_payment' && nowMs > Date.parse(order.payment_deadline)) {
        order.status = 'expired';
        order.version += 1;
        order.expired_at = new Date(nowMs).toISOString();
        order.fulfillment_hint = '超过支付保留期未付款，预占库存已自动回流。';
        this.restockIfWindowActive(order.item_id, order.quantity);
      }
    }
  }

  private restockIfWindowActive(itemId: string, quantity: number): void {
    const item = this.items.find((entry) => entry.id === itemId);
    if (!item) return;
    if (this.computeWindowStatus(item) === 'active') {
      item.available_stock = Math.min(item.total_stock, item.available_stock + quantity);
    }
  }

  private computeWindowStatus(item: SeedItem): WindowStatus {
    const now = this.clock();
    if (now < item.sale_starts_at_ms) return 'upcoming';
    if (now > item.sale_ends_at_ms) return 'ended';
    return 'active';
  }

  private toItemSummary(item: SeedItem): FlashItemSummary {
    const nowIso = new Date(this.clock()).toISOString();
    return {
      id: item.id,
      title: item.title,
      description: item.description,
      item_kind: item.item_kind,
      price_cents: item.price_cents,
      currency: 'CNY',
      total_stock: item.total_stock,
      available_stock: item.available_stock,
      per_user_limit: item.per_user_limit,
      sale_starts_at: new Date(item.sale_starts_at_ms).toISOString(),
      sale_ends_at: new Date(item.sale_ends_at_ms).toISOString(),
      window_status: this.computeWindowStatus(item),
      server_time: nowIso,
    };
  }

  private requireBuyerId(headers: Record<string, string> | undefined, path: string): string {
    const raw = headers?.['X-Buyer-Id']?.trim();
    if (!raw) {
      throw problemError(path, 400, 'missing_required_header', '缺少买家标识', '请求头必须携带 X-Buyer-Id。');
    }
    if (!BUYER_ID_PATTERN.test(raw)) {
      throw problemError(
        path,
        422,
        'validation_error',
        '买家标识格式无效',
        'X-Buyer-Id 仅支持 1-64 位字母、数字、下划线或连字符。',
        [{ field: 'X-Buyer-Id', message: '格式需匹配 ^[A-Za-z0-9_-]{1,64}$' }],
      );
    }
    return raw;
  }

  private requireIdempotencyKey(headers: Record<string, string> | undefined, path: string): string {
    const raw = headers?.['Idempotency-Key']?.trim();
    if (!raw || raw.length > 128) {
      throw problemError(path, 400, 'missing_required_header', '缺少幂等键', '写请求必须携带有效 Idempotency-Key。');
    }
    return raw;
  }

  private requireOwnedOrder(orderId: string, buyerId: string, path: string): StoredOrder {
    const order = this.orders.find((entry) => entry.id === orderId);
    if (!order) {
      throw problemError(path, 404, 'order_not_found', '订单不存在', `未找到订单 ${orderId}。`);
    }
    if (order.buyer_id !== buyerId) {
      throw problemError(path, 403, 'order_ownership_mismatch', '无权操作他人订单', '当前买家标识与订单归属不一致。');
    }
    return order;
  }

  private assertValidVersion(
    body: Partial<OrderMutationRequest> | undefined,
    order: StoredOrder,
    path: string,
  ): void {
    if (!body || typeof body.version !== 'number' || !Number.isInteger(body.version) || body.version < 1) {
      throw problemError(path, 422, 'validation_error', '缺少有效版本号', '请求体必须传入正整数 version。');
    }
    if (body.version !== order.version) {
      throw problemError(
        path,
        409,
        'order_version_conflict',
        '订单状态已发生并发变更',
        '订单版本号已过期，请刷新订单最新状态后再试。',
      );
    }
  }

  private nextUuid(prefix: string): string {
    const n = String(this.seq++).padStart(12, '0');
    const tag = prefix === 'ord' ? 'a000' : 'b000';
    return `00000000-0000-4000-${tag}-${n}`;
  }
}

function stripInternalOrder(order: StoredOrder): FlashOrderView {
  const { reservation_id: _unused, ...view } = order;
  return structuredClone(view);
}

function buildFulfillmentHint(kind: ItemKind, status: 'pending_payment' | 'paid', orderId: string): string {
  if (status === 'pending_payment') {
    return kind === 'digital' ? '完成模拟结算后即刻生成数字兑换凭证' : '完成模拟结算后进入限量实体待配货队列';
  }
  const suffix = orderId.slice(-6).toUpperCase();
  return kind === 'digital' ? `模拟数字兑换码：FM-DIGI-${suffix}` : `模拟配货单号：FM-PKG-${suffix}（等待出库）`;
}

function problemError(
  instance: string,
  status: number,
  code: string,
  title: string,
  detail: string,
  errors?: ProblemDetails['errors'],
): ApiProblemError {
  return new ApiProblemError({
    type: `urn:flashmart:problem:${code}`,
    title,
    status,
    detail,
    instance,
    code,
    errors,
  });
}

function waitWithAbort(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(new DOMException('Aborted', 'AbortError'));
      return;
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener('abort', onAbort);
      resolve();
    }, ms);
    const onAbort = () => {
      clearTimeout(timer);
      reject(new DOMException('Aborted', 'AbortError'));
    };
    signal?.addEventListener('abort', onAbort, { once: true });
  });
}

function createDefaultItems(nowMs: number): SeedItem[] {
  const min = 60 * 1000;
  return [
    {
      id: '11111111-1111-4111-8111-111111111101',
      title: 'FlashMart 创世数字徽章 · Pro Pass',
      description: '含独立编号与社区抢先体验资格的限量数字通行证，下单预占库存，结算后即刻交付凭证。',
      item_kind: 'digital',
      price_cents: 9900,
      total_stock: 10,
      available_stock: 3,
      per_user_limit: 1,
      sale_starts_at_ms: nowMs - 15 * min,
      sale_ends_at_ms: nowMs + 95 * min,
    },
    {
      id: '11111111-1111-4111-8111-111111111102',
      title: 'CNC 阳极氧化铝机械旋钮宏键盘',
      description: '实体好物限量批次，工业橙旋钮搭配静音线性轴，支持单人限购 1 台与取消退库回流。',
      item_kind: 'physical',
      price_cents: 49900,
      total_stock: 5,
      available_stock: 2,
      per_user_limit: 1,
      sale_starts_at_ms: nowMs - 8 * min,
      sale_ends_at_ms: nowMs + 48 * min,
    },
    {
      id: '11111111-1111-4111-8111-111111111103',
      title: '午夜合成器 WAV 采样母带包',
      description: '数字无损音频采样合集，本场首批已售罄，若有待支付超时或主动取消将实时回流。',
      item_kind: 'digital',
      price_cents: 5900,
      total_stock: 12,
      available_stock: 0,
      per_user_limit: 1,
      sale_starts_at_ms: nowMs - 30 * min,
      sale_ends_at_ms: nowMs + 60 * min,
    },
    {
      id: '11111111-1111-4111-8111-111111111104',
      title: '联名钛金属 EDC 随身钥匙扣',
      description: '整块五级钛 CNC 铣削成型，即将开放限时抢购，倒计时结束后可发起原子预占。',
      item_kind: 'physical',
      price_cents: 18900,
      total_stock: 8,
      available_stock: 8,
      per_user_limit: 1,
      sale_starts_at_ms: nowMs + 25 * min,
      sale_ends_at_ms: nowMs + 145 * min,
    },
    {
      id: '11111111-1111-4111-8111-111111111105',
      title: '首发工业贴纸与编号卡册套装',
      description: '首场测试批次已于早些时候截单，仅保留账面展示不再开放抢购。',
      item_kind: 'physical',
      price_cents: 3900,
      total_stock: 15,
      available_stock: 1,
      per_user_limit: 1,
      sale_starts_at_ms: nowMs - 180 * min,
      sale_ends_at_ms: nowMs - 20 * min,
    },
  ];
}
