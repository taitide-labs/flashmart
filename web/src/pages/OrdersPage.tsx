import { RefreshCw } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import type { FlashOrderView, OrderStatus } from '../api/types';
import { useFlashMartStore, type FlashMartStore } from '../stores/flashmartStore';
import {
  Badge,
  Button,
  Cluster,
  Dialog,
  Divider,
  EmptyState,
  InlineAlert,
  LinkButton,
  SkeletonCard,
  Stack,
  Surface,
  Tabs,
  Text,
} from '../ui';
import {
  formatCountdown,
  formatPriceCents,
  itemKindLabel,
  orderStatusMeta,
} from '../utils/format';

type OrderFilter = 'all' | OrderStatus;

export function OrdersPage({ store }: { store: FlashMartStore }) {
  const state = useFlashMartStore(store);
  const [filter, setFilter] = useState<OrderFilter>('all');
  const [nowTick, setNowTick] = useState(() => store.getServerNowMs());
  const [cancelTarget, setCancelTarget] = useState<FlashOrderView | null>(null);

  useEffect(() => {
    void store.loadOrders();
  }, [store]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      setNowTick(store.getServerNowMs());
    }, 1000);
    return () => window.clearInterval(timer);
  }, [store]);

  const counts = useMemo(() => {
    const map: Record<OrderFilter, number> = {
      all: state.orders.length,
      pending_payment: 0,
      paid: 0,
      cancelled: 0,
      expired: 0,
    };
    for (const order of state.orders) {
      map[order.status] += 1;
    }
    return map;
  }, [state.orders]);

  const visibleOrders = useMemo(() => {
    if (filter === 'all') return state.orders;
    return state.orders.filter((order) => order.status === filter);
  }, [state.orders, filter]);

  async function handleConfirmCancel() {
    if (!cancelTarget) return;
    await store.cancelOrder(cancelTarget);
    setCancelTarget(null);
  }

  return (
    <Stack gap="6">
      <Cluster justify="between" align="start" gap="4">
        <Stack gap="1">
          <Text variant="page-title" as="h1">
            我的订单
          </Text>
          <Cluster gap="2">
            <Text variant="body" tone="secondary">
              当前买家标识：
            </Text>
            <Badge tone="neutral">{state.buyerId}</Badge>
          </Cluster>
        </Stack>
        <Button
          variant="secondary"
          size="sm"
          loading={state.ordersLoading}
          onClick={() => void store.loadOrders()}
        >
          <RefreshCw size={14} aria-hidden="true" />
          <span>刷新订单</span>
        </Button>
      </Cluster>

      {state.feedback ? (
        <InlineAlert
          tone={state.feedback.tone}
          title={state.feedback.title}
          description={state.feedback.description}
          action={
            <Button variant="ghost" size="sm" onClick={() => store.clearFeedback()}>
              知道了
            </Button>
          }
        />
      ) : null}

      <Tabs
        ariaLabel="订单状态筛选"
        value={filter}
        onChange={setFilter}
        items={[
          { value: 'all', label: '全部', count: counts.all },
          { value: 'pending_payment', label: '待支付', count: counts.pending_payment },
          { value: 'paid', label: '已支付', count: counts.paid },
          { value: 'cancelled', label: '已取消', count: counts.cancelled },
          { value: 'expired', label: '已过期', count: counts.expired },
        ]}
      />

      {state.ordersError ? (
        <InlineAlert
          tone="danger"
          title="订单列表加载失败"
          description={state.ordersError}
          action={
            <Button variant="secondary" size="sm" onClick={() => void store.loadOrders()}>
              重试
            </Button>
          }
        />
      ) : null}

      {state.ordersLoading && state.orders.length === 0 ? (
        <Stack gap="4">
          <SkeletonCard />
          <SkeletonCard />
        </Stack>
      ) : visibleOrders.length === 0 ? (
        <EmptyState
          title="当前买家暂无匹配订单"
          description="前往限时抢购大厅挑选限量好物，下单后可在此完成模拟结算或取消退回库存。"
          action={
            <LinkButton to="/drops" variant="primary">
              去逛逛抢购大厅
            </LinkButton>
          }
        />
      ) : (
        <Stack gap="4">
          {visibleOrders.map((order) => {
            const statusMeta = orderStatusMeta(order.status);
            const busy = Boolean(state.mutatingOrderIds[order.id]);
            const canPay = order.status === 'pending_payment';
            const canCancel =
              order.status === 'pending_payment' || order.status === 'paid';

            return (
              <Surface key={order.id} variant="card" padding="md">
                <Stack gap="3">
                  <Cluster justify="between" align="start" gap="2">
                    <Stack gap="1">
                      <Cluster gap="2">
                        <Badge tone="neutral">{itemKindLabel(order.item_kind)}</Badge>
                        <Badge tone={statusMeta.tone}>{statusMeta.label}</Badge>
                      </Cluster>
                      <Text variant="section-title" as="h2">
                        {order.item_title}
                      </Text>
                    </Stack>
                    <Stack gap="1">
                      <Text variant="mono-code" tone="muted">
                        单号 {order.id.slice(-8)} · v{order.version}
                      </Text>
                      <Text variant="mono-stat">
                        {formatPriceCents(order.total_amount_cents)}
                      </Text>
                    </Stack>
                  </Cluster>

                  <Surface variant="subtle" padding="sm">
                    <Cluster justify="between" align="center" gap="3">
                      <Text variant="body" tone="secondary">
                        数量：{order.quantity} 件（单价 {formatPriceCents(order.unit_price_cents)}）
                      </Text>
                      {order.status === 'pending_payment' ? (
                        <Text variant="mono-code" tone="warning">
                          支付保留剩余：{formatCountdown(order.payment_deadline, nowTick)}
                        </Text>
                      ) : order.fulfillment_hint ? (
                        <Text
                          variant="body"
                          tone={order.status === 'paid' ? 'success' : 'secondary'}
                        >
                          {order.fulfillment_hint}
                        </Text>
                      ) : null}
                    </Cluster>
                  </Surface>

                  {canPay || canCancel ? (
                    <>
                      <Divider />
                      <Cluster justify="end" gap="2">
                        {canCancel ? (
                          <Button
                            variant="danger"
                            size="sm"
                            disabled={busy}
                            onClick={() => setCancelTarget(order)}
                          >
                            {order.status === 'paid' ? '取消并退库…' : '取消订单…'}
                          </Button>
                        ) : null}
                        {canPay ? (
                          <Button
                            variant="primary"
                            size="sm"
                            loading={busy}
                            onClick={() => void store.payOrder(order)}
                          >
                            确认模拟结算
                          </Button>
                        ) : null}
                      </Cluster>
                    </>
                  ) : null}
                </Stack>
              </Surface>
            );
          })}
        </Stack>
      )}

      <Dialog
        open={Boolean(cancelTarget)}
        onClose={() => setCancelTarget(null)}
        title="确认取消订单"
        description={
          cancelTarget?.status === 'paid'
            ? '取消已支付订单将同步触发模拟退款，并把已扣减库存退回至库存账本。'
            : '取消待支付订单将立即释放预占库存，并解除该商品的限购占位。'
        }
        footer={
          <>
            <Button variant="ghost" onClick={() => setCancelTarget(null)}>
              再想想
            </Button>
            <Button
              variant="danger"
              loading={Boolean(cancelTarget && state.mutatingOrderIds[cancelTarget.id])}
              onClick={() => void handleConfirmCancel()}
            >
              确认取消并退库
            </Button>
          </>
        }
      >
        {cancelTarget ? (
          <Surface variant="subtle" padding="sm">
            <Stack gap="1">
              <Text variant="card-title">{cancelTarget.item_title}</Text>
              <Text variant="mono-code" tone="secondary">
                数量：{cancelTarget.quantity} 件 · 金额：{formatPriceCents(cancelTarget.total_amount_cents)}
              </Text>
            </Stack>
          </Surface>
        ) : null}
      </Dialog>
    </Stack>
  );
}
