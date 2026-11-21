import { useState } from 'react';
import type { StubScenario, StubTransport } from '../api/stubTransport';
import type { FlashMartStore } from '../stores/flashmartStore';
import {
  Badge,
  Button,
  Cluster,
  EmptyState,
  Grid,
  InlineAlert,
  Input,
  NumberStepper,
  SkeletonCard,
  Stack,
  Surface,
  Tabs,
  Text,
} from '../ui';

export function GalleryPage({
  store,
  stubTransport,
}: {
  store: FlashMartStore;
  stubTransport: StubTransport | null;
}) {
  const [stepperVal, setStepperVal] = useState(1);
  const [tabVal, setTabVal] = useState<'a' | 'b'>('a');
  const [scenario, setScenario] = useState<StubScenario>('normal');

  function applyScenario(next: StubScenario) {
    if (!stubTransport) return;
    stubTransport.setScenario(next);
    setScenario(next);
    void store.loadItems();
    void store.loadOrders();
  }

  return (
    <Stack gap="6">
      <Stack gap="1">
        <Text variant="page-title" as="h1">
          UI 受控组件与全状态展厅
        </Text>
        <Text variant="body" tone="secondary">
          集中核验亮暗双主题表面材质、外置焦点环、语义徽章与传输拦截桩情景。
        </Text>
      </Stack>

      {stubTransport ? (
        <Surface variant="card" padding="md">
          <Stack gap="3">
            <Text variant="section-title" as="h2">
              前端本地拦截桩情景控制
            </Text>
            <Cluster gap="2">
              <Button
                size="sm"
                variant={scenario === 'normal' ? 'primary' : 'secondary'}
                onClick={() => applyScenario('normal')}
              >
                正常抢购数据
              </Button>
              <Button
                size="sm"
                variant={scenario === 'empty' ? 'primary' : 'secondary'}
                onClick={() => applyScenario('empty')}
              >
                模拟空态
              </Button>
              <Button
                size="sm"
                variant={scenario === 'network_error' ? 'primary' : 'secondary'}
                onClick={() => applyScenario('network_error')}
              >
                模拟网络故障
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => {
                  stubTransport.reset();
                  applyScenario('normal');
                }}
              >
                重置演示库存
              </Button>
            </Cluster>
          </Stack>
        </Surface>
      ) : null}

      <Grid cols={2} gap="4">
        <Surface variant="card" padding="md">
          <Stack gap="3">
            <Text variant="section-title" as="h2">
              按钮与加载态
            </Text>
            <Cluster gap="2">
              <Button variant="primary">主强调按钮</Button>
              <Button variant="secondary">次级按钮</Button>
              <Button variant="ghost">幽灵按钮</Button>
              <Button variant="danger">危险操作</Button>
              <Button variant="primary" loading>
                提交中
              </Button>
              <Button variant="secondary" disabled>
                不可用
              </Button>
            </Cluster>
          </Stack>
        </Surface>

        <Surface variant="card" padding="md">
          <Stack gap="3">
            <Text variant="section-title" as="h2">
              状态徽章与标签页
            </Text>
            <Cluster gap="2">
              <Badge tone="neutral">中性标签</Badge>
              <Badge tone="accent">抢购中</Badge>
              <Badge tone="success">已支付</Badge>
              <Badge tone="warning">待支付</Badge>
              <Badge tone="danger">已售罄</Badge>
            </Cluster>
            <Tabs
              ariaLabel="展厅示例标签"
              value={tabVal}
              onChange={setTabVal}
              items={[
                { value: 'a', label: '选项 A', count: 3 },
                { value: 'b', label: '选项 B', count: 0 },
              ]}
            />
          </Stack>
        </Surface>

        <Surface variant="card" padding="md">
          <Stack gap="3">
            <Text variant="section-title" as="h2">
              输入与步进器
            </Text>
            <Input label="常规输入" placeholder="buyer-demo" mono />
            <Input
              label="错误校验态"
              defaultValue="invalid buyer!"
              error="仅支持字母、数字、下划线或连字符"
            />
            <NumberStepper
              label="购买数量"
              value={stepperVal}
              min={1}
              max={3}
              onChange={setStepperVal}
            />
          </Stack>
        </Surface>

        <Surface variant="card" padding="md">
          <Stack gap="3">
            <Text variant="section-title" as="h2">
              反馈提示、空态与骨架屏
            </Text>
            <InlineAlert
              tone="danger"
              title="剩余库存不足 (insufficient_stock)"
              description="当前可售库存为 0，若有取消或超时订单将实时回流。"
            />
            <EmptyState
              title="空态占位示例"
              description="用于商品列表或订单列表为空时的清晰引导。"
            />
            <SkeletonCard />
          </Stack>
        </Surface>
      </Grid>
    </Stack>
  );
}
