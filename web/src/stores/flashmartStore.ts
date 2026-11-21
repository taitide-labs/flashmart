import { useSyncExternalStore } from 'react';
import { FlashMartClient } from '../api/client';
import {
  isApiProblemError,
  type FlashItemSummary,
  type FlashOrderView,
} from '../api/types';
import type { ThemeMode } from '../ui';

export interface FeedbackMessage {
  tone: 'info' | 'success' | 'warning' | 'danger';
  title: string;
  description: string;
}

export interface FlashMartState {
  buyerId: string;
  theme: ThemeMode;
  items: FlashItemSummary[];
  itemsLoading: boolean;
  itemsError: string | null;
  selectedItem: FlashItemSummary | null;
  selectedItemLoading: boolean;
  selectedItemError: string | null;
  orders: FlashOrderView[];
  ordersLoading: boolean;
  ordersError: string | null;
  placingItemId: string | null;
  mutatingOrderIds: Record<string, boolean>;
  feedback: FeedbackMessage | null;
  serverTimeOffsetMs: number;
}

export interface StoreDeps {
  client: FlashMartClient;
  now?: () => number;
  nextId?: () => string;
  initialBuyerId?: string;
  initialTheme?: ThemeMode;
}

const BUYER_STORAGE_KEY = 'flashmart.buyer_id';
const THEME_STORAGE_KEY = 'flashmart.theme';

export function createFlashMartStore(deps: StoreDeps) {
  const now = deps.now ?? (() => Date.now());
  const nextId =
    deps.nextId ??
    (() =>
      typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
        ? crypto.randomUUID()
        : `key-${now()}-${Math.random().toString(16).slice(2)}`);

  let state: FlashMartState = {
    buyerId: deps.initialBuyerId ?? readStorage(BUYER_STORAGE_KEY) ?? 'buyer-demo',
    theme: deps.initialTheme ?? readThemeStorage(),
    items: [],
    itemsLoading: false,
    itemsError: null,
    selectedItem: null,
    selectedItemLoading: false,
    selectedItemError: null,
    orders: [],
    ordersLoading: false,
    ordersError: null,
    placingItemId: null,
    mutatingOrderIds: {},
    feedback: null,
    serverTimeOffsetMs: 0,
  };

  applyThemeToDom(state.theme);

  const listeners = new Set<() => void>();
  let itemsReqSeq = 0;
  let selectedItemReqSeq = 0;
  let ordersReqSeq = 0;
  let itemsAbort: AbortController | null = null;
  let selectedItemAbort: AbortController | null = null;
  let ordersAbort: AbortController | null = null;

  function emit(next: Partial<FlashMartState>) {
    state = { ...state, ...next };
    for (const listener of listeners) {
      listener();
    }
  }

  function syncServerOffset(serverTimeIso: string) {
    const parsed = Date.parse(serverTimeIso);
    if (!Number.isNaN(parsed)) {
      emit({ serverTimeOffsetMs: parsed - now() });
    }
  }

  return {
    getSnapshot(): FlashMartState {
      return state;
    },

    subscribe(listener: () => void): () => void {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },

    getServerNowMs(): number {
      return now() + state.serverTimeOffsetMs;
    },

    clearFeedback(): void {
      emit({ feedback: null });
    },

    setTheme(theme: ThemeMode): void {
      writeStorage(THEME_STORAGE_KEY, theme);
      applyThemeToDom(theme);
      emit({ theme });
    },

    toggleTheme(): void {
      const next: ThemeMode = state.theme === 'light' ? 'dark' : 'light';
      this.setTheme(next);
    },

    async switchBuyer(nextBuyerId: string): Promise<void> {
      const trimmed = nextBuyerId.trim();
      writeStorage(BUYER_STORAGE_KEY, trimmed);
      emit({
        buyerId: trimmed,
        orders: [],
        ordersError: null,
        feedback: {
          tone: 'info',
          title: '已切换当前买家',
          description: `当前订单列表与限购占用将按买家标识 ${trimmed} 计算。`,
        },
      });
      await this.loadOrders();
    },

    async loadItems(): Promise<void> {
      itemsAbort?.abort();
      itemsAbort = new AbortController();
      const seq = ++itemsReqSeq;
      emit({ itemsLoading: true, itemsError: null });

      try {
        const res = await deps.client.listFlashItems(itemsAbort.signal);
        if (seq !== itemsReqSeq) return;
        syncServerOffset(res.server_time);
        emit({ items: res.items, itemsLoading: false, itemsError: null });
      } catch (error) {
        if (isAbortError(error) || seq !== itemsReqSeq) return;
        emit({
          itemsLoading: false,
          itemsError: formatErrorText(error, '加载抢购商品列表失败'),
        });
      }
    },

    async loadItemDetail(itemId: string): Promise<void> {
      selectedItemAbort?.abort();
      selectedItemAbort = new AbortController();
      const seq = ++selectedItemReqSeq;
      emit({ selectedItemLoading: true, selectedItemError: null });

      try {
        const item = await deps.client.getFlashItem(itemId, selectedItemAbort.signal);
        if (seq !== selectedItemReqSeq) return;
        syncServerOffset(item.server_time);
        emit({
          selectedItem: item,
          selectedItemLoading: false,
          selectedItemError: null,
        });
      } catch (error) {
        if (isAbortError(error) || seq !== selectedItemReqSeq) return;
        emit({
          selectedItem: null,
          selectedItemLoading: false,
          selectedItemError: formatErrorText(error, '加载商品详情失败'),
        });
      }
    },

    async loadOrders(): Promise<void> {
      ordersAbort?.abort();
      ordersAbort = new AbortController();
      const seq = ++ordersReqSeq;
      const buyer = state.buyerId;
      emit({ ordersLoading: true, ordersError: null });

      try {
        const res = await deps.client.listOrders(buyer, ordersAbort.signal);
        if (seq !== ordersReqSeq || buyer !== state.buyerId) return;
        syncServerOffset(res.server_time);
        emit({ orders: res.orders, ordersLoading: false, ordersError: null });
      } catch (error) {
        if (isAbortError(error) || seq !== ordersReqSeq) return;
        emit({
          ordersLoading: false,
          ordersError: formatErrorText(error, '加载个人订单失败'),
        });
      }
    },

    async placeOrder(itemId: string, quantity: number): Promise<FlashOrderView | null> {
      if (state.placingItemId) return null;
      const buyer = state.buyerId;
      emit({ placingItemId: itemId, feedback: null });

      try {
        const order = await deps.client.placeOrder(
          buyer,
          { item_id: itemId, quantity },
          nextId(),
        );
        emit({
          placingItemId: null,
          feedback: {
            tone: 'success',
            title: '库存预占成功，已生成待支付订单',
            description: '请在支付保留期内完成模拟结算，超时未付将自动释放库存。',
          },
        });
        await Promise.all([this.loadItems(), this.loadOrders()]);
        if (state.selectedItem?.id === itemId) {
          await this.loadItemDetail(itemId);
        }
        return order;
      } catch (error) {
        emit({
          placingItemId: null,
          feedback: {
            tone: 'danger',
            title: isApiProblemError(error) ? error.problem.title : '下单失败',
            description: formatErrorText(error, '抢购下单未成功，请刷新库存后再试'),
          },
        });
        await this.loadItems();
        if (state.selectedItem?.id === itemId) {
          await this.loadItemDetail(itemId);
        }
        return null;
      }
    },

    async payOrder(order: FlashOrderView): Promise<FlashOrderView | null> {
      if (state.mutatingOrderIds[order.id]) return null;
      emit({
        mutatingOrderIds: { ...state.mutatingOrderIds, [order.id]: true },
        feedback: null,
      });

      try {
        const updated = await deps.client.payOrder(
          state.buyerId,
          order.id,
          { version: order.version },
          nextId(),
        );
        emit({
          mutatingOrderIds: omitKey(state.mutatingOrderIds, order.id),
          feedback: {
            tone: 'success',
            title: '模拟结算成功',
            description: updated.fulfillment_hint ?? '订单已支付并完成库存扣减确认。',
          },
        });
        await Promise.all([this.loadOrders(), this.loadItems()]);
        return updated;
      } catch (error) {
        emit({
          mutatingOrderIds: omitKey(state.mutatingOrderIds, order.id),
          feedback: {
            tone: 'danger',
            title: isApiProblemError(error) ? error.problem.title : '支付失败',
            description: formatErrorText(error, '订单结算失败，请刷新状态后重试'),
          },
        });
        await Promise.all([this.loadOrders(), this.loadItems()]);
        return null;
      }
    },

    async cancelOrder(order: FlashOrderView): Promise<FlashOrderView | null> {
      if (state.mutatingOrderIds[order.id]) return null;
      emit({
        mutatingOrderIds: { ...state.mutatingOrderIds, [order.id]: true },
        feedback: null,
      });

      try {
        const updated = await deps.client.cancelOrder(
          state.buyerId,
          order.id,
          { version: order.version },
          nextId(),
        );
        emit({
          mutatingOrderIds: omitKey(state.mutatingOrderIds, order.id),
          feedback: {
            tone: 'info',
            title: '订单已取消，库存已退回',
            description: updated.fulfillment_hint ?? '库存已按抢购时间窗规则归还账本。',
          },
        });
        await Promise.all([this.loadOrders(), this.loadItems()]);
        return updated;
      } catch (error) {
        emit({
          mutatingOrderIds: omitKey(state.mutatingOrderIds, order.id),
          feedback: {
            tone: 'danger',
            title: isApiProblemError(error) ? error.problem.title : '取消订单失败',
            description: formatErrorText(error, '取消订单未成功，请刷新状态后重试'),
          },
        });
        await Promise.all([this.loadOrders(), this.loadItems()]);
        return null;
      }
    },
  };
}

export type FlashMartStore = ReturnType<typeof createFlashMartStore>;

export function useFlashMartStore(store: FlashMartStore): FlashMartState {
  return useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
}

function formatErrorText(error: unknown, fallback: string): string {
  if (isApiProblemError(error)) {
    return error.problem.detail || error.problem.title;
  }
  if (error instanceof Error && error.message) {
    return error.message;
  }
  return fallback;
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === 'AbortError';
}

function omitKey(record: Record<string, boolean>, target: string): Record<string, boolean> {
  const next = { ...record };
  delete next[target];
  return next;
}

function readStorage(key: string): string | null {
  try {
    return typeof localStorage !== 'undefined' ? localStorage.getItem(key) : null;
  } catch {
    return null;
  }
}

function writeStorage(key: string, value: string): void {
  try {
    if (typeof localStorage !== 'undefined') {
      localStorage.setItem(key, value);
    }
  } catch {
    // 忽略无痕模式存储受限
  }
}

function readThemeStorage(): ThemeMode {
  const stored = readStorage(THEME_STORAGE_KEY);
  return stored === 'dark' ? 'dark' : 'light';
}

function applyThemeToDom(theme: ThemeMode): void {
  if (typeof document !== 'undefined') {
    document.documentElement.dataset.theme = theme;
  }
}
