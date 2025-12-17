import type { ReactNode } from 'react';

/**
 * Prose wrapper to normalize markdown/MDX typography.
 * - Ensures ul/ol use clean markers (no embedded bullets)
 * - Tables are styled and horizontally scrollable on small screens
 */
export default function Prose({ children, className = '' }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={[
        'prose prose-slate max-w-none',
        // Reset lists to avoid double bullets when content contains stray characters
        '[&_.remove-bullet]:list-none [&_.remove-bullet]:pl-0',
        // Tables: wrap in overflow container
        '[&>table]:w-full [&>table]:text-sm [&>table]:border-collapse',
        '[&>table_th]:text-left [&>table_th]:font-semibold [&>table_th]:border-b [&>table_th]:border-slate-200',
        '[&>table_td]:border-b [&>table_td]:border-slate-100 [&>table_td]:py-2',
        className,
      ].join(' ')}
    >
      <div className="overflow-x-auto">
        {children}
      </div>
    </div>
  );
}
