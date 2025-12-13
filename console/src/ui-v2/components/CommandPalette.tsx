"use client"

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import Link from 'next/link'

type Command = {
  id: string
  label: string
  href: string
  keywords?: string[]
}

const COMMANDS: Command[] = [
  { id: 'dashboard', label: 'Go to Dashboard', href: '/dashboard', keywords: ['home', 'start'] },
  { id: 'billing', label: 'Open Billing → Usage', href: '/billing/usage', keywords: ['invoices', 'usage', 'payments'] },
  { id: 'transparency', label: 'Open Transparency → Auctions', href: '/transparency/auctions', keywords: ['receipts', 'hash', 'signature'] },
  { id: 'settings', label: 'Open Settings', href: '/settings', keywords: ['preferences', 'account'] },
  // Admin panel removed - operator cockpit is not for customers
  // Operators access directly via /admin/login
]

function match(q: string, cmd: Command): boolean {
  if (!q) return true
  const s = q.toLowerCase()
  return (
    cmd.label.toLowerCase().includes(s) ||
    cmd.href.toLowerCase().includes(s) ||
    (cmd.keywords || []).some((k) => k.toLowerCase().includes(s))
  )
}

export function CommandPalette() {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [activeIndex, setActiveIndex] = useState(0)
  const inputRef = useRef<HTMLInputElement | null>(null)

  const results = useMemo(() => COMMANDS.filter((c) => match(query, c)), [query])

  const onKeyDownGlobal = useCallback((e: KeyboardEvent) => {
    const metaOrCtrl = e.ctrlKey || e.metaKey
    if (metaOrCtrl && (e.key === 'k' || e.key === 'K')) {
      e.preventDefault()
      setOpen((v) => !v)
    }
    if (e.key === 'Escape') {
      setOpen(false)
    }
  }, [])

  useEffect(() => {
    window.addEventListener('keydown', onKeyDownGlobal)
    return () => window.removeEventListener('keydown', onKeyDownGlobal)
  }, [onKeyDownGlobal])

  useEffect(() => {
    if (!open) return
    const id = window.setTimeout(() => inputRef.current?.focus(), 0)
    return () => window.clearTimeout(id)
  }, [open])

  const onDialogKey = useCallback(
    (e: React.KeyboardEvent) => {
      if (!results.length) return
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        setActiveIndex((i) => (i + 1) % results.length)
      } else if (e.key === 'ArrowUp') {
        e.preventDefault()
        setActiveIndex((i) => (i - 1 + results.length) % results.length)
      } else if (e.key === 'Enter') {
        const cmd = results[activeIndex]
        if (cmd) {
          // Allow default navigation by clicking the hidden link programmatically
          const link = document.getElementById(`cmd-link-${cmd.id}`) as HTMLAnchorElement | null
          link?.click()
          setOpen(false)
        }
      }
    },
    [results, activeIndex]
  )

  if (!open) return null

  return (
    <div className="fixed inset-0 z-[70] flex items-start justify-center bg-black/40 p-4 md:p-8">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="command-palette-label"
        aria-describedby="command-palette-desc"
        className="w-full max-w-xl rounded-[var(--radius-lg)] border border-zinc-200 bg-white shadow-[var(--shadow-md)] dark:border-zinc-800 dark:bg-zinc-900"
      >
        <div className="border-b border-zinc-200 p-3 dark:border-zinc-800">
          <label htmlFor="command-input" id="command-palette-label" className="sr-only">
            Command Palette
          </label>
          <p id="command-palette-desc" className="sr-only">
            Type to filter results. Use Up/Down to navigate and Enter to open. Press Escape to close.
          </p>
          <input
            id="command-input"
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onDialogKey}
            placeholder="Type a command or search (Ctrl/Cmd+K)"
            className="h-10 w-full rounded-[var(--radius-md)] border border-transparent bg-zinc-50 px-3 text-[var(--text-sm)] outline-none focus-visible:[box-shadow:var(--focus-ring)] dark:bg-zinc-800"
          />
        </div>
        <ul role="listbox" aria-label="Command results" className="max-h-80 overflow-auto py-2">
          {results.length === 0 ? (
            <li className="px-3 py-2 text-sm text-zinc-500">No results</li>
          ) : (
            results.map((cmd, idx) => {
              const active = idx === activeIndex
              return (
                <li key={cmd.id} role="option" aria-selected={active}>
                  <Link
                    id={`cmd-link-${cmd.id}`}
                    href={cmd.href}
                    className={
                      'mx-2 block rounded-[var(--radius-md)] px-3 py-2 text-sm transition-colors focus-visible:outline-none focus-visible:[box-shadow:var(--focus-ring)] ' +
                      (active
                        ? 'bg-zinc-100 text-zinc-900 dark:bg-zinc-800 dark:text-zinc-100'
                        : 'text-zinc-700 hover:bg-zinc-100 dark:text-zinc-300 dark:hover:bg-zinc-800')
                    }
                    onClick={() => setOpen(false)}
                  >
                    <span className="font-medium">{cmd.label}</span>
                    <span className="ml-2 text-xs text-zinc-500">{cmd.href}</span>
                  </Link>
                </li>
              )
            })
          )}
        </ul>
        <div className="border-t border-zinc-200 p-2 text-right text-xs text-zinc-500 dark:border-zinc-800">
          Press Esc to close • Ctrl/Cmd+K to toggle
        </div>
      </div>
    </div>
  )
}

export default CommandPalette
