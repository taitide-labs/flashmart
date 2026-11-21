import { useState } from 'react';
import { BrowserRouter, Navigate, Route, Routes } from 'react-router-dom';
import type { StubTransport } from './api/stubTransport';
import { BuyerSwitcherDialog } from './components/BuyerSwitcherDialog';
import { DropDetailPage } from './pages/DropDetailPage';
import { DropsPage } from './pages/DropsPage';
import { GalleryPage } from './pages/GalleryPage';
import { OrdersPage } from './pages/OrdersPage';
import { useFlashMartStore, type FlashMartStore } from './stores/flashmartStore';
import { AppShell } from './ui';

export function AppRoutes({
  store,
  stubTransport = null,
}: {
  store: FlashMartStore;
  stubTransport?: StubTransport | null;
}) {
  const state = useFlashMartStore(store);
  const [buyerDialogOpen, setBuyerDialogOpen] = useState(false);

  const pendingCount = state.orders.filter((o) => o.status === 'pending_payment').length;

  return (
    <>
      <AppShell
        buyerId={state.buyerId}
        theme={state.theme}
        onToggleTheme={() => store.toggleTheme()}
        onOpenBuyerSwitcher={() => setBuyerDialogOpen(true)}
        navItems={[
          { to: '/drops', label: '限时抢购' },
          { to: '/orders', label: '我的订单', badgeCount: pendingCount },
          { to: '/gallery', label: '组件展厅' },
        ]}
      >
        <Routes>
          <Route path="/" element={<Navigate to="/drops" replace />} />
          <Route path="/drops" element={<DropsPage store={store} />} />
          <Route path="/drops/:itemId" element={<DropDetailPage store={store} />} />
          <Route path="/orders" element={<OrdersPage store={store} />} />
          <Route
            path="/gallery"
            element={<GalleryPage store={store} stubTransport={stubTransport} />}
          />
          <Route path="*" element={<Navigate to="/drops" replace />} />
        </Routes>
      </AppShell>

      <BuyerSwitcherDialog
        open={buyerDialogOpen}
        currentBuyerId={state.buyerId}
        onClose={() => setBuyerDialogOpen(false)}
        onConfirm={(next) => store.switchBuyer(next)}
      />
    </>
  );
}

export function App({
  store,
  stubTransport = null,
}: {
  store: FlashMartStore;
  stubTransport?: StubTransport | null;
}) {
  return (
    <BrowserRouter>
      <AppRoutes store={store} stubTransport={stubTransport} />
    </BrowserRouter>
  );
}
