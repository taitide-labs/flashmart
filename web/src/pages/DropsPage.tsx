import { RefreshCw } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import type { FlashItemSummary, FlashOrderView, WindowStatus } from '../api/types';
import { useFlashMartStore, type FlashMartStore } from '../stores/flashmartStore';
import {
  Badge,
  Button,
  Cluster,
  Dialog,
  Divider,
  EmptyState,
  Grid,
  InlineAlert,
  LinkButton,
  NumberStepper,
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
  windowStatusMeta,
} from '../utils/format';

type FilterTab = 'all' | WindowStatus;

export function DropsPage({ store }: { store: FlashMartStore }) {
  const state = useFlashMartStore(store);
  const [filter, setFilter] = useState<FilterTab>('all');
  const [nowTick, setNowTick] = useState(() => store.getServerNowMs());
  const [checkoutItem, setCheckoutItem] = useState<FlashItemSummary | null>(null);
  const [quantity, setQuantity] = useState(1);
  const [createdOrder, setCreatedOrder] = useState<FlashOrderView | null>(null);

  useEffect(() => {
    void store.loadItems();
    void store.loadOrders();
  }, [store]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      setNowTick(store.getServerNowMs());
    }, 1000);
    return () => window.clearInterval(timer);
  }, [store]);

  const activeOrderItemIds = useMemo(() => {
    return new Set(
      state.orders
        .filter((o) => o.status === 'pending_payment' || o.status === 'paid')
        .map((o) => o.item_id),
    );
  }, [state.orders]);

  const counts = useMemo(() => {
    const map: Record<FilterTab, number> = {
      all: state.items.length,
      active: 0,
      upcoming: 0,
      ended: 0,
    };
    for (const item of state.items) {
      map[item.window_status] += 1;
    }
    return map;
  }, [state.items]);

  const visibleItems = useMemo(() => {
    if (filter === 'all') return state.items;
    return state.items.filter((item) => item.window_status === filter);
  }, [state.items, filter]);

  function openCheckout(item: FlashItemSummary) {
    setCheckoutItem(item);
    setQuantity(1);
    setCreatedOrder(null);
  }

  function closeCheckout() {
    setCheckoutItem(null);
    setCreatedOrder(null);
  }

  async function handleConfirmPlaceOrder() {
    if (!checkoutItem) return;
    const order = await store.placeOrder(checkoutItem.id, quantity);
    if (order) {
      setCreatedOrder(order);
    }
  }

  async function handlePayCreatedOrder() {
    if (!createdOrder) return;
    const paid = await store.payOrder(createdOrder);
    if (paid) {
      setCreatedOrder(paid);
    }
  }

  return (
    <Stack gap="6">
      <Cluster justify="between" align="start" gap="4">
        <Stack gap="1">
          <Text variant="page-title" as="h1">
            限时抢购大厅
          </Text>
          <Text variant="body" tone="secondary">
            下单即原子预占库存并锁定 15 分钟支付保留期；取消或超时未付自动回流可售库存。
          </Text>
        </Stack>
        <Button
          variant="secondary"
          size="sm"
          loading={state.itemsLoading}
          onClick={() => void store.loadItems()}
        >
          <RefreshCw size={14} aria-hidden="true" />
          <span>刷新库存</span>
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
        ariaLabel="商品状态筛选"
        value={filter}
        onChange={setFilter}
        items={[
          { value: 'all', label: '全部', count: counts.all },
          { value: 'active', label: '抢购中', count: counts.active },
          { value: 'upcoming', label: '即将开抢', count: counts.upcoming },
          { value: 'ended', label: '已结束', count: counts.ended },
        ]}
      />

      {state.itemsError ? (
        <InlineAlert
          tone="danger"
          title="商品列表加载失败"
          description={state.itemsError}
          action={
            <Button variant="secondary" size="sm" onClick={() => void store.loadItems()}>
              重试
            </Button>
          }
        />
      ) : null}

      {state.itemsLoading && state.items.length === 0 ? (
        <Grid cols={2} gap="4">
          <SkeletonCard />
          <SkeletonCard />
          <SkeletonCard />
          <SkeletonCard />
        </Grid>
      ) : visibleItems.length === 0 ? (
        <EmptyState
          title="当前筛选下暂无抢购商品"
          description="可切换至全部商品查看即将开抢或已结束场次，或刷新最新库存。"
          action={
            <Button variant="secondary" onClick={() => setFilter('all')}>
              查看全部商品
            </Button>
          }
        />
      ) : (
        <Grid cols={2} gap="4">
          {visibleItems.map((item) => {
            const meta = windowStatusMeta(item.window_status, item.available_stock);
            const canOrder = item.window_status === 'active' && item.available_stock > 0;
            const hasActiveOrder = activeOrderItemIds.has(item.id);
            const countdownLabel =
              item.window_status === 'upcoming'
                ? `距开抢 ${formatCountdown(item.sale_starts_at, nowTick)}`
                : item.window_status === 'active'
                  ? `距结束 ${formatCountdown(item.sale_ends_at, nowTick)}`
                  : '本场已截单';

            return (
              <Surface key={item.id} variant="card" padding="md">
                <Stack gap="4">
                  <Cluster justify="between" align="start" gap="2">
                    <Cluster gap="2">
                      <Badge tone="neutral">{itemKindLabel(item.item_kind)}</Badge>
                      <Badge tone={meta.tone}>{meta.label}</Badge>
                      {hasActiveOrder ? (
                        <Badge tone="success">当前买家已持有效单</Badge>
                      ) : null}
                    </Cluster>
                    <Text variant="caption" tone="muted">
                      限购 {item.per_user_limit} 件
                    </Text>
                  </Cluster>

                  <Stack gap="2">
                    <Link to={`/drops/${item.id}`}>
                      <Text variant="section-title" as="h2">
                        {item.title}
                      </Text>
                    </Link>
                    <Text variant="body" tone="secondary">
                      {item.description}
                    </Text>
                  </Stack>

                  <Surface variant="subtle" padding="sm">
                    <Cluster justify="between" align="center" gap="3">
                      <Stack gap="1">
                        <Text variant="caption" tone="muted">
                          抢购价
                        </Text>
                        <Text variant="mono-stat" tone="primary">
                          {formatPriceCents(item.price_cents)}
                        </Text>
                      </Stack>

                      <Stack gap="1">
                        <Text variant="caption" tone="muted">
                          剩余库存
                        </Text>
                        <Text
                          variant="mono-stat"
                          tone={item.available_stock > 0 ? 'accent' : 'danger'}
                        >
                          {item.available_stock} / {item.total_stock}
                        </Text>
                      </Stack>

                      <Stack gap="1">
                        <Text variant="caption" tone="muted">
                          时间窗
                        </Text>
                        <Text variant="mono-code" tone="secondary">
                          {countdownLabel}
                        </Text>
                      </Stack>
                    </Cluster>
                  </Surface>

                  <Divider />

                  <Cluster justify="between" align="center" gap="2">
                    <LinkButton to={`/drops/${item.id}`} variant="ghost" size="sm">
                      商品详情
                    </LinkButton>
                    <Button
                      variant={canOrder ? 'primary' : 'secondary'}
                      size="md"
                      disabled={!canOrder}
                      loading={state.placingItemId === item.id}
                      onClick={() => openCheckout(item)}
                    >
                      {item.window_status === 'upcoming'
                        ? '尚未开抢'
                        : item.window_status === 'ended'
                          ? '抢购已结束'
                          : item.available_stock <= 0
                            ? '已售罄'
                            : '立即抢购…'}
                    </Button>
                  </Cluster>
                </Stack>
              </Surface>
            );
          })}
        </Grid>
      )}

      <Dialog
        open={Boolean(checkoutItem)}
        onClose={closeCheckout}
        title={createdOrder ? '库存预占成功' : '确认下单抢购'}
        description={
          createdOrder
            ? '订单已锁定对应库存，请在支付保留期内完成模拟结算。'
            : '提交将原子预占可售库存，若并发售罄或超出限购将立即拒绝。'
        }
        footer={
          createdOrder ? (
            <>
              <Button variant="ghost" onClick={closeCheckout}>
                留在本页
              </Button>
              <LinkButton to="/orders" variant="secondary" onClick={closeCheckout}>
                前往我的订单
              </LinkButton>
              {createdOrder.status === 'pending_payment' ? (
                <Button
                  variant="primary"
                  loading={Boolean(state.mutatingOrderIds[createdOrder.id])}
                  onClick={() => void handlePayCreatedOrder()}
                >
                  确认模拟结算
                </Button>
              ) : null}
            </>
          ) : (
            <>
              <Button variant="ghost" onClick={closeCheckout}>
                取消
              </Button>
              <Button
                variant="primary"
                loading={Boolean(checkoutItem && state.placingItemId === checkoutItem.id)}
                onClick={() => void handleConfirmPlaceOrder()}
              >
                提交并预占库存
              </Button>
            </>
          )
        }
      >
        {checkoutItem ? (
          createdOrder ? (
            <Stack gap="3">
              <Surface variant="subtle" padding="md">
                <Stack gap="2">
                  <Cluster justify="between">
                    <Text variant="card-title">{createdOrder.item_title}</Text>
                    <Badge tone={createdOrder.status === 'paid' ? 'success' : 'warning'}>
                      {createdOrder.status === 'paid' ? '已支付' : '待支付'}
                    </Badge>
                  </Cluster>
                  <Text variant="mono-code" tone="secondary">
                    订单号：{createdOrder.id}
                  </Text>
                  <Text variant="body" tone="secondary">
                    应付金额：{formatPriceCents(createdOrder.total_amount_cents)}（{createdOrder.quantity} 件）
                  </Text>
                  {createdOrder.status === 'pending_payment' ? (
                    <Text variant="mono-code" tone="warning">
                      支付保留倒计时：{formatCountdown(createdOrder.payment_deadline, nowTick)}
                    </Text>
                  ) : (
                    <Text variant="body" tone="success">
                      {createdOrder.fulfillment_hint}
                    </Text>
                  )}
                </Stack>
              </Surface>
            </Stack>
          ) : (
            <Stack gap="4">
              <Surface variant="subtle" padding="md">
                <Stack gap="2">
                  <Cluster justify="between">
                    <Text variant="card-title">{checkoutItem.title}</Text>
                    <Badge tone="neutral">{itemKindLabel(checkoutItem.item_kind)}</Badge>
                  </Cluster>
                  <Cluster justify="between">
                    <Text variant="body" tone="secondary">
                      单价：{formatPriceCents(checkoutItem.price_cents)}
                    </Text>
                    <Text variant="mono-code" tone="accent">
                      当前剩余：{checkoutItem.available_stock} 件
                    </Text>
                  </Cluster>
                </Stack>
              </Surface>

              <Cluster justify="between" align="center">
                <NumberStepper
                  label={`购买数量（单人限购 ${checkoutItem.per_user_limit} 件）`}
                  value={quantity}
                  min={1}
                  max={Math.max(1, Math.min(checkoutItem.per_user_limit, checkoutItem.available_stock))}
                  onChange={setQuantity}
                />
                <Stack gap="1">
                  <Text variant="caption" tone="muted">
                    合计应付
                  </Text>
                  <Text variant="mono-stat">
                    {formatPriceCents(checkoutItem.price_cents * quantity)}
                  </Text>
                </Stack>
              </Cluster>

              <Text variant="caption" tone="muted">
                当前下单买家：{state.buyerId}
              </Text>
            </Stack>
          )
        ) : null}
      </Dialog>
    </Stack>
  );
}
