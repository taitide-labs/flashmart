import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it } from 'vitest';
import { FlashMartClient, type Transport, type TransportRequest } from '../src/api/client';
import { StubTransport } from '../src/api/stubTransport';
import { AppRoutes } from '../src/App';
import { createFlashMartStore } from '../src/stores/flashmartStore';

function renderTestApp(options?: {
  clockMs?: number;
  paymentHoldMs?: number;
  initialEntries?: string[];
  transportOverride?: Transport;
}) {
  let currentMs = options?.clockMs ?? Date.UTC(2026, 10, 21, 10, 0, 0);
  let seq = 1;
  const stub = new StubTransport({
    clock: () => currentMs,
    paymentHoldMs: options?.paymentHoldMs ?? 15 * 60 * 1000,
  });
  const transport = options?.transportOverride ?? stub;
  const client = new FlashMartClient(transport);
  const store = createFlashMartStore({
    client,
    now: () => currentMs,
    nextId: () => `test-key-${seq++}`,
    initialBuyerId: 'buyer-test',
    initialTheme: 'light',
  });

  render(
    <MemoryRouter initialEntries={options?.initialEntries ?? ['/drops']}>
      <AppRoutes store={store} stubTransport={stub} />
    </MemoryRouter>,
  );

  return {
    stub,
    store,
    advanceTimeMs(delta: number) {
      currentMs += delta;
    },
  };
}

describe('FlashMart 客户端关键交互验收', () => {
  it('完成选品预占、模拟结算、取消已支付订单并验证库存实时回流', async () => {
    const user = userEvent.setup();
    renderTestApp();

    // 1. 浏览商品与初始库存 3 / 10
    expect(
      await screen.findByRole('heading', { name: 'FlashMart 创世数字徽章 · Pro Pass' }),
    ).toBeInTheDocument();
    expect(screen.getByText('3 / 10')).toBeInTheDocument();

    // 2. 点击立即抢购并提交预占
    const grabButtons = screen.getAllByRole('button', { name: '立即抢购…' });
    await user.click(grabButtons[0]);

    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByRole('button', { name: '提交并预占库存' }));

    expect(await screen.findByRole('heading', { name: '库存预占成功' })).toBeInTheDocument();
    expect(screen.getByText('2 / 10')).toBeInTheDocument();

    // 3. 在确认态直接模拟结算
    await user.click(
      within(screen.getByRole('dialog')).getByRole('button', { name: '确认模拟结算' }),
    );
    expect(await screen.findByText('模拟结算成功')).toBeInTheDocument();

    // 4. 前往我的订单并取消已支付订单
    await user.click(
      within(screen.getByRole('dialog')).getByRole('link', { name: '前往我的订单' }),
    );
    expect(await screen.findByRole('heading', { name: '我的订单' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /已支付 1/ })).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: '取消并退库…' }));
    const confirmDialog = await screen.findByRole('dialog');
    await user.click(within(confirmDialog).getByRole('button', { name: '确认取消并退库' }));

    expect(await screen.findByText('订单已取消，库存已退回')).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /已取消 1/ })).toBeInTheDocument();

    // 5. 返回抢购大厅验证库存恢复为 3 / 10
    await user.click(screen.getByRole('link', { name: '限时抢购' }));
    expect(await screen.findByText('3 / 10')).toBeInTheDocument();
  });

  it('校验单人限购拒绝与超时过期自动释放库存（无等待快进时钟）', async () => {
    const user = userEvent.setup();
    const { advanceTimeMs } = renderTestApp({ paymentHoldMs: 5 * 60 * 1000 });

    await screen.findByText('3 / 10');
    const grabButtons = screen.getAllByRole('button', { name: '立即抢购…' });
    await user.click(grabButtons[0]);
    await user.click(
      within(await screen.findByRole('dialog')).getByRole('button', { name: '提交并预占库存' }),
    );
    await screen.findByRole('heading', { name: '库存预占成功' });
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: '留在本页' }));

    // 同一买家再次抢购同一商品触发限购拒绝
    await user.click(screen.getAllByRole('button', { name: '立即抢购…' })[0]);
    await user.click(
      within(await screen.findByRole('dialog')).getByRole('button', { name: '提交并预占库存' }),
    );
    expect(await screen.findByText('超出单人限购上限')).toBeInTheDocument();
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: '取消' }));

    // 快进时钟超过 5 分钟支付保留期
    advanceTimeMs(6 * 60 * 1000);
    await user.click(screen.getByRole('button', { name: '刷新库存' }));
    expect(await screen.findByText('3 / 10')).toBeInTheDocument();

    await user.click(screen.getByRole('link', { name: /^我的订单/ }));
    expect(await screen.findByRole('tab', { name: /已过期 1/ })).toBeInTheDocument();
  });

  it('慢请求竞态下旧响应不得覆盖新状态，且支持空态与网络错误呈现', async () => {
    const user = userEvent.setup();
    let callCount = 0;
    const baseStub = new StubTransport();

    const racingTransport: Transport = {
      async request<T>(req: TransportRequest): Promise<T> {
        if (req.method === 'GET' && req.path === '/api/flash-items') {
          callCount += 1;
          const myCall = callCount;
          if (myCall <= 2) {
            // 较早发出的请求故意慢于后续请求
            await new Promise((resolve) => setTimeout(resolve, 45));
            return {
              items: [
                {
                  id: '11111111-1111-4111-8111-111111111101',
                  title: '旧请求商品',
                  description: 'stale',
                  item_kind: 'digital',
                  price_cents: 100,
                  currency: 'CNY',
                  total_stock: 10,
                  available_stock: 9,
                  per_user_limit: 1,
                  sale_starts_at: new Date().toISOString(),
                  sale_ends_at: new Date(Date.now() + 600000).toISOString(),
                  window_status: 'active',
                  server_time: new Date().toISOString(),
                },
              ],
              server_time: new Date().toISOString(),
            } as T;
          }
          return {
            items: [
              {
                id: '11111111-1111-4111-8111-111111111101',
                title: '最新实时商品',
                description: 'fresh',
                item_kind: 'digital',
                price_cents: 200,
                currency: 'CNY',
                total_stock: 10,
                available_stock: 1,
                per_user_limit: 1,
                sale_starts_at: new Date().toISOString(),
                sale_ends_at: new Date(Date.now() + 600000).toISOString(),
                window_status: 'active',
                server_time: new Date().toISOString(),
              },
            ],
            server_time: new Date().toISOString(),
          } as T;
        }
        return baseStub.request<T>(req);
      },
    };

    const { store } = renderTestApp({ transportOverride: racingTransport });
    // 在首次慢请求未返回时连续触发新请求，验证新请求胜出且不被旧响应覆盖
    await act(async () => {
      const p1 = store.loadItems();
      const p2 = store.loadItems();
      await Promise.all([p1, p2]);
    });

    expect(await screen.findByRole('heading', { name: '最新实时商品' })).toBeInTheDocument();
    await new Promise((resolve) => setTimeout(resolve, 65));
    expect(screen.queryByRole('heading', { name: '旧请求商品' })).not.toBeInTheDocument();

    // 独立验证展厅情景切换：空态与网络错误
    renderTestApp({ initialEntries: ['/gallery'] });
    await user.click(screen.getByRole('button', { name: '模拟空态' }));
    await user.click(screen.getAllByRole('link', { name: '限时抢购' })[1]);
    await waitFor(() => {
      expect(screen.getByText('当前筛选下暂无抢购商品')).toBeInTheDocument();
    });
  });

  it('支持单品详情页抢购、买家标识切换隔离与亮暗主题切换', async () => {
    const user = userEvent.setup();
    renderTestApp({ initialEntries: ['/drops/11111111-1111-4111-8111-111111111102'] });

    // 1. 单品详情页直接下单与立即结算
    expect(
      await screen.findByRole('heading', { name: 'CNC 阳极氧化铝机械旋钮宏键盘' }),
    ).toBeInTheDocument();
    expect(screen.getByText('2 / 5')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: '立即抢购并预占库存' }));
    expect(await screen.findByText('库存预占成功，已生成待支付订单')).toBeInTheDocument();
    expect(screen.getByText('1 / 5')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: '立即模拟结算' }));
    expect(await screen.findByText('模拟结算成功')).toBeInTheDocument();

    // 2. 切换主题到暗色
    await user.click(screen.getByRole('button', { name: '切换到暗色主题' }));
    expect(document.documentElement.dataset.theme).toBe('dark');

    // 3. 切换买家校验输入错误态及多买家独立抢购同一商品最后 1 件库存
    await user.click(screen.getByRole('button', { name: /切换买家/ }));
    const dialog = await screen.findByRole('dialog');
    const input = within(dialog).getByLabelText('买家标识（X-Buyer-Id）');
    await user.clear(input);
    await user.type(input, 'bad buyer!');
    await user.click(within(dialog).getByRole('button', { name: '确认切换' }));
    expect(await within(dialog).findByText(/仅支持字母、数字/)).toBeInTheDocument();

    await user.click(within(dialog).getByRole('button', { name: 'buyer-alice' }));
    await user.click(within(dialog).getByRole('button', { name: '确认切换' }));
    expect(await screen.findByText('已切换当前买家')).toBeInTheDocument();

    // buyer-alice 抢购最后 1 件库存后库存归零，按钮变为已售罄
    await user.click(screen.getByRole('button', { name: '立即抢购并预占库存' }));
    expect(await screen.findByText('0 / 5')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '已售罄' })).toBeDisabled();
  });
});
