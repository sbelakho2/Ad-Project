import type { ReactNode } from 'react';

export function Badge({
  children,
  variant = 'default',
  className,
}: {
  children: ReactNode;
  variant?: 'default' | 'success' | 'warning' | 'danger' | 'muted';
  className?: string;
}) {
  const styles = {
    default: 'text-ink bg-gray-100 border border-gray-200',
    success: 'text-success bg-success/10 border border-success/20',
    warning: 'text-warning bg-warning/10 border border-warning/20',
    danger: 'text-danger bg-danger/10 border border-danger/20',
    muted: 'text-inkMuted bg-gray-50 border border-gray-200',
  } as const;

  return (
    <span
      className={[
        'inline-flex items-center rounded-full px-2.5 py-1 text-xs font-medium',
        styles[variant],
        className,
      ].filter(Boolean).join(' ')}
    >
      {children}
    </span>
  );
}
