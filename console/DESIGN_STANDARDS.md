# Ad Platform Console Design Standards

**Version:** 1.1  
**Last Updated:** 2025-12-11  
Tailwind config sync: 72a748572d0f *(validated via `npm run design:verify`)*  
**Theme:** Aurora Slate — Tailwind-powered modern console

> Inspired by the TPS Co-Pilot "Neo-Industrial Nightfall" system while optimized for a data-dense SaaS console built with Next.js 14 and Tailwind CSS. This document is authoritative for all UI/UX decisions across the console.

---

## 🎨 Color System

All colors are defined in `tailwind.config.ts`. When extending Tailwind classes or authoring bespoke CSS, reference the tokens in this table.

### Brand Palette
| Token | Tailwind Path | Hex | Usage |
|-------|---------------|-----|-------|
| `primary-500` | `theme.colors.primary.500` | `#0ea5e9` | Actions, highlights, links |
| `primary-600` | `theme.colors.primary.600` | `#0284c7` | Primary buttons, hover |
| `primary-50`  | `theme.colors.primary.50`  | `#f0f9ff` | Soft background states |
| `primary-900` | `theme.colors.primary.900` | `#0c4a6e` | Deep gradients, focus rings |

### Functional Palette
| Token | Tailwind Class | Hex | Usage |
|-------|----------------|-----|-------|
| `success-500` | `bg-success-500`, `text-success-600` | `#22c55e` | Positive metrics, success badges |
| `warning-500` | `bg-warning-500`, `text-warning-600` | `#eab308` | Warnings, thresholds |
| `danger-500`  | `bg-danger-500`, `text-danger-600`  | `#ef4444` | Errors, destructive actions |
| `gray-50` → `gray-900` | Native Tailwind grays | — | Surfaces, text, borders |

### Gradient & Background Tokens
- Use Tailwind gradients (`from-primary-50 to-primary-100`, etc.) for subtle cards. 
- Avoid high-contrast neon gradients from TPS Co-Pilot; prioritize legibility on data tables.
- Body background: `bg-gray-50` by default. For dark sections, use `bg-slate-900` with white text and adjust charts accordingly.

---

## 📐 Layout & Rhythm

- **Base spacing unit:** 4px (aligns with Tailwind scale: `space-x-1`, `p-4`, etc.).
- **Breakpoints:** adhere to Tailwind defaults (`sm`, `md`, `lg`, `xl`, `2xl`). Do not introduce custom breakpoints unless documented here.
- **Content max width:** 1280px (`max-w-7xl`) centered with responsive padding (`px-4 sm:px-6 lg:px-8`).
- **Cards:** use `.card` utility defined in `globals.css` with consistent padding (`p-6` desktop, `p-4` mobile).
- **Grid patterns:** prefer `grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4` for KPI blocks and `grid grid-cols-1 lg:grid-cols-2` for charts.

---

## 🔠 Typography & Iconography

### Fonts
| Purpose | Family | Tailwind Class |
|---------|--------|----------------|
| Display / Headings | Inter (default) | `font-semibold`, `text-2xl`+ |
| Body | Inter | `text-sm`, `text-base` |
| Metrics | Inter or tabular numbers via `font-mono` when precision critical |

> Do not import additional web fonts without design review. Consistency outweighs ornamental styling.

### Type Scale (Desktop)
| Token | Tailwind Example | Usage |
|-------|------------------|-------|
| H1 | `text-3xl md:text-4xl font-semibold` | Page title |
| H2 | `text-2xl font-semibold` | Section headers |
| H3 | `text-lg font-semibold` | Card titles |
| Body | `text-sm text-gray-600` | Descriptive text |
| Data | `text-2xl font-bold text-gray-900` | KPI highlight |

### Icons
- Use `lucide-react` for all vector icons (outlined aesthetic). Size 16–24px.
- Align icons and text via `flex items-center gap-2`.
- Status icons: `text-success-600`, `text-warning-600`, `text-danger-600`.
- Avoid mixing icon libraries.

---

## 🧩 Core Components

### App Shell
- Layout: sticky top navigation (64px height), main content with `min-h-screen bg-gray-50`.
- Sidebar (if introduced): 72px min width, `bg-white`, `border-r border-gray-200`.
- Navigation actions: use `btn btn-outline` utilities or Tailwind combos (`px-4 py-2 rounded-lg bg-white shadow-sm`).

### Cards
- Base class: `.card` (white background, rounded-xl, subtle border, shadow-md, generous padding).
- Hover elevate: `hover:shadow-lg transition-shadow duration-200`.
- Internal structure: title row, content, optional footer (divider via `border-t pt-3`).

### Buttons
| Pattern | Classes | Usage |
|---------|---------|-------|
| Primary | `btn btn-primary` | Critical actions |
| Secondary | `btn btn-secondary` | Secondary emphasis |
| Outline | `btn btn-outline` | Neutral navigation |
| Destructive | `btn btn-danger` | Deletions, risk |
- All button variants inherit a soft elevation (`shadow-sm`) and consistent `focus:ring` / `focus:ring-offset` tokens for accessibility.

### Input Controls
- Use `.label` and `.input` utilities from `globals.css`.
- Input groups stack with `space-y-4` or `grid md:grid-cols-2 gap-4`.
- Inline validation: `text-danger-600 text-sm mt-1`.

### Badges & Status Chips
- Badge base: `.badge` utility.
- Choose from `.badge-success`, `.badge-warning`, `.badge-danger`, `.badge-info`.
- For custom statuses, extend via Tailwind plugin—document additions here.

---

## 📊 Data Visualization Standards

- Library: `recharts`. All charts wrap in `ResponsiveContainer` with `height` 240–320px.
- Grid: enable `CartesianGrid` with `strokeDasharray="3 3"` and `stroke="#e5e7eb"`.
- Axis text: `fontSize={12}`, `stroke="#6b7280"`.
- Tooltips: white background, `borderRadius: '8px'`, subtle border `#e5e7eb`.
- Colors: revenue (`#0ea5e9`), impressions (`#22c55e`), eCPM (`#f59e0b`), fill rate (`#8b5cf6`). Do not introduce new palette without approval.
- Multi-series charts: annotate with `Legend` and ensure color contrast ratio ≥ 4.5:1.

---

## ⚙️ Interaction & Motion

- Use Tailwind transitions: `transition-all duration-200` for hover states.
- Loading states: skeletons using `animate-pulse` with neutral gray blocks.
- Async feedback: for actions >400ms, show `Spinner` component (pending) or disable buttons with `opacity-60 cursor-not-allowed`.
- Do not use unbounded spinners; provide context (text label or inline hint).

---

## 🧱 Layout Primitives

### Flex & Grid
- Primary layout: `flex flex-col min-h-screen` in `src/app/layout.tsx`.
- Use utility classes like `flex items-center justify-between gap-4` for header toolbars.
- For forms, prefer `grid md:grid-cols-2 gap-6` to keep alignment consistent.

### Spacing Rules
- External page padding: `py-8` desktop (`py-6` mobile), `px-4 sm:px-6 lg:px-8`.
- Card gap: `gap-6` in grids, `space-y-4` vertically.
- Section separation: `mt-10` for large transitions.

---

## 🔒 Accessibility

- Minimum color contrast: 4.5:1 for body text, 3:1 for large text and UI elements.
- Focus states: ensure `focus:ring` utilities present on interactive elements.
- Use semantic headings (H1 per page). Do not skip heading levels.
- Provide `aria-labels` for icon-only buttons; avoid relying on tooltip-only context.

---

## 🧭 Navigation Patterns

- Primary navigation resides in upcoming sidebar; interim top-level navigation uses breadcrumb + quick actions area.
- Keep navigation density low: no more than 5 primary tabs. Additional modules should live under `/settings`, `/fraud`, `/placements`, etc.
- All nav items: `flex items-center gap-2 px-3 py-2 rounded-md text-sm font-medium transition-all` with hover state `bg-primary-50 text-primary-700`.

---

## 📝 Copy & Content Guidelines

- KPIs: concise labels (`Total Revenue`, `Impressions`). Subtext via `text-xs text-gray-500`.
- Empty states: Icon (48px) + headline + supportive text + primary/secondary CTA.
- Error states: Red icon + message, optionally `Try again` button.
- Tooltips: prefer inline helper text over hidden tooltips for critical information.

---

## 🎯 Transparency UI Patterns (Updated 2025-11-10)

The Transparency system implements specialized UX patterns for cryptographic verification workflows.

### VerifyBadge Component
**Location:** `console/src/components/ui/VerifyBadge.tsx`

Lazy-loading verification status badge with tooltips:
- **PASS** (green): Signature verified successfully
- **FAIL** (red): Signature verification failed
- **NOT_APPLICABLE** (gray): Auction not signed or not sampled
- **UNKNOWN_KEY** (orange): Signing key not found in registry
- **Loading**: Shows spinner during verification

Usage:
```tsx
<VerifyBadge 
  auctionId="auc-123" 
  hasSigned={true}
  autoLoad={true}  // Auto-verify on mount
  compact={true}   // Compact mode for table cells
/>
```

### Copy Affordances
**Location:** `console/src/components/ui/CopyButton.tsx`

Enhanced copy button with visual feedback:
- Checkmark icon appears on successful copy (2s duration)
- Three variants: `default` (button), `icon` (icon-only), `inline` (text link)
- Tooltips show copy status
- Full keyboard accessibility

Usage:
```tsx
<CopyButton text="auc-123" variant="icon" size="sm" />
<CopyButton text={signature} label="Copy Signature" />
```

### Skeleton Loaders
**Location:** `console/src/components/ui/Spinner.tsx`

Content placeholders that match final structure:
- Use gray-200 background with pulse animation
- Match dimensions of actual content
- Provide variants: `text`, `circular`, `rectangular`

Usage:
```tsx
<Skeleton variant="text" width="w-32" height="h-4" />
<Skeleton variant="circular" width="w-10" height="h-10" />
```

### Debounced Filters
**Location:** `console/src/lib/hooks.ts`

Use `useDebouncedValue` hook for search/filter inputs:
- Default 300ms delay reduces API calls
- Improves perceived performance
- Prevents excessive re-renders

Usage:
```tsx
const [search, setSearch] = useState('')
const debouncedSearch = useDebouncedValue(search, 300)

useEffect(() => {
  // API call with debouncedSearch
}, [debouncedSearch])
```

### Query String Persistence
**Location:** `console/src/lib/hooks.ts`

Use `useQueryParams` hook for shareable filtered views:
- Syncs filter state with URL query parameters
- Enables bookmarking and sharing
- Maintains pagination state across navigation

Usage:
```tsx
const { params, updateParams } = useQueryParams()

// Read from URL
const page = params.get('page') || '1'

// Update URL
updateParams({ page: '2', search: 'test' })
```

### Tooltip Guidelines
**Location:** `console/src/components/ui/Tooltip.tsx`

Use tooltips for contextual help on verification badges and technical fields:
- Position automatically (top/bottom/left/right)
- 200ms delay before showing (configurable)
- Keyboard accessible (focus/blur events)
- Use gray-900 background with white text

Usage:
```tsx
<Tooltip content="This auction was verified successfully">
  <span className="badge">PASS</span>
</Tooltip>
```

---

## 🧪 Implementation Checklist

- [ ] Page uses `RootLayout` defaults and wraps content in standard container.
- [ ] Typography matches type scale (no ad-hoc font sizes without utility classes).
- [ ] Colors limited to documented palette.
- [ ] Buttons and inputs use shared utilities (`btn`, `.input`).
- [ ] Cards leverage `.card` structure; no raw `div` with custom `box-shadow`.
- [ ] Charts use `recharts` components with documented colors.
- [ ] Loading/skeleton states present for async data.
- [ ] Responsive behavior validated at `sm`, `md`, `lg` breakpoints.
- [ ] Accessibility considerations (contrast, focus states) verified.
- [ ] No inline styles unless absolutely required (prefer Tailwind utilities).

---

## 🔄 Change Management

1. Proposals to adjust palette, typography, or core components require design review.
2. Update this document with change rationale, version bump, and date.
3. Communicate updates in project README or release notes.
4. Document new/updated components in the Storybook library for visual QA (`npm run storybook`).
5. Re-run `npm run design:verify` whenever `tailwind.config.ts` changes; commit the updated hash to this doc.

---

## 📕 Storybook Component Library

- **Location:** `.storybook/` configuration + stories colocated with components (e.g., `src/components/ui/*.stories.tsx`).
- **Run locally:** `npm run storybook` (dev server on http://localhost:6006). Build static docs via `npm run storybook:build`.
- **Scope:** Visual + a11y regression coverage for navigation primitives, billing widgets, forms, and tokens. Add at least one story per reusable component (CopyButton, StatusBadge, Tooltip, etc.).
- **Tailwind tokens:** Storybook imports `src/app/globals.css` so every change to `tailwind.config.ts` immediately surfaces in the component library; the `design:verify` script enforces that this document reflects the latest token hash.

---

## 📚 References & Resources

- Tailwind CSS Docs: https://tailwindcss.com/docs
- Lucide Icons: https://lucide.dev/
- Recharts Docs: https://recharts.org/
- Accessibility Contrast Checker: https://webaim.org/resources/contrastchecker/
- TPS Co-Pilot Design System (inspiration): `/Design.md`

> **Reminder:** Consistency first. Every new view should feel like a natural extension of the dashboard created in Task 14. If a design choice conflicts with this document or Tailwind config, pause development and consult the design owner.
