# Sensei-Rams Implementation Guide (v3.0)

> **Technical specification for implementing the Sensei-Rams Industrial Functionalist design system.**
> 
> This document provides exhaustive code-level guidance. Every developer must follow these patterns to ensure the interface maintains its proprietary "Industrial Instrument" aesthetic and high-precision rendering.

---

## 1. Project Configuration

### 1.1 Tailwind Configuration

The `tailwind.config.ts` must be locked to these specific values. **Do not use generic Tailwind colors** (e.g., `slate`, `zinc`, `gray`).

```typescript
// tailwind.config.ts
import type { Config } from 'tailwindcss'

const config: Config = {
  content: ['./src/**/*.{js,ts,jsx,tsx,mdx}'],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        rams: {
          // Core Warm-Grey Scale (Braun-derived)
          chassis: 'var(--rams-chassis)',     // #F2F2F2 light / #1A1A1A dark
          module: 'var(--rams-module)',       // #E6E6E6 light / #252525 dark
          panel: 'var(--rams-panel)',         // #D9D9D9 light / #2D2D2D dark
          line: 'var(--rams-line)',           // #CCCCCC light / #404040 dark
          muted: 'var(--rams-muted)',         // #999999 light / #666666 dark
          
          // Functional Accents (Semantic only)
          orange: 'var(--rams-accent, var(--rams-orange))', // #FFBE00 - Primary action
          green: 'var(--rams-green)',         // #2D8C3C - Success/operational
          red: 'var(--rams-red)',             // #D62D2D - Error/critical
          steel: 'var(--rams-steel)',         // #4A90E2 - Information
        },
      },
      borderRadius: {
        'rams-none': '0px',
        'rams-sm': '2px',
        'rams-md': '4px',
        'rams-lg': '8px',  // Maximum allowed
      },
      spacing: {
        'rams-1': '4px',
        'rams-2': '8px',
        'rams-3': '12px',
        'rams-4': '16px',
        'rams-6': '24px',
        'rams-8': '32px',
        'rams-12': '48px',
        'rams-16': '64px',
      },
      fontFamily: {
        sans: ['var(--font-geist-sans)', 'Inter', 'system-ui', 'sans-serif'],
        mono: ['var(--font-geist-mono)', 'JetBrains Mono', 'SF Mono', 'Consolas', 'monospace'],
      },
      fontSize: {
        '3xs': ['8px', { lineHeight: '12px' }],
        '2xs': ['10px', { lineHeight: '14px' }],
      },
      boxShadow: {
        'rams-inset': 'inset 1px 1px 0 rgba(255,255,255,0.5), inset -1px -1px 0 rgba(0,0,0,0.05)',
        'rams-pressed': 'inset 0 2px 4px rgba(0,0,0,0.1)',
        'rams-focus': '0 0 0 2px rgba(255,190,0,0.3)',
      },
      transitionDuration: {
        'rams-instant': '50ms',
        'rams-fast': '100ms',
        'rams-normal': '150ms',
        'rams-slow': '200ms',
      },
    },
  },
  plugins: [],
}

export default config
```

### 1.2 CSS Custom Properties

Define in `globals.css`:

```css
@tailwind base;
@tailwind components;
@tailwind utilities;

@layer base {
  :root {
    /* Core Warm-Grey Scale */
    --rams-chassis: #F2F2F2;
    --rams-module: #E6E6E6;
    --rams-panel: #D9D9D9;
    --rams-line: #CCCCCC;
    --rams-muted: #999999;
    --rams-foreground: #1A1A1A;
    
    /* Functional Accents */
    --rams-orange: #FFBE00;
    --rams-green: #2D8C3C;
    --rams-red: #D62D2D;
    --rams-steel: #4A90E2;
    
    /* User-customizable accent (defaults to orange) */
    --rams-accent: var(--rams-orange);
    
    /* Typography */
    --font-weight-normal: 400;
    --font-weight-medium: 500;
    --font-weight-semibold: 600;
    --font-weight-bold: 700;
  }
  
  .dark {
    --rams-chassis: #1A1A1A;
    --rams-module: #252525;
    --rams-panel: #2D2D2D;
    --rams-line: #404040;
    --rams-muted: #666666;
    --rams-foreground: #F2F2F2;
  }
  
  /* Anti-blur text rendering */
  body {
    text-rendering: optimizeLegibility;
    -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
    font-feature-settings: "kern" 1, "liga" 1;
    background-color: var(--rams-chassis);
    color: var(--rams-foreground);
  }
  
  /* Tabular figures for data */
  [data-numeric], .tabular-nums {
    font-variant-numeric: tabular-nums lining-nums;
    font-feature-settings: "tnum" 1, "lnum" 1;
  }
}

/* Density modes */
@layer utilities {
  .density-compact {
    --density-scale: 0.75;
    --font-scale: 0.9;
  }
  
  .density-comfortable {
    --density-scale: 1;
    --font-scale: 1;
  }
  
  .density-expanded {
    --density-scale: 1.25;
    --font-scale: 1.1;
  }
}

/* Reduced motion support */
@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
    scroll-behavior: auto !important;
  }
}
```

---

## 2. Layout System: The Control Station

### 2.1 Root Layout with Industrial Bezel

```tsx
// app/layout.tsx
import { GeistSans } from 'geist/font/sans'
import { GeistMono } from 'geist/font/mono'

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={`${GeistSans.variable} ${GeistMono.variable}`}>
      <body className="bg-rams-chassis text-rams-foreground min-h-screen">
        {/* Industrial Bezel Frame */}
        <div 
          className="fixed inset-0 border-[8px] border-rams-chassis pointer-events-none z-[100] hidden md:block" 
          aria-hidden="true" 
        />
        
        {/* Corner Screws */}
        <ScrewHead className="top-1 left-1" />
        <ScrewHead className="top-1 right-1" />
        <ScrewHead className="bottom-9 left-1" />
        <ScrewHead className="bottom-9 right-1" />
        
        {/* Main Content */}
        <main className="min-h-screen pb-8 md:pb-0">
          {children}
        </main>
        
        {/* Status Bar */}
        <StatusBar />
      </body>
    </html>
  )
}

function ScrewHead({ className }: { className: string }) {
  return (
    <div className={cn(
      "fixed z-[101] hidden md:block opacity-30 select-none",
      className
    )}>
      <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
        <circle cx="6" cy="6" r="5" fill="none" stroke="currentColor" strokeWidth="1" />
        <path d="M3 6L9 6M6 3L6 9" stroke="currentColor" strokeWidth="1" />
      </svg>
    </div>
  )
}

function StatusBar() {
  return (
    <div className="fixed bottom-0 left-0 right-0 h-8 bg-rams-chassis z-[100] border-t border-rams-line px-6 hidden md:flex items-center justify-between text-2xs font-mono text-rams-muted uppercase tracking-widest">
      <div className="flex gap-6">
        <span>STATION: SENSEI-ALPHA-01</span>
        <span>OS_VER: 3.0.1-RAMS</span>
      </div>
      <div className="flex gap-6 text-right">
        <span>INTEGRITY: OPTIMAL</span>
        <SystemClock />
      </div>
    </div>
  )
}
```

### 2.2 The Rack Sidebar

```tsx
// components/layout/rack-sidebar.tsx
'use client'

import { cn } from '@/lib/utils'
import { usePathname } from 'next/navigation'
import Link from 'next/link'

interface NavModule {
  id: string
  label: string
  labelKey: string
  href: string
  icon: React.ComponentType<{ className?: string }>
}

export function RackSidebar({ modules }: { modules: NavModule[] }) {
  const pathname = usePathname()
  
  return (
    <aside className="w-60 h-full bg-rams-module border-r border-rams-line flex flex-col">
      {/* Station Identifier */}
      <div className="h-14 border-b border-rams-line flex items-center px-4">
        <div className="flex items-center gap-3">
          <div className="w-8 h-8 bg-rams-panel border border-rams-line rounded-rams-sm flex items-center justify-center">
            <span className="font-mono font-bold text-sm">▣</span>
          </div>
          <div>
            <div className="font-semibold text-sm tracking-tight">SENSEI OS</div>
            <div className="text-2xs font-mono text-rams-muted uppercase">CONTROL STATION</div>
          </div>
        </div>
      </div>
      
      {/* Module Rack */}
      <nav className="flex-1 p-2 space-y-1">
        {modules.map((module) => {
          const isActive = pathname.startsWith(module.href)
          const Icon = module.icon
          
          return (
            <Link
              key={module.id}
              href={module.href}
              className={cn(
                // Base module slot
                "flex items-center gap-3 px-3 py-2 rounded-rams-sm border transition-colors duration-rams-fast",
                // Inactive state
                "border-transparent hover:border-rams-line hover:bg-rams-panel",
                // Active state - orange indicator
                isActive && "border-rams-line bg-rams-panel shadow-[inset_3px_0_0_0_var(--rams-orange)]"
              )}
            >
              {/* Activity indicator */}
              <div className={cn(
                "w-2 h-2 rounded-full transition-colors",
                isActive ? "bg-rams-orange" : "bg-rams-muted/30"
              )} />
              
              <Icon className={cn(
                "w-4 h-4",
                isActive ? "text-rams-foreground" : "text-rams-muted"
              )} />
              
              <span className={cn(
                "text-sm font-medium",
                isActive ? "text-rams-foreground" : "text-rams-muted"
              )}>
                {module.label}
              </span>
            </Link>
          )
        })}
      </nav>
      
      {/* System Status */}
      <div className="p-4 border-t border-rams-line">
        <div className="flex items-center gap-2">
          <AndonStack status="green" />
          <span className="text-2xs font-mono uppercase text-rams-green">SYSTEM OPTIMAL</span>
        </div>
      </div>
    </aside>
  )
}
```

### 2.3 Module Container (Not Card)

```tsx
// components/ui/module.tsx
import { cn } from '@/lib/utils'
import { forwardRef } from 'react'

interface ModuleProps extends React.HTMLAttributes<HTMLDivElement> {
  variant?: 'default' | 'inset' | 'raised'
}

const Module = forwardRef<HTMLDivElement, ModuleProps>(
  ({ className, variant = 'default', children, ...props }, ref) => {
    return (
      <div
        ref={ref}
        className={cn(
          // Base module styling
          "bg-rams-module border border-rams-line rounded-rams-sm",
          // Variant-specific
          variant === 'inset' && "bg-rams-panel shadow-rams-inset",
          variant === 'raised' && "border-2",
          className
        )}
        {...props}
      >
        {children}
      </div>
    )
  }
)
Module.displayName = 'Module'

const ModuleHeader = forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>(
  ({ className, children, ...props }, ref) => {
    return (
      <div
        ref={ref}
        className={cn(
          "px-4 py-3 border-b border-rams-line flex items-center justify-between",
          className
        )}
        {...props}
      >
        {children}
      </div>
    )
  }
)
ModuleHeader.displayName = 'ModuleHeader'

const ModuleTitle = forwardRef<HTMLHeadingElement, React.HTMLAttributes<HTMLHeadingElement>>(
  ({ className, children, ...props }, ref) => {
    return (
      <h3
        ref={ref}
        className={cn(
          "text-2xs font-mono font-bold uppercase tracking-widest text-rams-muted",
          className
        )}
        {...props}
      >
        {children}
      </h3>
    )
  }
)
ModuleTitle.displayName = 'ModuleTitle'

const ModuleContent = forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>(
  ({ className, children, ...props }, ref) => {
    return (
      <div
        ref={ref}
        className={cn("p-4", className)}
        {...props}
      >
        {children}
      </div>
    )
  }
)
ModuleContent.displayName = 'ModuleContent'

export { Module, ModuleHeader, ModuleTitle, ModuleContent }
```

---

## 3. Component Patterns

### 3.1 Industrial Button

```tsx
// components/ui/button.tsx
import { cn } from '@/lib/utils'
import { forwardRef } from 'react'

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: 'default' | 'primary' | 'danger' | 'ghost'
  size?: 'sm' | 'md' | 'lg'
}

const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant = 'default', size = 'md', children, ...props }, ref) => {
    return (
      <button
        ref={ref}
        className={cn(
          // Base mechanical button
          "inline-flex items-center justify-center gap-2",
          "font-medium rounded-rams-sm border",
          "transition-all duration-rams-instant",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rams-orange focus-visible:ring-offset-2",
          "disabled:opacity-50 disabled:cursor-not-allowed",
          // Active state - pressed effect
          "active:scale-[0.98] active:shadow-rams-pressed",
          
          // Size variants
          size === 'sm' && "h-8 px-3 text-sm",
          size === 'md' && "h-10 px-4 text-sm",
          size === 'lg' && "h-12 px-6 text-base",
          
          // Color variants
          variant === 'default' && [
            "bg-rams-module border-rams-line text-rams-foreground",
            "hover:bg-rams-panel hover:border-rams-muted",
          ],
          variant === 'primary' && [
            "bg-rams-orange border-rams-orange text-black",
            "hover:brightness-110",
          ],
          variant === 'danger' && [
            "bg-rams-red border-rams-red text-white",
            "hover:brightness-110",
          ],
          variant === 'ghost' && [
            "bg-transparent border-transparent text-rams-foreground",
            "hover:bg-rams-panel hover:border-rams-line",
          ],
          
          className
        )}
        {...props}
      >
        {children}
      </button>
    )
  }
)
Button.displayName = 'Button'

export { Button }
```

### 3.2 Mechanical Toggle Switch

```tsx
// components/ui/toggle-switch.tsx
'use client'

import { cn } from '@/lib/utils'
import { forwardRef } from 'react'

interface ToggleSwitchProps {
  checked: boolean
  onCheckedChange: (checked: boolean) => void
  disabled?: boolean
  label?: string
  className?: string
}

export const ToggleSwitch = forwardRef<HTMLButtonElement, ToggleSwitchProps>(
  ({ checked, onCheckedChange, disabled, label, className }, ref) => {
    return (
      <button
        ref={ref}
        role="switch"
        aria-checked={checked}
        aria-label={label}
        disabled={disabled}
        onClick={() => onCheckedChange(!checked)}
        className={cn(
          // Track
          "relative w-12 h-6 rounded-rams-sm border border-rams-line",
          "bg-rams-panel transition-colors duration-rams-fast",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rams-orange",
          "disabled:opacity-50 disabled:cursor-not-allowed",
          checked && "bg-rams-orange/20 border-rams-orange",
          className
        )}
      >
        {/* Knob */}
        <div
          className={cn(
            "absolute top-1 w-4 h-4 rounded-full",
            "bg-rams-foreground border border-rams-line",
            "transition-transform duration-rams-fast",
            "shadow-sm",
            checked ? "translate-x-7" : "translate-x-1"
          )}
        />
        
        {/* Track indicators */}
        <div className="absolute inset-x-2 top-1/2 -translate-y-1/2 flex justify-between text-3xs font-mono">
          <span className={cn("transition-opacity", checked && "opacity-30")}>○</span>
          <span className={cn("transition-opacity", !checked && "opacity-30")}>●</span>
        </div>
      </button>
    )
  }
)
ToggleSwitch.displayName = 'ToggleSwitch'
```

### 3.3 Industrial Input

```tsx
// components/ui/input.tsx
import { cn } from '@/lib/utils'
import { forwardRef } from 'react'

interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  label?: string
  error?: string
}

const Input = forwardRef<HTMLInputElement, InputProps>(
  ({ className, label, error, type = 'text', ...props }, ref) => {
    return (
      <div className="space-y-1">
        {label && (
          <label className="text-2xs font-mono font-bold uppercase tracking-widest text-rams-muted">
            {label}
          </label>
        )}
        <input
          type={type}
          ref={ref}
          className={cn(
            // Base input - instrument display window
            "w-full h-10 px-3 rounded-rams-sm border",
            "bg-rams-panel text-rams-foreground",
            "font-mono text-sm",
            "shadow-rams-inset",
            "placeholder:text-rams-muted/60",
            // Focus state
            "focus:outline-none focus:border-rams-orange focus:shadow-rams-focus",
            // Error state
            error ? "border-rams-red" : "border-rams-line",
            // Disabled state
            "disabled:opacity-50 disabled:cursor-not-allowed",
            className
          )}
          {...props}
        />
        {error && (
          <p className="text-xs text-rams-red font-medium">{error}</p>
        )}
      </div>
    )
  }
)
Input.displayName = 'Input'

export { Input }
```

### 3.4 Andon Status Stack

```tsx
// components/ui/andon-stack.tsx
import { cn } from '@/lib/utils'

type AndonStatus = 'green' | 'yellow' | 'red' | 'off'

interface AndonStackProps {
  status: AndonStatus
  size?: 'sm' | 'md' | 'lg'
  className?: string
}

export function AndonStack({ status, size = 'sm', className }: AndonStackProps) {
  const sizes = {
    sm: 'w-3 h-3',
    md: 'w-4 h-4',
    lg: 'w-5 h-5',
  }
  
  return (
    <div 
      className={cn(
        "flex flex-col gap-1 p-1 bg-rams-panel border border-rams-line rounded-rams-sm",
        className
      )}
      role="status"
      aria-label={`System status: ${status}`}
    >
      <div 
        className={cn(
          sizes[size],
          "rounded-full transition-all duration-rams-fast",
          status === 'red' 
            ? "bg-rams-red shadow-[0_0_8px_rgba(214,45,45,0.5)]" 
            : "bg-rams-muted/20"
        )}
      />
      <div 
        className={cn(
          sizes[size],
          "rounded-full transition-all duration-rams-fast",
          status === 'yellow' 
            ? "bg-rams-orange shadow-[0_0_8px_rgba(255,190,0,0.5)]" 
            : "bg-rams-muted/20"
        )}
      />
      <div 
        className={cn(
          sizes[size],
          "rounded-full transition-all duration-rams-fast",
          status === 'green' 
            ? "bg-rams-green shadow-[0_0_8px_rgba(45,140,60,0.5)]" 
            : "bg-rams-muted/20"
        )}
      />
    </div>
  )
}
```

### 3.5 Dymo Label

```tsx
// components/ui/dymo-label.tsx
import { cn } from '@/lib/utils'

interface DymoLabelProps {
  children: React.ReactNode
  variant?: 'default' | 'warning' | 'critical'
  className?: string
}

export function DymoLabel({ children, variant = 'default', className }: DymoLabelProps) {
  return (
    <span
      className={cn(
        "inline-block px-2 py-0.5",
        "font-mono text-2xs font-bold uppercase tracking-widest",
        "rotate-[-0.5deg]",
        "select-none",
        // Variants
        variant === 'default' && "bg-black text-white",
        variant === 'warning' && "bg-rams-orange text-black",
        variant === 'critical' && "bg-rams-red text-white",
        className
      )}
    >
      {children}
    </span>
  )
}
```

### 3.6 Metric Display

```tsx
// components/ui/metric-display.tsx
import { cn } from '@/lib/utils'

interface MetricDisplayProps {
  label: string
  value: string | number
  unit?: string
  trend?: {
    direction: 'up' | 'down' | 'neutral'
    value: string
  }
  className?: string
}

export function MetricDisplay({ label, value, unit, trend, className }: MetricDisplayProps) {
  return (
    <div className={cn("space-y-1", className)}>
      {/* Label - Dymo style */}
      <div className="text-2xs font-mono font-bold uppercase tracking-widest text-rams-muted">
        {label}
      </div>
      
      {/* Value - Large instrument readout */}
      <div className="flex items-baseline gap-2">
        <span className="font-mono text-3xl font-bold tabular-nums tracking-tight">
          {value}
        </span>
        {unit && (
          <span className="text-sm font-mono text-rams-muted uppercase">
            {unit}
          </span>
        )}
      </div>
      
      {/* Trend indicator */}
      {trend && (
        <div className={cn(
          "flex items-center gap-1 text-xs font-medium",
          trend.direction === 'up' && "text-rams-green",
          trend.direction === 'down' && "text-rams-red",
          trend.direction === 'neutral' && "text-rams-muted"
        )}>
          <span>
            {trend.direction === 'up' && '▲'}
            {trend.direction === 'down' && '▼'}
            {trend.direction === 'neutral' && '●'}
          </span>
          <span>{trend.value}</span>
        </div>
      )}
    </div>
  )
}
```

---

## 4. Industrial Visual Effects

### 4.1 Perforated Grille Pattern

```css
/* In globals.css */
@layer utilities {
  .perforated-grille {
    background-image: radial-gradient(circle, var(--rams-muted) 1px, transparent 1px);
    background-size: 6px 6px;
  }
  
  .perforated-grille-fine {
    background-image: radial-gradient(circle, var(--rams-muted) 0.5px, transparent 0.5px);
    background-size: 4px 4px;
  }
  
  .perforated-grille-coarse {
    background-image: radial-gradient(circle, var(--rams-muted) 1.5px, transparent 1.5px);
    background-size: 8px 8px;
  }
}
```

### 4.2 Engineering Blueprint Grid

```css
@layer utilities {
  .blueprint-grid {
    background-image: 
      radial-gradient(circle, var(--rams-line) 0.5px, transparent 0.5px);
    background-size: 20px 20px;
  }
  
  .blueprint-grid-fine {
    background-image: 
      linear-gradient(var(--rams-line) 1px, transparent 1px),
      linear-gradient(90deg, var(--rams-line) 1px, transparent 1px);
    background-size: 4px 4px;
    opacity: 0.3;
  }
}
```

### 4.3 Stamp Overlay (Approval/Status)

```tsx
// components/ui/stamp.tsx
import { cn } from '@/lib/utils'

interface StampProps {
  children: React.ReactNode
  variant?: 'approved' | 'rejected' | 'pending'
  className?: string
}

export function Stamp({ children, variant = 'approved', className }: StampProps) {
  return (
    <div
      className={cn(
        "absolute top-4 right-4 px-4 py-2",
        "border-4 font-black uppercase tracking-widest",
        "rotate-[-12deg] opacity-60",
        "mix-blend-multiply dark:mix-blend-screen",
        "select-none pointer-events-none",
        // Variants
        variant === 'approved' && "border-rams-green text-rams-green",
        variant === 'rejected' && "border-rams-red text-rams-red",
        variant === 'pending' && "border-rams-orange text-rams-orange",
        className
      )}
      aria-hidden="true"
    >
      {children}
    </div>
  )
}
```

---

## 5. Data Table Pattern

### 5.1 Industrial Data Table

```tsx
// components/ui/data-table.tsx
import { cn } from '@/lib/utils'

interface Column<T> {
  key: keyof T
  header: string
  align?: 'left' | 'center' | 'right'
  mono?: boolean
}

interface DataTableProps<T> {
  columns: Column<T>[]
  data: T[]
  className?: string
}

export function DataTable<T extends Record<string, unknown>>({
  columns,
  data,
  className
}: DataTableProps<T>) {
  return (
    <div className={cn("border border-rams-line rounded-rams-sm overflow-hidden", className)}>
      <table className="w-full">
        <thead>
          <tr className="bg-rams-panel border-b border-rams-line">
            {columns.map((col) => (
              <th
                key={String(col.key)}
                className={cn(
                  "px-4 py-3",
                  "text-2xs font-mono font-bold uppercase tracking-widest text-rams-muted",
                  "text-left",
                  col.align === 'center' && "text-center",
                  col.align === 'right' && "text-right"
                )}
              >
                {col.header}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {data.map((row, i) => (
            <tr
              key={i}
              className={cn(
                "border-b border-rams-line last:border-b-0",
                "hover:bg-rams-panel transition-colors duration-rams-fast"
              )}
            >
              {columns.map((col) => (
                <td
                  key={String(col.key)}
                  className={cn(
                    "px-4 py-3 text-sm",
                    col.mono && "font-mono",
                    col.align === 'center' && "text-center",
                    col.align === 'right' && "text-right"
                  )}
                >
                  {String(row[col.key])}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
```

---

## 6. Accessibility Implementation

### 6.1 Focus Management

```css
/* Focus ring specification */
@layer base {
  :focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px var(--rams-orange);
  }
  
  /* Skip link for keyboard users */
  .skip-link {
    position: absolute;
    top: -40px;
    left: 0;
    padding: 8px 16px;
    background: var(--rams-orange);
    color: black;
    z-index: 9999;
    transition: top 0.2s;
  }
  
  .skip-link:focus {
    top: 0;
  }
}
```

### 6.2 ARIA Patterns

```tsx
// Example: Accessible module with proper ARIA
function AccessibleModule({ title, children }: { title: string; children: React.ReactNode }) {
  const headingId = useId()
  
  return (
    <section
      aria-labelledby={headingId}
      className="bg-rams-module border border-rams-line rounded-rams-sm"
    >
      <header className="px-4 py-3 border-b border-rams-line">
        <h2 
          id={headingId}
          className="text-2xs font-mono font-bold uppercase tracking-widest text-rams-muted"
        >
          {title}
        </h2>
      </header>
      <div className="p-4">
        {children}
      </div>
    </section>
  )
}
```

### 6.3 Color Blindness Considerations

```tsx
// Always pair color with text/icon
function StatusIndicator({ status }: { status: 'success' | 'error' | 'warning' }) {
  const config = {
    success: { color: 'text-rams-green', icon: '✓', label: 'Success' },
    error: { color: 'text-rams-red', icon: '✕', label: 'Error' },
    warning: { color: 'text-rams-orange', icon: '!', label: 'Warning' },
  }
  
  const { color, icon, label } = config[status]
  
  return (
    <span className={cn("flex items-center gap-1", color)}>
      <span aria-hidden="true">{icon}</span>
      <span>{label}</span>
    </span>
  )
}
```

---

## 7. Performance Guidelines

### 7.1 Asset Optimization

```typescript
// next.config.js
module.exports = {
  images: {
    formats: ['image/avif', 'image/webp'],
    minimumCacheTTL: 31536000, // 1 year
  },
  experimental: {
    optimizeCss: true,
  },
}
```

### 7.2 Bundle Analysis Targets

| Metric | Target | Critical |
|--------|--------|----------|
| First Contentful Paint | < 1.5s | < 2.5s |
| Largest Contentful Paint | < 2.5s | < 4.0s |
| Cumulative Layout Shift | < 0.1 | < 0.25 |
| Total Blocking Time | < 200ms | < 600ms |
| JavaScript Bundle | < 200KB | < 400KB |
| CSS Bundle | < 50KB | < 100KB |

### 7.3 Font Loading Strategy

```tsx
// Prevent FOUT with font preload
<head>
  <link
    rel="preload"
    href="/fonts/GeistVF.woff2"
    as="font"
    type="font/woff2"
    crossOrigin="anonymous"
  />
  <link
    rel="preload"
    href="/fonts/GeistMonoVF.woff2"
    as="font"
    type="font/woff2"
    crossOrigin="anonymous"
  />
</head>
```

---

## 8. RTL Support

### 8.1 Bidirectional Layout

```tsx
// RTL-aware component
function RTLAwareModule({ children }: { children: React.ReactNode }) {
  const { direction } = useI18n()
  
  return (
    <div 
      className="bg-rams-module border border-rams-line rounded-rams-sm"
      dir={direction}
    >
      {children}
    </div>
  )
}
```

### 8.2 Logical Properties

```css
/* Use logical properties for RTL support */
.module-content {
  padding-inline: 16px;  /* Not padding-left/right */
  margin-block: 8px;     /* Not margin-top/bottom */
  border-inline-start: 3px solid var(--rams-orange);  /* Not border-left */
}
```

---

## 9. Implementation Verification Checklist

Before any component ships, verify all items:

### Visual Fidelity
- [ ] Uses only `--rams-*` color tokens
- [ ] Border radius ≤ 8px (preferably 2-4px)
- [ ] No drop shadows (only `shadow-rams-inset` or `shadow-rams-pressed`)
- [ ] Spacing values divisible by 4px
- [ ] Font weight ≥ 400

### Interaction
- [ ] Hover state uses border/background change (not shadow)
- [ ] Active state uses `scale(0.98)` and inset shadow
- [ ] Focus state uses 2px orange ring with 2px offset
- [ ] Animation duration ≤ 200ms
- [ ] `prefers-reduced-motion` respected

### Accessibility
- [ ] Color contrast ≥ 4.5:1 for text
- [ ] Color contrast ≥ 3:1 for interactive elements
- [ ] Touch targets ≥ 44px
- [ ] ARIA labels present on interactive elements
- [ ] Keyboard navigation functional
- [ ] Screen reader tested

### Performance
- [ ] No layout shift on load
- [ ] Fonts preloaded
- [ ] Images optimized
- [ ] Bundle size within limits

---

*End of Implementation Specification*

**Document History:**
- v3.0.0 - Initial release
- v3.0.1 - Added accessibility section, RTL support, performance targets
