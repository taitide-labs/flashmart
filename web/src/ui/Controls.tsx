import { useId, type ButtonHTMLAttributes, type InputHTMLAttributes, type ReactNode } from 'react';
import type { Tone } from './tokens';

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: 'primary' | 'secondary' | 'ghost' | 'danger';
  size?: 'sm' | 'md' | 'lg';
  loading?: boolean;
  fullWidth?: boolean;
  children: ReactNode;
}

export function Button({
  variant = 'secondary',
  size = 'md',
  loading = false,
  fullWidth = false,
  disabled,
  type = 'button',
  children,
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      disabled={Boolean(disabled || loading)}
      aria-busy={loading || undefined}
      className={`fm-btn fm-btn--${variant} fm-btn--${size} ${fullWidth ? 'fm-btn--full' : ''}`}
      {...rest}
    >
      {loading ? <span className="fm-spinner" aria-hidden="true" /> : null}
      {children}
    </button>
  );
}

export function Badge({
  tone = 'neutral',
  children,
}: {
  tone?: Tone;
  children: ReactNode;
}) {
  return <span className={`fm-badge fm-badge--${tone}`}>{children}</span>;
}

export interface InputProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'size'> {
  label: string;
  hint?: string;
  error?: string | null;
  mono?: boolean;
}

export function Input({ label, hint, error, mono = false, id, ...rest }: InputProps) {
  const generatedId = useId();
  const inputId = id ?? generatedId;
  const hintId = `${inputId}-hint`;
  const errorId = `${inputId}-error`;
  const describedBy =
    [error ? errorId : null, hint ? hintId : null].filter(Boolean).join(' ') || undefined;

  return (
    <div className="fm-field">
      <label htmlFor={inputId} className="fm-field-label">
        {label}
      </label>
      <input
        id={inputId}
        aria-invalid={Boolean(error)}
        aria-describedby={describedBy}
        className={`fm-input ${mono ? 'fm-input--mono' : ''}`}
        {...rest}
      />
      {hint ? (
        <span id={hintId} className="fm-text--caption fm-tone--muted">
          {hint}
        </span>
      ) : null}
      {error ? (
        <span id={errorId} role="alert" className="fm-text--caption fm-tone--danger">
          {error}
        </span>
      ) : null}
    </div>
  );
}

export function NumberStepper({
  label,
  value,
  min,
  max,
  onChange,
  disabled = false,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  onChange: (next: number) => void;
  disabled?: boolean;
}) {
  return (
    <div className="fm-field">
      <span className="fm-field-label">{label}</span>
      <div className="fm-stepper" role="group" aria-label={label}>
        <Button
          size="sm"
          variant="ghost"
          aria-label="减少数量"
          disabled={disabled || value <= min}
          onClick={() => onChange(Math.max(min, value - 1))}
        >
          -
        </Button>
        <span className="fm-stepper-value" aria-live="polite">
          {value}
        </span>
        <Button
          size="sm"
          variant="ghost"
          aria-label="增加数量"
          disabled={disabled || value >= max}
          onClick={() => onChange(Math.min(max, value + 1))}
        >
          +
        </Button>
      </div>
    </div>
  );
}

export interface TabOption<T extends string> {
  value: T;
  label: string;
  count?: number;
}

export function Tabs<T extends string>({
  items,
  value,
  onChange,
  ariaLabel,
}: {
  items: TabOption<T>[];
  value: T;
  onChange: (next: T) => void;
  ariaLabel: string;
}) {
  return (
    <div className="fm-tabs" role="tablist" aria-label={ariaLabel}>
      {items.map((item) => {
        const selected = item.value === value;
        return (
          <button
            key={item.value}
            type="button"
            role="tab"
            aria-selected={selected}
            className="fm-tab"
            onClick={() => onChange(item.value)}
          >
            <span>{item.label}</span>
            {typeof item.count === 'number' ? (
              <span className="tabular-mono">{item.count}</span>
            ) : null}
          </button>
        );
      })}
    </div>
  );
}
