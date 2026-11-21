import { ArrowLeft } from 'lucide-react';
import { useEffect, useState } from 'react';
import { Link, useParams } from 'react-router-dom';
import type { FlashOrderView } from '../api/types';
import { useFlashMartStore, type FlashMartStore } from '../stores/flashmartStore';
import {
  Badge,
  Button,
  Cluster,
  Divider,
  InlineAlert,
  NumberStepper,
  SkeletonCard,
  Stack,
  Surface,
  Text,
} from '../ui';
import {
  formatCountdown,
  formatPriceCents,
  itemKindLabel,
  windowStatusMeta,
} from '../utils/format';

export function DropDetailPage({ store }: { store: FlashMartStore }) {
  const { itemId = '' } = useParams();
  const state = useFlashMartStore(store);
  const [quantity, setQuantity] = useState(1);
  const [nowTick, setNowTick] = useState(() => store.getServerNowMs());
  const [recentOrder, setRecentOrder] = useState<FlashOrderView | null>(null);

  useEffect(() => {
    if (itemId) {
      void store.loadItemDetail(itemId);
    }
  }, [store, itemId]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      setNowTick(store.getServerNowMs());
    }, 1000);
    return () => window.clearInterval(timer);
  }, [store]);

  const item = state.selectedItem;
  const canOrder = item ? item.window_status === 'active' && item.available_stock > 0 : false;
  const meta = item ? windowStatusMeta(item.window_status, item.available_stock) : null;

  async function handleOrder() {
    if (!item) return;
    const placed = await store.placeOrder(item.id, quantity);
    if (placed) {
      setRecentOrder(placed);
    }
  }

  return (
    <Stack gap="5">
      <Cluster justify="between" align="center">
        <Link to="/drops">
          <Button variant="ghost" size="sm">
            <ArrowLeft size={14} aria-hidden="true" />
            <span>返回抢购大厅</span>
          </Button>
        </Link>
        <Link to="/orders">
          <Button variant="secondary" size="sm">
            查看我的订单
          </Button>
        </Link>
      </Cluster>

      {state.feedback ? (
        <InlineAlert
          tone={state.feedback.tone}
          title={state.feedback.title}
          description={state.feedback.description}
          action={
            <Button variant="ghost" size="sm" onClick={() => store.clearFeedback()}>
              关闭
            </Button>
          }
        />
      ) : null}

      {state.selectedItemLoading && !item ? (
        <SkeletonCard />
      ) : state.selectedItemError ? (
        <InlineAlert
          tone="danger"
          title="无法加载商品详情"
          description={state.selectedItemError}
        />
      ) : item && meta ? (
        <Surface variant="card" padding="lg">
          <Stack gap="5">
            <Cluster gap="2">
              <Badge tone="neutral">{itemKindLabel(item.item_kind)}</Badge>
              <Badge tone={meta.tone}>{meta.label}</Badge>
              <Badge tone="neutral">单人限购 {item.per_user_limit} 件</Badge>
            </Cluster>

            <Stack gap="2">
              <Text variant="page-title" as="h1">
                {item.title}
              </Text>
              <Text variant="body" tone="secondary">
                {item.description}
              </Text>
            </Stack>

            <Surface variant="subtle" padding="md">
              <Cluster justify="between" align="center" gap="4">
                <Stack gap="1">
                  <Text variant="caption" tone="muted">
                    限时价格
                  </Text>
                  <Text variant="mono-stat">{formatPriceCents(item.price_cents)}</Text>
                </Stack>
                <Stack gap="1">
                  <Text variant="caption" tone="muted">
                    实时可售库存
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
                    窗口倒计时
                  </Text>
                  <Text variant="mono-code" tone="secondary">
                    {item.window_status === 'upcoming'
                      ? `距开抢 ${formatCountdown(item.sale_starts_at, nowTick)}`
                      : item.window_status === 'active'
                        ? `距结束 ${formatCountdown(item.sale_ends_at, nowTick)}`
                        : '已结束'}
                  </Text>
                </Stack>
              </Cluster>
            </Surface>

            <Divider />

            <Cluster justify="between" align="center" gap="4">
              <NumberStepper
                label="购买件数"
                value={quantity}
                min={1}
                max={Math.max(1, Math.min(item.per_user_limit, item.available_stock))}
                disabled={!canOrder}
                onChange={setQuantity}
              />

              <Cluster gap="3">
                {recentOrder && recentOrder.status === 'pending_payment' ? (
                  <Button
                    variant="secondary"
                    size="lg"
                    loading={Boolean(state.mutatingOrderIds[recentOrder.id])}
                    onClick={async () => {
                      const paid = await store.payOrder(recentOrder);
                      if (paid) setRecentOrder(paid);
                    }}
                  >
                    立即模拟结算
                  </Button>
                ) : null}
                <Button
                  variant="primary"
                  size="lg"
                  disabled={!canOrder}
                  loading={state.placingItemId === item.id}
                  onClick={() => void handleOrder()}
                >
                  {canOrder ? '立即抢购并预占库存' : meta.label}
                </Button>
              </Cluster>
            </Cluster>
          </Stack>
        </Surface>
      ) : null}
    </Stack>
  );
}
