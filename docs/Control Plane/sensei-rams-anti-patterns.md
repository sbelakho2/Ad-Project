# Sensei-Rams Anti-Patterns & Component Catalog

> **A visual reference contrasting forbidden "Modern SaaS" patterns with correct Sensei-Rams implementations.**
>
> "Indifference towards people and the reality in which they live is actually the one and only cardinal sin in design." — Dieter Rams

---

## Purpose

This document serves as a quick reference to prevent regression toward generic SaaS aesthetics. Every component choice should be deliberate, functional, and true to the Industrial Functionalist philosophy.

---

## 1. The Philosophy Gap

| Aspect | SaaS Trend (AVOID) | Sensei-Rams (CORRECT) |
|--------|-------------------|----------------------|
| **Mental Model** | "App" — disposable software | "Instrument" — precision tool |
| **Visual Metaphor** | Cloud dashboard | Control station |
| **Color Use** | Gradient for "vibrancy" | Solid for legibility |
| **Typography** | Variable for "personality" | Fixed for precision |
| **Feedback** | "Delightful" animations | Mechanical confirmation |
| **Density** | "Breathing room" | Information efficiency |

---

## 2. Component Anti-Patterns

### 2.1 Buttons

**❌ FORBIDDEN: Pill Button**
```tsx
// NEVER DO THIS
<button className="rounded-full px-6 py-3 bg-gradient-to-r from-purple-500 to-pink-500 text-white shadow-lg hover:shadow-xl transition-all">
  Get Started
</button>
```
- Rounded corners > 8px
- Gradient fills
- Drop shadows
- "Call to action" marketing language
- Excessive padding

**✓ CORRECT: Industrial Button**
```tsx
// DO THIS
<button className="rounded-rams-sm px-4 py-2 bg-rams-module border border-rams-line hover:bg-rams-panel active:scale-[0.98] active:shadow-rams-pressed">
  Execute
</button>
```
- Sharp corners (2-4px radius)
- Solid fills
- Inset shadows only
- Imperative verb labels
- Precise padding

---

### 2.2 Cards

**❌ FORBIDDEN: Floating Card**
```tsx
// NEVER DO THIS
<div className="rounded-2xl bg-white shadow-2xl p-8 hover:scale-105 transition-transform">
  <div className="text-3xl font-bold bg-gradient-to-r from-blue-600 to-cyan-500 bg-clip-text text-transparent">
    Welcome!
  </div>
</div>
```
- Large rounded corners
- Heavy drop shadows
- Scale transform on hover
- Gradient text
- Exclamatory copy

**✓ CORRECT: Module Container**
```tsx
// DO THIS
<div className="rounded-rams-sm bg-rams-module border border-rams-line">
  <div className="px-4 py-3 border-b border-rams-line">
    <h3 className="text-2xs font-mono font-bold uppercase tracking-widest text-rams-muted">
      SYSTEM STATUS
    </h3>
  </div>
  <div className="p-4">
    {/* Content */}
  </div>
</div>
```
- Sharp corners (2-4px)
- Border, no shadow
- Border dividers
- Uppercase mono labels
- Imperative language

---

### 2.3 Navigation

**❌ FORBIDDEN: Sidebar with Avatars**
```tsx
// NEVER DO THIS
<aside className="w-72 bg-slate-900 text-white">
  <div className="p-6 flex items-center gap-4">
    <img src="/avatar.jpg" className="w-12 h-12 rounded-full ring-4 ring-purple-500" />
    <div>
      <div className="font-semibold">John Doe</div>
      <div className="text-sm text-slate-400">Administrator</div>
    </div>
  </div>
  <nav className="mt-6 space-y-2 px-4">
    <a className="flex items-center gap-3 px-4 py-3 rounded-lg bg-gradient-to-r from-purple-500/20 to-transparent text-purple-300">
      <HomeIcon />
      Dashboard
    </a>
  </nav>
</aside>
```
- Profile avatars
- Colored backgrounds
- Gradient active states
- Large rounded items
- Social/consumer feel

**✓ CORRECT: Rack Sidebar**
```tsx
// DO THIS
<aside className="w-60 bg-rams-module border-r border-rams-line">
  <div className="h-14 border-b border-rams-line flex items-center px-4">
    <div className="font-mono font-bold text-sm tracking-tight">SENSEI OS</div>
  </div>
  <nav className="p-2 space-y-1">
    <a className="flex items-center gap-3 px-3 py-2 rounded-rams-sm border border-transparent hover:border-rams-line hover:bg-rams-panel">
      <div className="w-2 h-2 rounded-full bg-rams-muted/30" />
      <span className="text-sm font-medium text-rams-muted">Operations</span>
    </a>
  </nav>
</aside>
```
- System identifier, not personal
- Neutral background
- Border/background active states
- LED indicators
- Professional/industrial feel

---

### 2.4 Status Indicators

**❌ FORBIDDEN: Badge Pills**
```tsx
// NEVER DO THIS
<span className="inline-flex items-center gap-1 rounded-full px-3 py-1 bg-green-100 text-green-700 text-sm font-medium">
  <span className="w-2 h-2 rounded-full bg-green-500 animate-pulse" />
  Active
</span>
```
- Pill shape
- Tinted backgrounds
- Pulsing animations
- Soft colors

**✓ CORRECT: Andon Indicator**
```tsx
// DO THIS
<div className="flex items-center gap-2">
  <div className="w-3 h-3 rounded-full bg-rams-green shadow-[0_0_8px_rgba(45,140,60,0.5)]" />
  <span className="text-2xs font-mono uppercase text-rams-green">OPERATIONAL</span>
</div>
```
- Simple circle
- Glow effect (not pulse)
- Monospace uppercase
- Industrial terminology

---

### 2.5 Data Tables

**❌ FORBIDDEN: Zebra Stripe Table**
```tsx
// NEVER DO THIS
<table className="w-full">
  <thead>
    <tr className="bg-slate-100 border-b border-slate-200">
      <th className="px-6 py-4 text-left text-sm font-semibold text-slate-900">Name</th>
    </tr>
  </thead>
  <tbody>
    <tr className="odd:bg-white even:bg-slate-50 hover:bg-blue-50">
      <td className="px-6 py-4 text-sm text-slate-700">John Doe</td>
    </tr>
  </tbody>
</table>
```
- Zebra striping
- Colored hover
- Large padding
- Rounded corners on cells
- Soft colors

**✓ CORRECT: Industrial Table**
```tsx
// DO THIS
<table className="w-full border border-rams-line">
  <thead>
    <tr className="bg-rams-panel border-b border-rams-line">
      <th className="px-4 py-3 text-left text-2xs font-mono font-bold uppercase tracking-widest text-rams-muted">
        OPERATOR
      </th>
    </tr>
  </thead>
  <tbody>
    <tr className="border-b border-rams-line hover:bg-rams-panel transition-colors duration-rams-fast">
      <td className="px-4 py-3 text-sm font-mono">RODRIGUEZ, M.</td>
    </tr>
  </tbody>
</table>
```
- Uniform background
- Border hover
- Compact padding
- Mono uppercase headers
- Structured data format

---

### 2.6 Forms

**❌ FORBIDDEN: Floating Labels**
```tsx
// NEVER DO THIS
<div className="relative">
  <input 
    className="peer w-full px-4 pt-6 pb-2 rounded-xl border-2 border-slate-200 focus:border-blue-500 focus:ring-4 focus:ring-blue-200"
    placeholder=" "
  />
  <label className="absolute left-4 top-4 text-slate-500 transition-all peer-focus:top-1 peer-focus:text-xs peer-focus:text-blue-500">
    Email Address
  </label>
</div>
```
- Floating/animated labels
- Large rounded corners
- Colored focus rings
- Ring spread effect

**✓ CORRECT: Industrial Input**
```tsx
// DO THIS
<div className="space-y-1">
  <label className="text-2xs font-mono font-bold uppercase tracking-widest text-rams-muted">
    OPERATOR ID
  </label>
  <input 
    className="w-full h-10 px-3 rounded-rams-sm border border-rams-line bg-rams-panel shadow-rams-inset focus:border-rams-orange focus:outline-none"
    placeholder="Enter ID"
  />
</div>
```
- Fixed labels above
- Sharp corners
- Orange focus border
- Inset shadow
- Instrument display feel

---

### 2.7 Modals

**❌ FORBIDDEN: Blur Backdrop Modal**
```tsx
// NEVER DO THIS
<div className="fixed inset-0 bg-black/40 backdrop-blur-sm flex items-center justify-center">
  <div className="bg-white rounded-3xl shadow-2xl p-8 w-full max-w-lg animate-bounce-in">
    <h2 className="text-2xl font-bold text-center">🎉 Success!</h2>
  </div>
</div>
```
- Backdrop blur
- Extreme rounded corners
- Heavy shadows
- Bounce animations
- Emoji
- Centered headings

**✓ CORRECT: Industrial Panel**
```tsx
// DO THIS
<div className="fixed inset-0 bg-black/60 flex items-center justify-center">
  <div className="bg-rams-module border border-rams-line rounded-rams-sm w-full max-w-md">
    <div className="px-4 py-3 border-b border-rams-line flex items-center justify-between">
      <h2 className="text-2xs font-mono font-bold uppercase tracking-widest text-rams-muted">
        OPERATION COMPLETE
      </h2>
      <button className="text-rams-muted hover:text-rams-foreground">×</button>
    </div>
    <div className="p-4">
      {/* Content */}
    </div>
  </div>
</div>
```
- Solid backdrop
- Sharp corners
- Border, no shadow
- No bounce
- No emoji
- Left-aligned mono header

---

### 2.8 Notifications/Toasts

**❌ FORBIDDEN: Sliding Toast**
```tsx
// NEVER DO THIS
<div className="fixed bottom-4 right-4 bg-gradient-to-r from-green-400 to-emerald-500 text-white px-6 py-4 rounded-2xl shadow-2xl animate-slide-up">
  <div className="flex items-center gap-3">
    <span className="text-2xl">✨</span>
    <span>Your changes have been saved!</span>
  </div>
</div>
```
- Gradient background
- Large rounded corners
- Heavy shadow
- Slide animations
- Emoji
- Exclamatory language

**✓ CORRECT: Status Notification**
```tsx
// DO THIS
<div className="fixed bottom-12 right-6 bg-rams-module border border-rams-green rounded-rams-sm px-4 py-3">
  <div className="flex items-center gap-3">
    <div className="w-2 h-2 rounded-full bg-rams-green" />
    <span className="text-sm font-medium">Changes persisted</span>
  </div>
</div>
```
- Solid background
- Sharp corners
- Border accent
- Instant appearance
- LED indicator
- Factual language

---

## 3. Typography Anti-Patterns

### 3.1 Headlines

**❌ FORBIDDEN:**
```css
/* NEVER */
font-size: 4rem;
font-weight: 800;
background: linear-gradient(to right, #667eea, #764ba2);
-webkit-background-clip: text;
-webkit-text-fill-color: transparent;
```

**✓ CORRECT:**
```css
/* DO */
font-size: 1.5rem;
font-weight: 600;
font-family: var(--font-geist-sans);
color: var(--rams-foreground);
letter-spacing: -0.02em;
```

### 3.2 Body Text

**❌ FORBIDDEN:**
```css
/* NEVER */
font-size: 1.125rem;
line-height: 2;
font-weight: 300;
color: #64748b;
```

**✓ CORRECT:**
```css
/* DO */
font-size: 0.875rem;
line-height: 1.5;
font-weight: 400;
color: var(--rams-foreground);
```

### 3.3 Labels

**❌ FORBIDDEN:**
```css
/* NEVER */
font-size: 0.875rem;
font-weight: 500;
color: #6366f1;
```

**✓ CORRECT:**
```css
/* DO */
font-family: var(--font-geist-mono);
font-size: 0.625rem;
font-weight: 700;
text-transform: uppercase;
letter-spacing: 0.1em;
color: var(--rams-muted);
```

---

## 4. Color Usage Anti-Patterns

### 4.1 Backgrounds

| ❌ FORBIDDEN | ✓ CORRECT |
|-------------|-----------|
| `bg-slate-900` | `bg-rams-chassis` |
| `bg-white` | `bg-rams-module` |
| `bg-gradient-to-br` | (no gradients) |
| `bg-blue-50` | `bg-rams-panel` |

### 4.2 Borders

| ❌ FORBIDDEN | ✓ CORRECT |
|-------------|-----------|
| `border-slate-200` | `border-rams-line` |
| `border-blue-500` | `border-rams-orange` |
| `ring-4 ring-purple-200` | `ring-2 ring-rams-orange ring-offset-2` |

### 4.3 Text

| ❌ FORBIDDEN | ✓ CORRECT |
|-------------|-----------|
| `text-slate-500` | `text-rams-muted` |
| `text-blue-600` | `text-rams-foreground` |
| `text-transparent bg-gradient-to-r` | `text-rams-foreground` |

---

## 5. Animation Anti-Patterns

### 5.1 Transitions

**❌ FORBIDDEN:**
```css
/* NEVER */
transition: all 0.5s cubic-bezier(0.68, -0.55, 0.265, 1.55);
```

**✓ CORRECT:**
```css
/* DO */
transition: background-color 100ms ease-out, border-color 100ms ease-out;
```

### 5.2 Hover Effects

| ❌ FORBIDDEN | ✓ CORRECT |
|-------------|-----------|
| `hover:scale-105` | `active:scale-[0.98]` |
| `hover:shadow-xl` | `hover:border-rams-muted` |
| `hover:-translate-y-1` | (no Y translation) |
| `hover:rotate-3` | (no rotation) |

### 5.3 Loading States

**❌ FORBIDDEN:**
```css
/* NEVER */
@keyframes bounce {
  0%, 100% { transform: translateY(-25%); }
  50% { transform: translateY(0); }
}
animation: bounce 1s infinite;
```

**✓ CORRECT:**
```css
/* DO */
@keyframes pulse-opacity {
  0%, 100% { opacity: 0.3; }
  50% { opacity: 1; }
}
animation: pulse-opacity 1.5s ease-in-out infinite;
```

Or better — use an LED indicator that glows without animation.

---

## 6. Language Anti-Patterns

### 6.1 Button Labels

| ❌ FORBIDDEN | ✓ CORRECT |
|-------------|-----------|
| "Get Started" | "Initialize" |
| "Let's Go!" | "Execute" |
| "Learn More" | "Documentation" |
| "Sign Up Free" | "Create Account" |
| "Save Changes" | "Persist" |
| "Upgrade Now" | "Modify Plan" |

### 6.2 Status Messages

| ❌ FORBIDDEN | ✓ CORRECT |
|-------------|-----------|
| "Awesome! Changes saved! 🎉" | "Changes persisted" |
| "Oops! Something went wrong" | "Operation failed: [error]" |
| "Please wait while we process..." | "Processing" |
| "You're all set!" | "Configuration complete" |

### 6.3 Section Headings

| ❌ FORBIDDEN | ✓ CORRECT |
|-------------|-----------|
| "Welcome Back, John!" | "OPERATOR STATION" |
| "Your Dashboard" | "SYSTEM OVERVIEW" |
| "Recent Activity" | "OPERATIONS LOG" |
| "Quick Actions" | "CONTROLS" |

---

## 7. Quick Reference Card

### Colors (Only These)
- Background: `rams-chassis`, `rams-module`, `rams-panel`
- Borders: `rams-line`
- Text: `rams-foreground`, `rams-muted`
- Accents: `rams-orange`, `rams-green`, `rams-red`, `rams-steel`

### Radii (Only These)
- Default: `rounded-rams-sm` (2px)
- Containers: `rounded-rams-sm` (2px) or `rounded-rams-md` (4px)
- Maximum: `rounded-rams-lg` (8px) — rare

### Shadows (Only These)
- Inset: `shadow-rams-inset` for input fields
- Pressed: `shadow-rams-pressed` for active buttons
- Focus: `shadow-rams-focus` for focus rings

### Transitions (Only These)
- Instant: 50ms (microinteractions)
- Fast: 100ms (state changes)
- Normal: 150ms (appearance changes)
- Slow: 200ms (maximum)

### Typography
- Labels: `text-2xs font-mono font-bold uppercase tracking-widest text-rams-muted`
- Body: `text-sm font-sans text-rams-foreground`
- Data: `font-mono tabular-nums`
- Headers: `font-semibold tracking-tight`

---

*This document is a living anti-pattern catalog. Update it when new SaaS trends emerge that must be explicitly rejected.*
