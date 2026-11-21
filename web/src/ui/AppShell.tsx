import { Moon, Sun, UserRound } from 'lucide-react';
import type { ReactNode } from 'react';
import { NavLink } from 'react-router-dom';
import { Button } from './Controls';
import { Cluster, Container } from './Layout';
import type { ThemeMode } from './tokens';

export interface NavItem {
  to: string;
  label: string;
  badgeCount?: number;
}

export function AppShell({
  navItems,
  buyerId,
  onOpenBuyerSwitcher,
  theme,
  onToggleTheme,
  children,
}: {
  navItems: NavItem[];
  buyerId: string;
  onOpenBuyerSwitcher: () => void;
  theme: ThemeMode;
  onToggleTheme: () => void;
  children: ReactNode;
}) {
  return (
    <div className="fm-shell">
      <header className="fm-header">
        <div className="fm-header-inner">
          <Cluster gap="5" align="center">
            <NavLink to="/drops" className="fm-brand">
              <span className="fm-brand-dot" aria-hidden="true" />
              <span>FlashMart</span>
            </NavLink>
            <nav className="fm-nav" aria-label="主导航">
              {navItems.map((item) => (
                <NavLink key={item.to} to={item.to} className="fm-nav-link">
                  <span>{item.label}</span>
                  {typeof item.badgeCount === 'number' && item.badgeCount > 0 ? (
                    <span className="tabular-mono">({item.badgeCount})</span>
                  ) : null}
                </NavLink>
              ))}
            </nav>
          </Cluster>

          <Cluster gap="2" align="center">
            <Button
              variant="secondary"
              size="sm"
              onClick={onOpenBuyerSwitcher}
              aria-label={`切换买家，当前买家 ${buyerId}`}
            >
              <UserRound size={14} aria-hidden="true" />
              <span className="tabular-mono">{buyerId}</span>
              <span>切换…</span>
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={onToggleTheme}
              aria-label={theme === 'light' ? '切换到暗色主题' : '切换到亮色主题'}
            >
              {theme === 'light' ? <Moon size={15} aria-hidden="true" /> : <Sun size={15} aria-hidden="true" />}
              <span>{theme === 'light' ? '暗色' : '亮色'}</span>
            </Button>
          </Cluster>
        </div>
      </header>
      <main className="fm-main">
        <Container>{children}</Container>
      </main>
    </div>
  );
}
