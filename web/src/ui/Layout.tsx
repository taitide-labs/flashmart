import type { ReactNode } from 'react';
import { SPACE_CLASS, type SpaceScale } from './tokens';

export function Container({
  size = 'lg',
  children,
}: {
  size?: 'md' | 'lg';
  children: ReactNode;
}) {
  return <div className={`fm-container fm-container--${size}`}>{children}</div>;
}

export function Stack({
  gap = '4',
  align = 'stretch',
  children,
}: {
  gap?: SpaceScale;
  align?: 'center' | 'stretch';
  children: ReactNode;
}) {
  return (
    <div className={`fm-stack fm-stack--align-${align} ${SPACE_CLASS[gap]}`}>
      {children}
    </div>
  );
}

export function Cluster({
  gap = '3',
  align = 'center',
  justify = 'start',
  wrap = true,
  children,
}: {
  gap?: SpaceScale;
  align?: 'center' | 'baseline' | 'start';
  justify?: 'start' | 'between' | 'end';
  wrap?: boolean;
  children: ReactNode;
}) {
  return (
    <div
      className={`fm-cluster fm-cluster--align-${align} fm-cluster--justify-${justify} ${
        wrap ? '' : 'fm-cluster--nowrap'
      } ${SPACE_CLASS[gap]}`}
    >
      {children}
    </div>
  );
}

export function Grid({
  cols = 2,
  gap = '4',
  children,
}: {
  cols?: 1 | 2 | 3;
  gap?: SpaceScale;
  children: ReactNode;
}) {
  return (
    <div className={`fm-grid fm-grid--cols-${cols} ${SPACE_CLASS[gap]}`}>
      {children}
    </div>
  );
}

export function Surface({
  variant = 'card',
  padding = 'md',
  children,
}: {
  variant?: 'card' | 'subtle' | 'elevated';
  padding?: 'none' | 'sm' | 'md' | 'lg';
  children: ReactNode;
}) {
  return (
    <div className={`fm-surface fm-surface--${variant} fm-pad-${padding}`}>
      {children}
    </div>
  );
}

export function Divider() {
  return <hr className="fm-divider" />;
}

export function Text({
  variant = 'body',
  tone = 'primary',
  as = 'p',
  children,
}: {
  variant?:
    | 'page-title'
    | 'section-title'
    | 'card-title'
    | 'body'
    | 'caption'
    | 'mono-stat'
    | 'mono-code';
  tone?: 'primary' | 'secondary' | 'muted' | 'accent' | 'success' | 'warning' | 'danger';
  as?: 'h1' | 'h2' | 'h3' | 'p' | 'span' | 'div';
  children: ReactNode;
}) {
  const Tag = as;
  return (
    <Tag className={`fm-text--${variant} fm-tone--${tone}`}>{children}</Tag>
  );
}
