import type { ReactNode } from 'react';

export function StepCard({
  step,
  title,
  children,
  className,
}: {
  step: number | string;
  title: string;
  children?: ReactNode;
  className?: string;
}) {
  return (
    <div className={[
      'flex items-start gap-4',
      'rounded-xl border border-gray-200 bg-white p-4',
      className,
    ].filter(Boolean).join(' ')}>
      <div
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-brand-600 text-white font-semibold"
        aria-hidden="true"
      >
        <span>{step}</span>
      </div>
      <div className="min-w-0">
        <h4 className="text-base font-semibold text-gray-900">{title}</h4>
        {children ? (
          <p className="mt-1 text-sm text-gray-700 leading-relaxed">{children}</p>
        ) : null}
      </div>
    </div>
  );
}
