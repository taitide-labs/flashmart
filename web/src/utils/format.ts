import type { ItemKind, OrderStatus, WindowStatus } from '../api/types';
import type { Tone } from '../ui';

export function formatPriceCents(cents: number): string {
  return `¥${(cents / 100).toFixed(2)}`;
}

export function formatCountdown(targetIso: string, nowMs: number): string {
  const diffSec = Math.max(0, Math.floor((Date.parse(targetIso) - nowMs) / 1000));
  const hours = Math.floor(diffSec / 3600);
  const minutes = Math.floor((diffSec % 3600) / 60);
  const seconds = diffSec % 60;
  const pad = (n: number) => String(n).padStart(2, '0');
  if (hours > 0) {
    return `${pad(hours)}:${pad(minutes)}:${pad(seconds)}`;
  }
  return `${pad(minutes)}:${pad(seconds)}`;
}

export function itemKindLabel(kind: ItemKind): string {
  return kind === 'digital' ? '数字好物' : '实体好物';
}

export function windowStatusMeta(
  status: WindowStatus,
  availableStock: number,
): { label: string; tone: Tone } {
  if (status === 'upcoming') {
    return { label: '即将开抢', tone: 'warning' };
  }
  if (status === 'ended') {
    return { label: '已结束', tone: 'neutral' };
  }
  if (availableStock <= 0) {
    return { label: '已售罄', tone: 'danger' };
  }
  return { label: '抢购中', tone: 'accent' };
}

export function orderStatusMeta(status: OrderStatus): { label: string; tone: Tone } {
  switch (status) {
    case 'pending_payment':
      return { label: '待支付', tone: 'warning' };
    case 'paid':
      return { label: '已支付', tone: 'success' };
    case 'cancelled':
      return { label: '已取消', tone: 'neutral' };
    case 'expired':
      return { label: '已过期', tone: 'danger' };
  }
}

export function validateBuyerId(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) {
    return '买家标识不能为空';
  }
  if (trimmed.length > 64) {
    return '买家标识长度不能超过 64 个字符';
  }
  if (!/^[A-Za-z0-9_-]+$/.test(trimmed)) {
    return '仅支持字母、数字、下划线（_）与连字符（-）';
  }
  return null;
}
