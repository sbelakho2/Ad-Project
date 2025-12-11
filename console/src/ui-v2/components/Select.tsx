"use client"

import { forwardRef } from 'react'
import clsx from 'clsx'

export interface SelectProps extends Omit<React.SelectHTMLAttributes<HTMLSelectElement>, 'size'> {
  invalid?: boolean
  selectSize?: 'sm' | 'md' | 'lg'
}

function sizeCls(size: NonNullable<SelectProps['selectSize']>) {
  switch (size) {
    case 'sm':
      return 'h-8 px-3 text-[var(--text-sm)]'
    case 'lg':
      return 'h-11 px-4 text-[var(--text-lg)]'
    case 'md':
    default:
      return 'h-10 px-3 text-[var(--text-sm)]'
  }
}

export const Select = forwardRef<HTMLSelectElement, SelectProps>(function Select(
  { className, invalid, selectSize = 'md', children, ...props },
  ref
) {
  return (
    <div className={clsx('relative')}>
      <select
        ref={ref}
        className={clsx(
          'w-full appearance-none rounded-[var(--radius-md)] bg-[var(--color-surface)] text-[var(--color-fg)]',
          'border border-[var(--color-border)] pr-10',
          'focus-visible:outline-none focus-visible:[box-shadow:var(--focus-ring)]',
          'disabled:opacity-60 disabled:cursor-not-allowed',
          '!bg-none', // Override global CSS background-image since we use SVG caret
          sizeCls(selectSize),
          invalid && 'border-[var(--color-danger)]',
          className
        )}
        aria-invalid={invalid || undefined}
        {...props}
      >
        {children}
      </select>
      {/* caret */}
      <svg
        className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 h-4 w-4 text-[var(--color-muted)]"
        xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 20 20"
        fill="currentColor"
        aria-hidden
      >
        <path fillRule="evenodd" d="M5.23 7.21a.75.75 0 011.06.02L10 10.94l3.71-3.71a.75.75 0 111.08 1.04l-4.25 4.25a.75.75 0 01-1.08 0L5.21 8.27a.75.75 0 01.02-1.06z" clipRule="evenodd" />
      </svg>
    </div>
  )
})

export default Select
