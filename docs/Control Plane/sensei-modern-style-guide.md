# Sensei OS Design System: Sensei-Rams (Version 3.0)

> **The authoritative technical specification for Sensei OS frontend architecture.**
> 
> **Version 3.0.1** - Updated January 2026
> **Philosophy**: "Less, but better" (*Weniger, aber besser*) — Dieter Rams
> **Paradigm**: High-Precision Industrial Functionalism

---

## Preamble: Why Not "Modern SaaS"

Sensei OS 3.0 **explicitly rejects** the visual language that has become ubiquitous across enterprise software:

| ❌ The "2024 SaaS Look" | Why We Reject It |
|------------------------|------------------|
| Rounded cards with soft shadows | Conveys "cloud" impermanence; users don't trust floating data |
| Glassmorphism / frosted blur | Computationally expensive; reduces legibility; purely decorative |
| Gradient backgrounds | Fashion-dependent; will date rapidly |
| Oversized padding and whitespace | Wastes screen real estate that manufacturing users need for data |
| "Friendly" illustrations | Patronizing to professional operators; reduces information density |
| Animated micro-interactions everywhere | Distracting; slows perceived performance; accessibility hazard |
| Sidebar-centric navigation | Creates wasted gutter space; limits content area |

**Our users are not browsing—they are operating.** A manufacturing ERP is closer to an aircraft cockpit than a social media feed. The interface must convey:
- **Permanence**: Data persists; decisions matter
- **Precision**: Every pixel serves a purpose
- **Professionalism**: Respect for the operator's expertise
- **Performance**: Instant response; zero animation tax

---

## 1. Design Philosophy: The Braun Legacy in Digital Form

### 1.1 Historical Context

Dieter Rams served as head of design at Braun from 1961 to 1995, creating products that remain functional and aesthetically relevant 60 years later. His work was characterized by:

- **Warm neutral tones**: Not clinical white, but purposeful off-white and warm greys
- **Honest materials**: Plastic looked like plastic, metal like metal—no simulation
- **Functional color**: Orange for controls, red for warnings—never decorative
- **Grid precision**: Every element aligned to invisible structure
- **Removal of the unnecessary**: If a feature didn't improve function, it was eliminated

### 1.2 The 10 Principles Applied to Sensei OS

| # | Rams' Principle | Sensei OS Implementation | Verification |
|---|----------------|-------------------------|--------------|
| 1 | **Good design is innovative** | Novel interaction patterns for manufacturing workflows; not innovation for novelty | Does this feature solve a real problem uniquely? |
| 2 | **Good design makes a product useful** | Every UI element enables a task; decorative elements are prohibited | Can this element be removed without loss of function? |
| 3 | **Good design is aesthetic** | Beauty from proportion, alignment, and negative space—not ornamentation | Does it look good with all content removed? |
| 4 | **Good design makes a product understandable** | Self-documenting interfaces; icon + label always; status always visible | Can a new user understand this without training? |
| 5 | **Good design is unobtrusive** | The UI recedes; content dominates; no chrome competition | Is the user looking at their data or our design? |
| 6 | **Good design is honest** | No fake depth, no simulated materials, no misleading affordances | Does every visual cue accurately represent function? |
| 7 | **Good design is long-lasting** | Neutral palette; no trendy effects; timeless typography | Will this look dated in 5 years? |
| 8 | **Good design is thorough down to the last detail** | 4px grid absolute; consistent spacing; precise kerning | Zoom to 400%—does alignment hold? |
| 9 | **Good design is environmentally friendly** | Optimized assets; reduced motion; dark mode efficiency | Lighthouse performance score >90? |
| 10 | **Good design is as little design as possible** | Maximum 5 elements per information card; ruthless simplification | What can be removed? |

### 1.3 The "Control Station" Metaphor

Sensei OS is not a "web application"—it is a **digital control station** for manufacturing operations. This metaphor informs every design decision:

| Concept | Web Application Metaphor | Control Station Metaphor |
|---------|-------------------------|-------------------------|
| **Layout** | Pages, cards, sidebars | Instrument panels, modules, racks |
| **Navigation** | Menu bars, breadcrumbs | Process pipes, station indicators |
| **Data Display** | Lists, tables | Gauges, readouts, indicators |
| **Actions** | Buttons, links | Switches, controls, levers |
| **Status** | Badges, toasts | Andon lights, status strips |
| **Feedback** | Animations, transitions | State changes, indicator lights |

---

## 2. Typography: The Precision of Scientific Instruments

### 2.1 Font Stack Specification

| Layer | Primary Font | Fallback | Weight Range | Usage |
|-------|-------------|----------|--------------|-------|
| **UI Text** | Geist Sans | Inter, system-ui | 400-600 | Navigation, labels, descriptions |
| **Data/Metrics** | JetBrains Mono | SF Mono, Consolas | 400-700 | Numbers, codes, identifiers |
| **System/Labels** | Geist Mono | SF Mono, monospace | 500-800 | Dymo labels, status text, metadata |
| **Headings** | Geist Sans | Inter | 600-800 | Section titles, page headers |

### 2.2 Typography Scale (Based on 4px Grid)

```
--text-3xs: 8px    / 12px line-height  — Station metadata, timestamps
--text-2xs: 10px   / 14px line-height  — Dymo labels, system status
--text-xs:  12px   / 16px line-height  — Secondary labels, hints
--text-sm:  14px   / 20px line-height  — Body text, descriptions
--text-base: 16px  / 24px line-height  — Primary content
--text-lg:  18px   / 28px line-height  — Subheadings
--text-xl:  20px   / 28px line-height  — Section headers
--text-2xl: 24px   / 32px line-height  — Page titles
--text-3xl: 32px   / 40px line-height  — Dashboard metrics
--text-4xl: 40px   / 48px line-height  — Hero numbers (OEE, Takt)
```

### 2.3 Anti-Blur Rendering Protocol

Every text element must pass these rendering standards:

```css
/* Required on all text-rendering contexts */
body {
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  font-feature-settings: "kern" 1, "liga" 1;
}

/* Numeric data must use tabular figures */
.metric, .data-value, [data-numeric] {
  font-variant-numeric: tabular-nums lining-nums;
  font-feature-settings: "tnum" 1, "lnum" 1;
}

/* Labels use tight tracking */
.label, .dymo-label {
  letter-spacing: 0.05em;
  text-transform: uppercase;
  font-weight: 700;
}
```

### 2.4 Typography Don'ts

- ❌ Never use `text-shadow` for legibility—use contrast
- ❌ Never use font-weight below 400 for UI text
- ❌ Never use fractional line-heights (1.5, 1.6)—use pixel values
- ❌ Never use decorative fonts (scripts, display faces)
- ❌ Never use centered text for data-heavy content

---

## 3. The Sensei-Rams Color System

### 3.1 Core Palette: The Braun Warm-Grey Scale

The palette is derived from analysis of Braun products from 1960-1990. We reject the blue-tinted "cool greys" ubiquitous in tech software.

| Token | Light Mode | Dark Mode | Usage |
|-------|-----------|-----------|-------|
| `--rams-chassis` | `#F2F2F2` | `#1A1A1A` | Primary background; the "case" |
| `--rams-module` | `#E6E6E6` | `#252525` | Card/panel background |
| `--rams-panel` | `#D9D9D9` | `#2D2D2D` | Inset areas, secondary panels |
| `--rams-line` | `#CCCCCC` | `#404040` | Borders, dividers, structure |
| `--rams-muted` | `#999999` | `#666666` | Muted text, inactive elements |
| `--rams-foreground` | `#1A1A1A` | `#F2F2F2` | Primary text |

**Critical**: Never use pure white (`#FFFFFF`) or pure black (`#000000`) for backgrounds. These create harsh contrast that causes eye strain over 8-hour shifts.

### 3.2 Functional Accent Colors

Colors are **semantic**, not decorative. Each color has exactly one meaning:

| Color | Token | Hex | Meaning | Never Use For |
|-------|-------|-----|---------|---------------|
| **Braun Orange** | `--rams-orange` | `#FFBE00` | Primary action, active state, current position | Warnings, errors |
| **Functional Green** | `--rams-green` | `#2D8C3C` | Success, operational, healthy, complete | General buttons |
| **Industrial Red** | `--rams-red` | `#D62D2D` | Error, critical, stop, danger | Decoration, branding |
| **Steel Blue** | `--rams-steel` | `#4A90E2` | Information, links | Primary actions |

### 3.3 Color Accessibility Requirements

| Combination | Minimum Ratio | Target Ratio | WCAG Level |
|-------------|---------------|--------------|------------|
| `--rams-foreground` on `--rams-chassis` | 4.5:1 | 15:1+ | AAA |
| `--rams-foreground` on `--rams-module` | 4.5:1 | 12:1+ | AAA |
| `--rams-orange` on `--rams-chassis` | 3:1 | 4.5:1+ | AA (large text) |
| Interactive element contrast | 3:1 | 4.5:1 | AA |

### 3.4 Color Don'ts

- ❌ Never use color as the only indicator of state (always pair with icon/text)
- ❌ Never use gradients in the core UI
- ❌ Never use opacity below 60% for meaningful text
- ❌ Never use brand colors from external companies
- ❌ Never use more than 3 colors on a single screen region

---

## 4. Spatial System: The 4px Grid of Necessity

### 4.1 Base Unit

Every measurement in Sensei OS derives from a **4px base unit**. This creates visual harmony and ensures pixel-perfect rendering.

```
1 unit  = 4px   — Minimum spacing, borders
2 units = 8px   — Compact spacing, icon gaps  
3 units = 12px  — Standard spacing
4 units = 16px  — Section spacing
6 units = 24px  — Component padding
8 units = 32px  — Card padding, major spacing
12 units = 48px — Section separation
16 units = 64px — Page-level spacing
```

### 4.2 The "Industrial Bezel" Frame

The viewport is wrapped in a structural frame that reinforces the "control station" metaphor:

```
┌─────────────────────────────────────────────────────────────┐
│ ○                                                         ○ │ ← 8px bezel with corner screws
│   ┌─────────────────────────────────────────────────────┐   │
│   │                                                     │   │
│   │                   CONTENT AREA                      │   │
│   │                                                     │   │
│   │                                                     │   │
│   └─────────────────────────────────────────────────────┘   │
│ STATION: SENSEI-01  │  OS: 3.0.0  │  STATUS: OPERATIONAL    │ ← 32px status bar
└─────────────────────────────────────────────────────────────┘
```

### 4.3 Spacing Don'ts

- ❌ Never use fractional pixels (10.5px)—round to nearest 4px
- ❌ Never use inconsistent spacing within the same component
- ❌ Never use margin where padding is appropriate (or vice versa)
- ❌ Never exceed 32px padding on interactive elements
- ❌ Never use negative margins to fix alignment issues

---

## 5. Component Philosophy: Modules, Not Cards

### 5.1 The "Racked Module" Pattern

Components in Sensei OS are not floating cards—they are **modules slotted into a rack**:

| SaaS Card Pattern | Sensei-Rams Module Pattern |
|-------------------|---------------------------|
| Rounded corners (12-24px) | Sharp or micro-rounded (2-4px) |
| Drop shadow for elevation | 1px solid border for definition |
| Generous padding (24-32px) | Efficient padding (12-16px) |
| Isolated with large gaps | Edge-to-edge or minimal gaps (4px) |
| White background | Warm grey (`--rams-module`) |
| Hover: shadow increase | Hover: border highlight, no shadow |

### 5.2 The 5-Element Maximum Rule

Every information module must contain **no more than 5 primary elements**. This is based on cognitive load research (Cowan's 4±1 working memory limit).

```
┌─────────────────────────────────────┐
│ [1] LABEL                           │  ← Element 1: Identifier
│ [2] 847                             │  ← Element 2: Primary metric
│ [3] units/hour                      │  ← Element 3: Unit/context
│ [4] ▲ 12% vs target                 │  ← Element 4: Comparison
│ [5] ████████░░ 84%                  │  ← Element 5: Progress/gauge
└─────────────────────────────────────┘
```

If more information is needed, **nest modules** rather than overcrowd.

### 5.3 Border Philosophy: Mass, Not Shadow

Rams' Braun products conveyed depth through **material mass and structural lines**, not illumination effects:

```css
/* Correct: Structural border */
.module {
  border: 1px solid var(--rams-line);
  background: var(--rams-module);
}

/* Correct: Inset effect via border */
.inset-panel {
  border: 1px solid var(--rams-line);
  box-shadow: inset 1px 1px 0 rgba(255,255,255,0.5),
              inset -1px -1px 0 rgba(0,0,0,0.05);
}

/* WRONG: Drop shadow */
.card {
  box-shadow: 0 4px 6px rgba(0,0,0,0.1); /* ❌ PROHIBITED */
}
```

---

## 6. Interactive Elements: Mechanical Precision

### 6.1 Buttons: Controls, Not Clickables

Buttons in Sensei OS are modeled after physical control panel switches:

| State | Visual Treatment |
|-------|-----------------|
| Default | Solid fill, 1px border, neutral background |
| Hover | Border color intensifies, subtle background shift |
| Active/Pressed | `scale(0.98)`, inset shadow simulation |
| Focus | 2px outline with `--rams-orange`, 2px offset |
| Disabled | 50% opacity, cursor: not-allowed |

```css
.rams-button {
  /* Base */
  padding: 8px 16px;
  border: 1px solid var(--rams-line);
  border-radius: 2px;
  background: var(--rams-module);
  font-weight: 500;
  
  /* Mechanical feel */
  transition: transform 50ms, box-shadow 50ms;
}

.rams-button:active {
  transform: scale(0.98);
  box-shadow: inset 0 2px 4px rgba(0,0,0,0.1);
}
```

### 6.2 Toggle Switches: Rotary Knob Metaphor

Avoid the pill-shaped iOS toggle. Use a mechanical switch metaphor:

```
OFF Position:        ON Position:
┌─────────────┐      ┌─────────────┐
│ ●───────────│      │───────────● │
└─────────────┘      └─────────────┘
  ↑ Circular knob on track
```

### 6.3 Inputs: Instrument Readouts

Text inputs resemble instrument display windows:

```css
.rams-input {
  border: 1px solid var(--rams-line);
  border-radius: 2px;
  background: var(--rams-panel);
  padding: 8px 12px;
  font-family: 'JetBrains Mono', monospace;
  
  /* Inset "display window" effect */
  box-shadow: inset 1px 1px 2px rgba(0,0,0,0.05);
}

.rams-input:focus {
  outline: none;
  border-color: var(--rams-orange);
  box-shadow: inset 1px 1px 2px rgba(0,0,0,0.05),
              0 0 0 2px rgba(255,190,0,0.2);
}
```

---

## 7. Navigation: Process Flow, Not Menu Trees

### 7.1 The "Process Pipe" Pattern

Instead of hierarchical menus, Sensei OS uses **linear process visualization**:

```
┌──────┐    ┌──────┐    ┌──────┐    ┌──────┐
│ RFQ  │───▶│Quote │───▶│Order │───▶│ Ship │
└──────┘    └──────┘    └──────┘    └──────┘
   ●           ○           ○           ○
   ↑
   Current position indicator (Braun Orange)
```

### 7.2 Sidebar: The "Rack Chassis"

The sidebar is not a menu list—it is a **vertical rack of station indicators**:

```
┌────────────────────────┐
│ ▣ SENSEI OS            │  ← Station identifier
├────────────────────────┤
│ ┌────────────────────┐ │
│ │ ◉ Dashboard        │ │  ← Active module (orange indicator)
│ └────────────────────┘ │
│ ┌────────────────────┐ │
│ │ ○ Manufacturing    │ │  ← Inactive module
│ └────────────────────┘ │
│ ┌────────────────────┐ │
│ │ ○ Inventory        │ │
│ └────────────────────┘ │
├────────────────────────┤
│ ● SYSTEM OPTIMAL       │  ← Status indicator (green)
└────────────────────────┘
```

### 7.3 Breadcrumbs: Position Indicators

Breadcrumbs indicate position in the station hierarchy:

```
MANUFACTURING ▸ WORK ORDERS ▸ WO-2024-0847
     ↑               ↑              ↑
   Zone          Section       Current Station
```

---

## 8. Data Visualization: Gauges Over Charts

### 8.1 The Analog Gauge Preference

Where possible, use **analog-style gauges** rather than modern charts:

| Data Type | Preferred Visualization |
|-----------|------------------------|
| Single value in range | Semicircular gauge with needle |
| Percentage/completion | Linear progress bar with tick marks |
| Status | Andon light stack |
| Trend | Sparkline (minimal, no axis decoration) |
| Comparison | Bar gauge with target marker |

### 8.2 Andon Light Status System

The three-light stack from Toyota Production System:

```css
.andon-stack {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 4px;
  background: var(--rams-panel);
  border: 1px solid var(--rams-line);
}

.andon-light {
  width: 12px;
  height: 12px;
  border-radius: 50%;
  background: var(--rams-muted);
  opacity: 0.3;
}

.andon-light.active.red { background: var(--rams-red); opacity: 1; }
.andon-light.active.yellow { background: var(--rams-orange); opacity: 1; }
.andon-light.active.green { background: var(--rams-green); opacity: 1; }
```

### 8.3 Chart Styling Requirements

When charts are necessary:

- ❌ No 3D effects
- ❌ No gradient fills
- ❌ No decorative grid lines
- ✅ 1px solid lines only
- ✅ Monospace labels
- ✅ Muted axis colors
- ✅ Data points in functional colors only

---

## 9. Motion & Animation: Mechanical Restraint

### 9.1 The 200ms Maximum

No animation in Sensei OS exceeds 200ms. This ensures:
- Perceived instantaneous response
- No motion-sickness risk
- No productivity loss waiting for animations

### 9.2 Animation Principles

| Animation Type | Duration | Easing | Use Case |
|---------------|----------|--------|----------|
| State change | 50ms | linear | Button press, toggle |
| Micro-feedback | 100ms | ease-out | Hover states |
| Content transition | 150ms | ease-out | Panel switching |
| Modal appearance | 200ms | ease-out | Dialog open |
| **Page transitions** | 0ms | none | Instant navigation |

### 9.3 Reduced Motion Support

```css
@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
  }
}
```

---

## 10. Anti-Patterns Catalog

### 10.1 Visual Anti-Patterns

| Pattern | Why It's Wrong | Correct Alternative |
|---------|----------------|---------------------|
| `border-radius: 9999px` (pill) | Fashion-dependent; looks dated | `border-radius: 2px` |
| `backdrop-filter: blur()` | Performance tax; reduces legibility | Solid backgrounds |
| `box-shadow: 0 4px 6px` | Creates "floating" instability | `border: 1px solid` |
| Gradient backgrounds | Decorative; no functional purpose | Solid `--rams-*` colors |
| Animated loading spinners | Distracting; implies slowness | Static progress indicators |
| "Skeleton" loading states | Visually noisy; uncertain duration | Minimal loading indicator |
| Card hover lift effect | Implies impermanence | Border color change only |
| Icon-only buttons | Recall burden; accessibility failure | Icon + label always |

### 10.2 Structural Anti-Patterns

| Pattern | Why It's Wrong | Correct Alternative |
|---------|----------------|---------------------|
| Modal for simple confirmations | Interrupts flow | Inline confirmation |
| Toast notifications for errors | Too easily dismissed | Persistent error state |
| Infinite scroll | Disorienting; no position sense | Pagination with position indicator |
| Hamburger menu | Hides navigation; extra click | Always-visible rack sidebar |
| Tabs with > 5 items | Cognitive overload | Nested navigation |
| Multi-step wizards > 4 steps | User loses context | Single-page progressive disclosure |

### 10.3 Behavioral Anti-Patterns

| Pattern | Why It's Wrong | Correct Alternative |
|---------|----------------|---------------------|
| Auto-save without indicator | User uncertainty | Explicit save state display |
| Destructive action without undo | Irreversible anxiety | 10-second undo window |
| Form validation on blur | Premature interruption | Validation on submit |
| Disabled submit until valid | User confusion | Submit + show errors |
| Session timeout without warning | Data loss risk | 5-minute countdown warning |

---

## 11. Responsive Behavior: Density, Not Simplification

### 11.1 Breakpoint Strategy

| Breakpoint | Name | Strategy |
|------------|------|----------|
| < 640px | `mobile` | Compact mode mandatory; simplified layouts |
| 640-1024px | `tablet` | Standard density; stacked modules |
| 1024-1440px | `desktop` | Full density; side-by-side modules |
| > 1440px | `station` | Maximum density; command center layout |

### 11.2 Density Modes

Users can select their preferred density:

| Mode | Padding Scale | Font Scale | Target Users |
|------|--------------|------------|--------------|
| **Compact** | 0.75x | 0.9x | Power users, dense data needs |
| **Comfortable** | 1.0x | 1.0x | Standard operation |
| **Expanded** | 1.25x | 1.1x | Accessibility, touch interfaces |

### 11.3 Mobile Adaptation

On mobile, the interface becomes a **portable instrument panel**, not a "mobile website":

- Navigation collapses to bottom tab bar (5 items max)
- Cards become full-width modules
- Actions move to bottom sheet (thumb-reachable)
- Data density reduces but information hierarchy preserves

---

## 12. Iconography: Industrial Pictograms

### 12.1 Icon Style

Icons in Sensei OS are **industrial pictograms**, not friendly illustrations:

| Characteristic | Specification |
|---------------|---------------|
| Style | Outline, 1.5px stroke |
| Grid | 24x24 with 2px padding |
| Corners | 90° angles preferred; 2px radius max |
| Fills | Solid only when active/selected |
| Animation | None |

### 12.2 Icon Requirements

- Every icon MUST have a text label (except in toolbars with tooltips)
- Icons MUST maintain meaning at 16px size
- Icons MUST be distinguishable at 10% opacity (for inactive states)
- Custom icons MUST follow the Lucide icon grid

---

## 13. Implementation Checklist

Before any UI component ships, verify:

### Typography
- [ ] Font renders at exact pixel sizes (no fractional)
- [ ] `antialiased` applied
- [ ] `optimizeLegibility` set
- [ ] Tabular figures for numeric data
- [ ] Weight ≥ 400 for all UI text

### Color
- [ ] Only `--rams-*` tokens used
- [ ] Contrast ratio meets targets
- [ ] Color not sole indicator of state
- [ ] Dark mode tested

### Spacing
- [ ] All values divisible by 4px
- [ ] Grid alignment verified at 400% zoom
- [ ] No fractional pixels

### Interaction
- [ ] Focus states visible (2px outline)
- [ ] Keyboard navigation complete
- [ ] Touch targets ≥ 44px
- [ ] Animation ≤ 200ms
- [ ] `prefers-reduced-motion` respected

### Accessibility
- [ ] ARIA labels on interactive elements
- [ ] Screen reader tested
- [ ] Color blindness simulation passed
- [ ] Zoom to 200% functional

---

*This document is the single source of truth for Sensei OS visual language. Deviation requires explicit approval and documentation of rationale.*

**References:**
- Dieter Rams, "Ten Principles for Good Design" (1976)
- Vitsœ, "The Power of Good Design"
- Nielsen Norman Group, "10 Usability Heuristics for User Interface Design"
- W3C, "Web Content Accessibility Guidelines (WCAG) 2.1"
