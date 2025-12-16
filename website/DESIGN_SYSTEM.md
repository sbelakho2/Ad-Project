# Website Design System

**Last updated:** December 16, 2025  
**Status:** LOCKED - Do not modify without design review

## Purpose

This document defines the canonical design standards for the ApexMediation website. These standards are enforced through shared components and must not be bypassed with inline styles or one-off implementations.

---

## Hero Layout System

### Shared Components (REQUIRED)

All marketing and documentation pages **must** use these shared components:

1. **`<SiteHeader />`** - White header with navigation (use on every public page)
2. **`<PageHero variant="dark" | "light">`** - Hero section wrapper

#### Import paths:
```tsx
import SiteHeader from '@/components/SiteHeader';
import PageHero from '@/components/PageHero';
```

### Layout Tokens (DO NOT OVERRIDE)

These tokens are baked into `PageHero` and must be used consistently:

| Token | Value | Usage |
|-------|-------|-------|
| **Wrapper** | `container mx-auto px-4` | Horizontal constraints |
| **Hero padding** | `py-16 md:py-20` | Vertical spacing |
| **Max text width** | `max-w-3xl` | Content width constraint |
| **H1** | `text-4xl md:text-5xl font-bold leading-tight` | Main heading |
| **Subhead** | `text-xl leading-relaxed` | Supporting text |

### Hero Variants

#### Dark Hero (`variant="dark"`)

**Used for:** Marketing pages (/, /product, /features, /pricing, /company)

```tsx
<PageHero variant="dark">
  <div className="text-center">
    <h1 className="text-4xl md:text-5xl font-bold leading-tight text-white mb-6">
      Your Headline
    </h1>
    <p className="text-xl leading-relaxed text-slate-200/90">
      Your subhead text
    </p>
  </div>
</PageHero>
```

**Colors:**
- Section background: `bg-gradient-to-br from-slate-950 via-slate-900 to-slate-800 text-white`
- H1: `text-white`
- Paragraph: `text-slate-200/90`
- Secondary text: `text-slate-300/80`

**Buttons:**
- Primary: `bg-brand-600 hover:bg-brand-700 text-white`
- Secondary: `bg-white/10 hover:bg-white/15 text-white border border-white/20`

#### Light Hero (`variant="light"`)

**Used for:** Trust/docs pages (/developers, /security, /contact)

```tsx
<PageHero variant="light">
  <div className="text-center">
    <h1 className="text-4xl md:text-5xl font-bold leading-tight text-slate-900 mb-6">
      Your Headline
    </h1>
    <p className="text-xl leading-relaxed text-slate-600">
      Your subhead text
    </p>
  </div>
</PageHero>
```

**Colors:**
- Section background: `bg-white text-slate-900`
- H1: `text-slate-900`
- Paragraph: `text-slate-600`

**Optional premium tint** (for Security and similar trust pages):
```tsx
<PageHero variant="light" className="bg-slate-50 border-b border-slate-200/60">
```

---

## Header System

### Site Header (ALWAYS WHITE)

```tsx
<SiteHeader />
```

**DO NOT:**
- ❌ Create custom `<header>` blocks
- ❌ Change header background color based on page type
- ❌ Add inline header navigation

The header **must remain white** (`bg-white border-b border-slate-200/60`) site-wide to prevent disorienting "inverted" navigation experiences.

---

## Navigation Order

**Current order (locked):**

1. Product
2. Features
3. Pricing
4. Company
5. Developers
6. Security

**Location:** `/website/src/components/HomeNav.tsx` (line ~32)

**Change process:** Requires UX review + A/B test validation before reordering.

---

## Anti-Patterns (DO NOT DO)

### ❌ Inline hero sections
```tsx
// WRONG - bypasses design system
<section className="py-16 md:py-20 bg-gradient-to-br from-slate-950...">
  <div className="container mx-auto px-4">
    ...
  </div>
</section>
```

### ❌ One-off typography
```tsx
// WRONG - creates design drift
<h1 className="text-5xl font-extrabold">...</h1>
```

### ❌ Custom header implementations
```tsx
// WRONG - creates inconsistency
<header className="bg-black">
  <HomeNav />
</header>
```

---

## When to Extend (Rare)

If you absolutely need to deviate from these standards:

1. **Document why** in a GitHub issue
2. **Get design review** from team lead
3. **Consider if** the design system should be updated instead of bypassed
4. **Add a comment** explaining the exception in the code

---

## Enforcement

- **Shared components:** `PageHero.tsx`, `SiteHeader.tsx` contain JSDoc warnings
- **This document:** Source of truth for all hero/header patterns
- **Code review:** PRs that bypass these components should be flagged

---

## Migration Checklist

✅ All target pages migrated to `PageHero` + `SiteHeader`:
- ✅ / (home)
- ✅ /product
- ✅ /features
- ✅ /pricing
- ✅ /company
- ✅ /developers
- ✅ /security
- ✅ /contact

**Status:** Design system locked as of December 16, 2025
