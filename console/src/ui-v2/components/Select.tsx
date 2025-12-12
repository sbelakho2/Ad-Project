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
    <select
      ref={ref}
      className={clsx(
        'w-full appearance-none rounded-[var(--radius-md)] bg-[var(--color-surface)] text-[var(--color-fg)]',
        'border border-[var(--color-border)]',
        'focus-visible:outline-none focus-visible:[box-shadow:var(--focus-ring)]',
        'disabled:opacity-60 disabled:cursor-not-allowed',
        // Global CSS provides the dropdown arrow via background-image
        sizeCls(selectSize),
        invalid && 'border-[var(--color-danger)]',
        className
      )}
      aria-invalid={invalid || undefined}
      {...props}
    >
      {children}
    </select>
  )
})

export default Select
