import { useEffect, useState } from 'react';
import { Button, Cluster, Dialog, Input, Stack, Text } from '../ui';
import { validateBuyerId } from '../utils/format';

const PRESET_BUYERS = ['buyer-demo', 'buyer-alice', 'buyer-bob'];

export function BuyerSwitcherDialog({
  open,
  currentBuyerId,
  onClose,
  onConfirm,
}: {
  open: boolean;
  currentBuyerId: string;
  onClose: () => void;
  onConfirm: (nextBuyerId: string) => Promise<void>;
}) {
  const [draft, setDraft] = useState(currentBuyerId);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (open) {
      setDraft(currentBuyerId);
      setError(null);
    }
  }, [open, currentBuyerId]);

  async function handleSubmit() {
    const validation = validateBuyerId(draft);
    if (validation) {
      setError(validation);
      return;
    }
    setSubmitting(true);
    try {
      await onConfirm(draft.trim());
      onClose();
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="切换买家标识"
      description="个人订单列表与每人限购占用按当前买家标识隔离，可快速切换多个买家验证防超卖与限购规则。"
      footer={
        <>
          <Button variant="ghost" onClick={onClose} disabled={submitting}>
            取消
          </Button>
          <Button variant="primary" onClick={handleSubmit} loading={submitting}>
            确认切换
          </Button>
        </>
      }
    >
      <Stack gap="4">
        <Input
          label="买家标识（X-Buyer-Id）"
          value={draft}
          mono
          error={error}
          hint="支持 1-64 位字母、数字、下划线或连字符"
          onChange={(e) => {
            setDraft(e.target.value);
            if (error) setError(null);
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              void handleSubmit();
            }
          }}
        />
        <Stack gap="2">
          <Text variant="caption" tone="muted">
            快捷测试买家
          </Text>
          <Cluster gap="2">
            {PRESET_BUYERS.map((preset) => (
              <Button
                key={preset}
                size="sm"
                variant={draft.trim() === preset ? 'primary' : 'secondary'}
                onClick={() => {
                  setDraft(preset);
                  setError(null);
                }}
              >
                {preset}
              </Button>
            ))}
          </Cluster>
        </Stack>
      </Stack>
    </Dialog>
  );
}
