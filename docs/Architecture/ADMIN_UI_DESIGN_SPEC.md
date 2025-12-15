# Admin Control Plane UI Design Specification

This document provides a comprehensive design specification based on the existing Console UI patterns. Use this as a reference when implementing the admin control plane interface.

---

## Table of Contents

1. [Layout Structure](#layout-structure)
2. [Color Palette](#color-palette)
3. [Typography](#typography)
4. [Component Patterns](#component-patterns)
5. [Chart Library & Patterns](#chart-library--patterns)
6. [Common UI Patterns](#common-ui-patterns)
7. [Page Structure Templates](#page-structure-templates)
8. [CSS Classes Reference](#css-classes-reference)

---

## Layout Structure

### App Shell Pattern

The console uses a **fixed sidebar + main content** layout:

```
┌─────────────────────────────────────────────────────────┐
│ ┌─────────┐ ┌─────────────────────────────────────────┐ │
│ │         │ │                                         │ │
│ │ Sidebar │ │           Main Content                  │ │
│ │  240px  │ │                                         │ │
│ │  fixed  │ │        (scrollable)                     │ │
│ │         │ │                                         │ │
│ │         │ │                                         │ │
│ └─────────┘ └─────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

### Sidebar Structure

```tsx
// Width: 240px fixed
// Background: white
// Border: right gray-100

<aside className="fixed top-0 left-0 bottom-0 w-[240px] bg-white border-r border-gray-100 z-40">
  {/* Logo Section */}
  <div className="px-5 py-6 border-b border-gray-100">
    {/* Logo + Brand Name */}
  </div>

  {/* Navigation */}
  <nav className="flex-1 px-3 py-5 space-y-4 overflow-y-auto">
    {/* Nav Sections */}
  </nav>

  {/* User Menu */}
  <div className="p-4 border-t border-gray-100">
    {/* User avatar + sign out */}
  </div>
</aside>
```

### Navigation Section Pattern

```tsx
<div className="space-y-1">
  {/* Section Label */}
  <div className="px-2 text-[11px] font-semibold uppercase tracking-[0.08em] text-gray-400">
    Section Name
  </div>
  
  {/* Nav Items */}
  <Link
    href="/path"
    className={`
      relative flex items-center gap-3 px-3 py-2.5 rounded-lg 
      text-[13px] font-medium tracking-tight transition-colors duration-150
      ${isActive 
        ? 'bg-primary-50 text-primary-700 ring-1 ring-primary-100' 
        : 'text-gray-600 hover:bg-gray-50 hover:text-gray-900'
      }
    `}
  >
    {isActive && (
      <span className="absolute left-0 top-1 bottom-1 w-0.5 rounded-full bg-primary-600" />
    )}
    <Icon className={`h-5 w-5 ${isActive ? 'text-primary-700' : 'text-gray-400'}`} />
    <span className="truncate">{label}</span>
  </Link>
</div>
```

### Main Content Area

```tsx
<div className="lg:pl-[240px]">
  <div className="min-h-screen bg-[#f9fafb]">
    {/* Page Header */}
    <header className="bg-white/90 backdrop-blur border-b">
      <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
        {/* Header content */}
      </div>
    </header>
    
    {/* Page Content */}
    <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8 space-y-10">
      {/* Sections */}
    </main>
  </div>
</div>
```

---

## Color Palette

### Brand Colors

```typescript
// Primary Blues (ApexMediation)
'primary-blue': '#1E3A5F',
'sunshine-yellow': '#F59E0B',
'pale-yellow': '#FCD34D',
'cream': '#FFF8F0',
'accent-red': '#DC2626',
'success-green': '#059669',
```

### Primary Scale

```typescript
primary: {
  50: '#EFF6FF',   // Light backgrounds, active states
  100: '#DBEAFE',  // Hover backgrounds
  200: '#BFDBFE',  // Borders
  300: '#93C5FD',
  400: '#60A5FA',
  500: '#3B82F6',  // Primary buttons
  600: '#2563EB',  // Primary text, links
  700: '#1D4ED8',  // Hover states
  800: '#1E3A5F',  // Brand primary
  900: '#172554',
}
```

### Semantic Colors

```typescript
// Success (Green)
success: {
  50: '#f0fdf4',
  500: '#22c55e',
  600: '#16a34a',
  700: '#15803d',
}

// Warning (Yellow/Amber)
warning: {
  50: '#fefce8',
  500: '#eab308',
  600: '#ca8a04',
  700: '#a16207',
}

// Danger (Red)
danger: {
  50: '#fef2f2',
  500: '#ef4444',
  600: '#dc2626',
  700: '#b91c1c',
}
```

### Fraud Detection Colors

```typescript
fraud: {
  low: '#2563eb',
  'low-bg': '#eff6ff',
  'low-border': '#bfdbfe',
  medium: '#b45309',
  'medium-bg': '#fef3c7',
  'medium-border': '#fcd34d',
  high: '#ea580c',
  'high-bg': '#fff7ed',
  'high-border': '#fdba74',
  critical: '#b91c1c',
  'critical-bg': '#fef2f2',
  'critical-border': '#fecaca',
}
```

### Chart Colors

```typescript
// Primary chart colors
const CHART_COLORS = {
  revenue: '#0ea5e9',      // Sky blue
  impressions: '#22c55e',  // Green
  ecpm: '#f59e0b',         // Amber
  fillRate: '#8b5cf6',     // Purple
  clicks: '#6b7280',       // Gray
}
```

---

## Typography

### Font Family

```typescript
fontFamily: {
  sans: ['Plus Jakarta Sans', 'Inter', 'ui-sans-serif', 'system-ui'],
  display: ['Plus Jakarta Sans', 'Inter', 'ui-sans-serif', 'system-ui'],
  mono: ['JetBrains Mono', 'Fira Code', 'ui-monospace'],
}
```

### Heading Styles

```css
h1 {
  @apply text-2xl font-semibold tracking-tight text-gray-900;
  letter-spacing: -0.02em;
  line-height: 1.3;
}

h2 {
  @apply text-lg font-semibold text-gray-900;
  line-height: 1.35;
}

h3 {
  @apply text-base font-semibold text-gray-900;
}

h4 {
  @apply text-sm font-semibold text-gray-900;
}
```

### Body Text

```css
p {
  @apply text-sm text-gray-700 leading-relaxed;
}

small {
  @apply text-xs text-gray-500;
}

/* Muted labels */
.muted-label {
  @apply text-xs font-medium text-gray-500 tracking-tight;
}
```

---

## Component Patterns

### Card Component

```tsx
// Standard card
<div className="card">
  {/* content */}
</div>

// CSS class
.card {
  @apply rounded-xl border border-gray-200 bg-white px-6 py-5 shadow-sm;
}

// Dense card variant
.card-dense {
  @apply rounded-lg border border-gray-200 bg-white px-4 py-4 shadow-sm;
}

// Error card
<div className="card border-danger-200 bg-danger-50 text-danger-800">
  {/* error content */}
</div>

// Empty state card
<div className="card border border-dashed border-gray-200 bg-white">
  {/* empty state */}
</div>
```

### Metric Card

```tsx
interface MetricCardProps {
  title: string
  value: string
  change?: number
  icon: React.ReactNode
  trend?: 'up' | 'down' | 'neutral'
  comparisonLabel?: string
}

<div className="card">
  <div className="flex items-start justify-between mb-3">
    <div className="leading-tight">
      <span className="text-xs font-medium text-gray-500 block">{title}</span>
      <span className="mt-2 text-2xl font-semibold text-gray-900 leading-tight block">
        {value}
      </span>
    </div>
    <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-primary-50 text-primary-600">
      {icon}
    </div>
  </div>
  
  {/* Change indicator */}
  <div className={`flex items-center text-sm font-medium ${trendColor}`}>
    {trend === 'up' && <ArrowUpRight className="mr-1 h-4 w-4" />}
    {trend === 'down' && <ArrowDownRight className="mr-1 h-4 w-4" />}
    <span>{changeText}</span>
  </div>
</div>
```

### Button Variants

```css
/* Base button */
.btn {
  @apply inline-flex items-center justify-center gap-2 rounded-lg px-4 py-2 
         text-sm font-medium text-gray-900 transition-colors duration-200 
         focus:outline-none focus-visible:ring-2 focus-visible:ring-offset-2 
         focus-visible:ring-primary-600;
}

.btn-sm {
  @apply h-9 px-3 text-sm;
}

/* Primary - solid blue */
.btn-primary {
  @apply bg-primary-600 text-white hover:bg-primary-700;
}

/* Secondary - gray */
.btn-secondary {
  @apply bg-gray-100 text-gray-900 hover:bg-gray-200;
}

/* Outline */
.btn-outline {
  @apply border border-gray-300 text-gray-800 hover:border-primary-300 hover:bg-primary-50;
}

/* Soft primary */
.btn-soft-primary {
  @apply bg-primary-50 text-primary-700 hover:bg-primary-100;
}

/* Danger */
.btn-danger {
  @apply bg-red-600 text-white hover:bg-red-700;
}
```

### Badge/Status Components

```css
/* Base badge */
.badge {
  @apply inline-flex items-center gap-2 rounded-full border border-gray-200 
         bg-white px-3 py-1 text-xs font-medium text-gray-700;
}

.badge-success {
  @apply border-green-200 bg-green-50 text-green-700;
}

.badge-warning {
  @apply border-amber-200 bg-amber-50 text-amber-700;
}

.badge-danger {
  @apply border-red-200 bg-red-50 text-red-700;
}
```

### Status Badge with Icon

```tsx
const STATUS_VARIANTS = {
  paid: {
    icon: CheckCircle,
    classes: 'bg-success-50 text-success-700 border-success-200',
  },
  open: {
    icon: Clock,
    classes: 'bg-primary-50 text-primary-700 border-primary-200',
  },
  draft: {
    icon: FileText,
    classes: 'bg-warning-50 text-warning-700 border-warning-200',
  },
  void: {
    icon: XCircle,
    classes: 'bg-gray-100 text-gray-700 border-gray-200',
  },
  uncollectible: {
    icon: AlertCircle,
    classes: 'bg-danger-50 text-danger-700 border-danger-200',
  },
}

<span className={`inline-flex items-center gap-1 border rounded font-medium px-2 py-0.5 text-xs ${statusClasses}`}>
  <Icon className="h-3.5 w-3.5" />
  <span>{label}</span>
</span>
```

### Form Inputs

```css
.input {
  @apply block w-full rounded-lg border border-gray-300 bg-white px-3 py-2 
         text-sm text-gray-900 shadow-sm;
  @apply focus:border-primary-500 focus:outline-none focus:ring-2 
         focus:ring-primary-500/20;
  @apply disabled:cursor-not-allowed disabled:bg-gray-50 disabled:text-gray-500;
}

/* Select with custom arrow */
select.input {
  @apply appearance-none bg-no-repeat;
  background-image: url("data:image/svg+xml,...");
  background-position: right 0.75rem center;
  background-size: 1rem 1rem;
  padding-right: 2.5rem;
}
```

### Spinner & Skeleton

```tsx
// Spinner
<Loader2 className="h-6 w-6 text-primary-600 animate-spin" />

// Skeleton
<div className="animate-pulse bg-gray-200 rounded-lg" style={{ width, height }} />

// Skeleton variants
const variantClasses = {
  text: 'rounded',
  circular: 'rounded-full',
  rectangular: 'rounded-lg',
}
```

---

## Chart Library & Patterns

### Library: Recharts

The console uses **Recharts** for all data visualization:

```tsx
import {
  LineChart,
  Line,
  AreaChart,
  Area,
  BarChart,
  Bar,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Legend,
  ResponsiveContainer,
} from 'recharts'
```

### Chart Container Pattern

```tsx
<div className="card">
  <h3 className="text-lg font-semibold mb-4">Chart Title</h3>
  <ResponsiveContainer width="100%" height={280}>
    {/* Chart component */}
  </ResponsiveContainer>
</div>
```

### Area Chart (Revenue)

```tsx
<ResponsiveContainer width="100%" height={height}>
  <AreaChart data={data}>
    <defs>
      <linearGradient id="colorRevenue" x1="0" y1="0" x2="0" y2="1">
        <stop offset="5%" stopColor="#0ea5e9" stopOpacity={0.3}/>
        <stop offset="95%" stopColor="#0ea5e9" stopOpacity={0}/>
      </linearGradient>
    </defs>
    <CartesianGrid strokeDasharray="3 3" stroke="#e5e7eb" />
    <XAxis 
      dataKey="date" 
      stroke="#6b7280"
      fontSize={12}
      tickFormatter={axisFormatter}
    />
    <YAxis 
      stroke="#6b7280"
      fontSize={12}
      tickFormatter={currencyFormatter}
    />
    <Tooltip 
      contentStyle={{ 
        backgroundColor: '#fff', 
        border: '1px solid #e5e7eb', 
        borderRadius: '8px' 
      }}
      labelFormatter={tooltipFormatter}
    />
    <Area 
      type="monotone" 
      dataKey="revenue" 
      stroke="#0ea5e9" 
      strokeWidth={2}
      fillOpacity={1} 
      fill="url(#colorRevenue)"
    />
  </AreaChart>
</ResponsiveContainer>
```

### Line Chart (eCPM, Fill Rate)

```tsx
<ResponsiveContainer width="100%" height={height}>
  <LineChart data={data}>
    <CartesianGrid strokeDasharray="3 3" stroke="#e5e7eb" />
    <XAxis dataKey="date" stroke="#6b7280" fontSize={12} />
    <YAxis stroke="#6b7280" fontSize={12} />
    <Tooltip 
      contentStyle={{ backgroundColor: '#fff', border: '1px solid #e5e7eb', borderRadius: '8px' }}
    />
    <Legend />
    <Line 
      type="monotone" 
      dataKey="ecpm" 
      stroke="#f59e0b" 
      strokeWidth={2}
      dot={{ fill: '#f59e0b', r: 4 }}
      activeDot={{ r: 6 }}
    />
  </LineChart>
</ResponsiveContainer>
```

### Bar Chart (Impressions)

```tsx
<ResponsiveContainer width="100%" height={height}>
  <BarChart data={data}>
    <CartesianGrid strokeDasharray="3 3" stroke="#e5e7eb" />
    <XAxis dataKey="date" stroke="#6b7280" fontSize={12} />
    <YAxis stroke="#6b7280" fontSize={12} />
    <Tooltip 
      contentStyle={{ backgroundColor: '#fff', border: '1px solid #e5e7eb', borderRadius: '8px' }}
    />
    <Legend />
    <Bar 
      dataKey="impressions" 
      fill="#22c55e" 
      radius={[4, 4, 0, 0]}
    />
  </BarChart>
</ResponsiveContainer>
```

### Chart Skeleton

```tsx
function ChartSkeleton({ height }: { height: number }) {
  return (
    <div
      className="w-full rounded-lg border border-dashed border-gray-200 bg-gray-50 animate-pulse"
      style={{ height }}
    />
  )
}
```

---

## Common UI Patterns

### Page Header Pattern

```tsx
<header className="bg-white/90 backdrop-blur border-b">
  <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
    <div className="flex flex-wrap items-center justify-between gap-4">
      <div className="space-y-1">
        <p className="text-xs font-medium text-gray-500">Kicker/Category</p>
        <h1 className="text-2xl font-semibold tracking-tight text-gray-900">
          Page Title
        </h1>
        <p className="text-sm text-gray-600">Page description</p>
      </div>
      <div className="flex items-center gap-3">
        {/* Actions: filters, export buttons, etc. */}
      </div>
    </div>
  </div>
</header>
```

### Date Range Selector

```tsx
<div className="flex items-center gap-2">
  <CalendarRange className="w-5 h-5 text-gray-500" />
  <select
    value={`${dateRange.startDate}|${dateRange.endDate}`}
    onChange={handlePresetSelect}
    className="h-10 w-48 rounded-lg border border-gray-300 bg-white px-3 text-sm"
  >
    <option value={presets.last7}>Last 7 days</option>
    <option value={presets.last30}>Last 30 days</option>
    <option value={presets.last90}>Last 90 days</option>
  </select>
</div>
```

### Metrics Grid (4 columns)

```tsx
<section>
  <h2 className="sr-only">Key performance indicators</h2>
  <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-6">
    <MetricCard title="Total Revenue" value="$12,345" icon={<DollarSign />} change={5.2} />
    <MetricCard title="Impressions" value="1.2M" icon={<Eye />} change={-2.1} />
    <MetricCard title="eCPM" value="$4.50" icon={<TrendingUp />} change={3.8} />
    <MetricCard title="Fill Rate" value="85%" icon={<MousePointer />} change={1.5} />
  </div>
</section>
```

### Charts Grid (2 columns)

```tsx
<section className="space-y-6">
  <h2 className="text-xl font-semibold text-gray-900">Revenue Intelligence</h2>
  <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
    <div className="card">
      <h3 className="text-lg font-semibold mb-4">Revenue Trend</h3>
      <RevenueAreaChart data={data} height={280} />
    </div>
    <div className="card">
      <h3 className="text-lg font-semibold mb-4">eCPM Trend</h3>
      <EcpmLineChart data={data} height={280} />
    </div>
  </div>
</section>
```

### Widget Grid (3 columns)

```tsx
<section>
  <div className="grid grid-cols-1 xl:grid-cols-3 gap-6">
    <FraudWidget />
    <PayoutWidget />
    <QuickActionsPanel />
  </div>
</section>
```

### Error State Pattern

```tsx
<div className="card border-red-200 bg-red-50 text-red-900">
  <div className="flex flex-col gap-3">
    <div>
      <h3 className="text-lg font-semibold">Error Title</h3>
      <p className="text-sm">{errorMessage}</p>
    </div>
    <div>
      <button type="button" className="btn btn-outline btn-sm" onClick={refetch}>
        Retry
      </button>
    </div>
  </div>
</div>
```

### Empty State Pattern

```tsx
<div className="card border border-dashed border-gray-200 bg-white">
  <div className="flex flex-col gap-3 text-center py-10">
    <Icon className="w-12 h-12 mx-auto mb-2 text-gray-300" />
    <h3 className="text-lg font-semibold text-gray-900">No data yet</h3>
    <p className="text-sm text-gray-600">
      Helpful message explaining why there's no data
    </p>
    <div className="flex justify-center">
      <button className="btn btn-outline btn-sm">Refresh</button>
    </div>
  </div>
</div>
```

### Loading State Pattern

```tsx
// Full page loading
<div className="min-h-screen flex items-center justify-center bg-gray-50">
  <div className="text-sm text-gray-600">Loading...</div>
</div>

// Skeleton cards
<MetricCardSkeleton count={4} />

// Skeleton table
<div className="animate-pulse space-y-4">
  <div className="h-10 bg-gray-200 rounded"></div>
  <div className="h-10 bg-gray-200 rounded"></div>
  <div className="h-10 bg-gray-200 rounded"></div>
</div>
```

### Pagination Pattern

```tsx
<div className="flex flex-wrap items-center gap-3 justify-between">
  <div className="flex items-center gap-2">
    <button className="px-3 py-2 rounded border text-sm disabled:opacity-50">First</button>
    <button className="px-3 py-2 rounded border text-sm disabled:opacity-50">Previous</button>
  </div>
  <div className="flex items-center gap-2">
    <button className="px-3 py-2 rounded border text-sm disabled:opacity-50">Next</button>
    <button className="px-3 py-2 rounded border text-sm disabled:opacity-50">Last</button>
  </div>
</div>
```

### Table Pattern

```tsx
<div className="overflow-x-auto rounded border">
  <table className="min-w-full divide-y divide-gray-200">
    <thead className="bg-gray-50">
      <tr>
        <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">
          Column
        </th>
      </tr>
    </thead>
    <tbody className="divide-y divide-gray-100 bg-white">
      <tr>
        <td className="px-4 py-3 text-sm text-gray-900">Value</td>
      </tr>
    </tbody>
  </table>
</div>
```

---

## Page Structure Templates

### Dashboard Page Template

```tsx
export default function DashboardPage() {
  return (
    <div className="min-h-screen bg-[#f9fafb]">
      {/* Page Header */}
      <header className="bg-white/90 backdrop-blur border-b">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
          {/* Title + Actions */}
        </div>
      </header>

      {/* Main Content */}
      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8 space-y-10">
        {/* Metrics Section */}
        <section>
          <h2 className="sr-only">Key metrics</h2>
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-6">
            {/* MetricCards */}
          </div>
        </section>

        {/* Charts Section */}
        <section className="space-y-6">
          <h2 className="text-xl font-semibold text-gray-900">Section Title</h2>
          <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
            {/* Chart cards */}
          </div>
        </section>

        {/* Widgets Section */}
        <section>
          <div className="grid grid-cols-1 xl:grid-cols-3 gap-6">
            {/* Widget cards */}
          </div>
        </section>
      </main>
    </div>
  )
}
```

### List/Table Page Template

```tsx
export default function ListPage() {
  return (
    <div className="min-h-screen bg-[#f9fafb]">
      <header className="bg-white border-b">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
          <div className="flex items-center justify-between">
            <div>
              <h1 className="text-2xl font-semibold">Page Title</h1>
              <p className="text-sm text-gray-600 mt-1">Description</p>
            </div>
            <div className="flex items-center gap-3">
              {/* Filters */}
              <Filters status={status} onStatusChange={setStatus} />
            </div>
          </div>
        </div>
      </header>

      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        {/* Summary Cards */}
        <div className="grid grid-cols-4 gap-6 mb-8">
          {/* Stats */}
        </div>

        {/* Table */}
        <div className="card">
          <table className="min-w-full">
            {/* Table content */}
          </table>
        </div>

        {/* Pagination */}
        <div className="mt-6">
          <Pagination page={page} totalPages={totalPages} onPageChange={setPage} />
        </div>
      </main>
    </div>
  )
}
```

---

## CSS Classes Reference

### Quick Reference

| Class | Usage |
|-------|-------|
| `.card` | Standard container with border, shadow |
| `.card-dense` | Compact card variant |
| `.btn` | Base button styles |
| `.btn-primary` | Primary blue button |
| `.btn-secondary` | Gray button |
| `.btn-outline` | Outlined button |
| `.btn-danger` | Red button |
| `.btn-sm` | Small button |
| `.badge` | Base badge |
| `.badge-success` | Green badge |
| `.badge-warning` | Amber badge |
| `.badge-danger` | Red badge |
| `.input` | Form input styles |
| `.muted-label` | Small gray label text |

### Spacing Conventions

- Page padding: `px-4 sm:px-6 lg:px-8`
- Section spacing: `space-y-10` or `py-8`
- Card padding: `px-6 py-5`
- Grid gaps: `gap-6`
- Header padding: `py-6`

### Responsive Breakpoints

- `sm`: 640px
- `md`: 768px (2-column grids)
- `lg`: 1024px (4-column grids, sidebar visible)
- `xl`: 1280px (3-column widget grids)

### Animation Classes

```css
.animate-fade-in { animation: fadeIn 0.3s ease-in-out; }
.animate-slide-up { animation: slideUp 0.3s ease-out; }
.animate-slide-down { animation: slideDown 0.3s ease-out; }
.animate-pulse { /* Tailwind default */ }
.animate-spin { /* Tailwind default - for loaders */ }
```

---

## Implementation Checklist

When building admin control plane UI:

- [ ] Use `Recharts` for all charts
- [ ] Follow the sidebar + main content layout
- [ ] Use `@tanstack/react-query` for data fetching
- [ ] Implement loading skeletons
- [ ] Add error states with retry buttons
- [ ] Add empty states with helpful messages
- [ ] Use semantic HTML with proper ARIA labels
- [ ] Make layouts responsive (mobile → desktop)
- [ ] Use the established color palette
- [ ] Follow typography hierarchy
- [ ] Use Lucide icons consistently
- [ ] Apply consistent card styling
- [ ] Use the `.btn-*` classes for buttons
- [ ] Use the `.badge-*` classes for status indicators

---

*Last Updated: December 2025*
*Based on console/src analysis*
