import { useEffect, useId, type ReactNode } from 'react';
import { Button } from './Controls';
import { Cluster, Stack, Surface, Text } from './Layout';

export function InlineAlert({
  tone = 'info',
  title,
  description,
  action,
}: {
  tone?: 'info' | 'success' | 'warning' | 'danger';
  title: string;
  description?: string;
  action?: ReactNode;
}) {
  return (
    <div className={`fm-alert fm-alert--${tone}`} role="alert">
      <Stack gap="1">
        <Text variant="card-title" tone={tone === 'info' ? 'primary' : tone}>
          {title}
        </Text>
        {description ? (
          <Text variant="body" tone={tone === 'info' ? 'secondary' : tone}>
            {description}
          </Text>
        ) : null}
      </Stack>
      {action ? <div>{action}</div> : null}
    </div>
  );
}

export function EmptyState({
  title,
  description,
  action,
}: {
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <Surface variant="subtle" padding="lg">
      <Stack gap="3" align="center">
        <Text variant="section-title" as="h2">
          {title}
        </Text>
        <Text variant="body" tone="secondary">
          {description}
        </Text>
        {action}
      </Stack>
    </Surface>
  );
}

export function SkeletonCard() {
  return (
    <Surface variant="card" padding="md">
      <Stack gap="3">
        <div className="fm-skeleton fm-skeleton--short" />
        <div className="fm-skeleton fm-skeleton--title" />
        <div className="fm-skeleton" />
        <div className="fm-skeleton fm-skeleton--short" />
      </Stack>
    </Surface>
  );
}

export function Dialog({
  open,
  onClose,
  title,
  description,
  children,
  footer,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  description?: string;
  children: ReactNode;
  footer?: ReactNode;
}) {
  const titleId = useId();
  const descId = useId();

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        onClose();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div
      className="fm-dialog-backdrop"
      role="presentation"
      onClick={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <div
        className="fm-dialog-panel"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={description ? descId : undefined}
      >
        <Cluster justify="between" align="start">
          <Stack gap="1">
            <h2 id={titleId} className="fm-text--section-title">
              {title}
            </h2>
            {description ? (
              <p id={descId} className="fm-text--body fm-tone--secondary">
                {description}
              </p>
            ) : null}
          </Stack>
          <Button variant="ghost" size="sm" aria-label="关闭弹窗" onClick={onClose}>
            关闭
          </Button>
        </Cluster>
        <div>{children}</div>
        {footer ? (
          <Cluster justify="end" gap="2">
            {footer}
          </Cluster>
        ) : null}
      </div>
    </div>
  );
}
