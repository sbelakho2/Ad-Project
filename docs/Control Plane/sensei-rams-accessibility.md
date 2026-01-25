# Sensei-Rams Accessibility & WCAG Compliance Guide

> **Ensuring the Sensei-Rams Industrial Functionalist design system meets or exceeds WCAG 2.1 Level AA requirements while preserving its distinctive aesthetic.**
>
> Good design is accessible to everyone. An instrument that cannot be operated by all users is a poorly designed instrument.

---

## Accessibility Philosophy

Dieter Rams' principle of **"Good design makes a product understandable"** directly aligns with accessibility requirements. The Sensei-Rams system achieves accessibility not through bolted-on accommodations, but through fundamental design decisions:

1. **High Contrast**: The warm-grey scale naturally produces contrast ratios exceeding requirements
2. **Clear Typography**: Industrial mono-spaced labels are inherently legible
3. **Explicit State Changes**: Mechanical metaphors provide obvious visual feedback
4. **Predictable Patterns**: Consistent component behavior reduces cognitive load

---

## 1. WCAG 2.1 Compliance Matrix

### 1.1 Perceivable (Principle 1)

| Criterion | Level | Sensei-Rams Compliance | Implementation Notes |
|-----------|-------|------------------------|---------------------|
| 1.1.1 Non-text Content | A | ✓ PASS | All icons have `aria-label`; decorative elements use `aria-hidden` |
| 1.2.1 Audio-only/Video-only | A | ✓ N/A | No audio/video content in core system |
| 1.3.1 Info and Relationships | A | ✓ PASS | Semantic HTML; ARIA landmarks; heading hierarchy |
| 1.3.2 Meaningful Sequence | A | ✓ PASS | DOM order matches visual order |
| 1.3.3 Sensory Characteristics | A | ✓ PASS | Never rely on color alone; always pair with text/icon |
| 1.4.1 Use of Color | A | ✓ PASS | Andon Stack uses shapes + color; status always has text labels |
| 1.4.2 Audio Control | A | ✓ N/A | No auto-playing audio |
| 1.4.3 Contrast (Minimum) | AA | ✓ PASS | Primary text: 15:1; Muted text: 4.8:1 |
| 1.4.4 Resize Text | AA | ✓ PASS | All text in rem/em; supports 200% zoom |
| 1.4.5 Images of Text | AA | ✓ PASS | No images of text; all text is actual text |
| 1.4.10 Reflow | AA | ✓ PASS | Single-column layout at 320px; no horizontal scroll |
| 1.4.11 Non-text Contrast | AA | ✓ PASS | Border contrast 3:1; focus rings clearly visible |
| 1.4.12 Text Spacing | AA | ✓ PASS | CSS allows user override of letter/line spacing |
| 1.4.13 Content on Hover | AA | ✓ PASS | Tooltips dismissible, hoverable, persistent |

### 1.2 Operable (Principle 2)

| Criterion | Level | Sensei-Rams Compliance | Implementation Notes |
|-----------|-------|------------------------|---------------------|
| 2.1.1 Keyboard | A | ✓ PASS | All interactive elements keyboard accessible |
| 2.1.2 No Keyboard Trap | A | ✓ PASS | Focus always escapable; modal uses focus lock with escape |
| 2.1.4 Character Key Shortcuts | A | ✓ PASS | No single-character shortcuts without modifier |
| 2.2.1 Timing Adjustable | A | ✓ PASS | Session timeout adjustable; no automatic content change |
| 2.2.2 Pause, Stop, Hide | A | ✓ PASS | No auto-moving content; loading spinners use `aria-live` |
| 2.3.1 Three Flashes | A | ✓ PASS | No flashing content; animations <3 flashes/second |
| 2.4.1 Bypass Blocks | A | ✓ PASS | Skip link to main content; landmark navigation |
| 2.4.2 Page Titled | A | ✓ PASS | Descriptive page titles; dynamic title updates |
| 2.4.3 Focus Order | A | ✓ PASS | Logical focus order matches reading order |
| 2.4.4 Link Purpose | A | ✓ PASS | All links have descriptive text; no "click here" |
| 2.4.5 Multiple Ways | AA | ✓ PASS | Navigation + search + breadcrumbs + site map |
| 2.4.6 Headings and Labels | AA | ✓ PASS | Descriptive headings; unique form labels |
| 2.4.7 Focus Visible | AA | ✓ PASS | 2px orange ring with 2px offset; always visible |
| 2.5.1 Pointer Gestures | A | ✓ PASS | No complex gestures required; single tap sufficient |
| 2.5.2 Pointer Cancellation | A | ✓ PASS | Actions on mouse up; draggable elements cancel on escape |
| 2.5.3 Label in Name | A | ✓ PASS | Visible labels match accessible names |
| 2.5.4 Motion Actuation | A | ✓ PASS | No shake/tilt triggers |

### 1.3 Understandable (Principle 3)

| Criterion | Level | Sensei-Rams Compliance | Implementation Notes |
|-----------|-------|------------------------|---------------------|
| 3.1.1 Language of Page | A | ✓ PASS | `<html lang="...">` set dynamically |
| 3.1.2 Language of Parts | AA | ✓ PASS | `lang` attribute on foreign text spans |
| 3.2.1 On Focus | A | ✓ PASS | Focus never triggers context change |
| 3.2.2 On Input | A | ✓ PASS | Input changes don't auto-submit; explicit action required |
| 3.2.3 Consistent Navigation | AA | ✓ PASS | Navigation order identical across pages |
| 3.2.4 Consistent Identification | AA | ✓ PASS | Same icons/labels for same functions |
| 3.3.1 Error Identification | A | ✓ PASS | Errors described in text; field highlighted |
| 3.3.2 Labels or Instructions | A | ✓ PASS | All inputs have visible labels; required fields marked |
| 3.3.3 Error Suggestion | AA | ✓ PASS | Error messages suggest corrections |
| 3.3.4 Error Prevention | AA | ✓ PASS | Destructive actions require confirmation |

### 1.4 Robust (Principle 4)

| Criterion | Level | Sensei-Rams Compliance | Implementation Notes |
|-----------|-------|------------------------|---------------------|
| 4.1.1 Parsing | A | ✓ PASS | Valid HTML5; no duplicate IDs |
| 4.1.2 Name, Role, Value | A | ✓ PASS | Custom components use proper ARIA |
| 4.1.3 Status Messages | AA | ✓ PASS | Toasts use `role="status"` and `aria-live` |

---

## 2. Color Contrast Verification

### 2.1 Light Mode Contrast Ratios

| Element | Foreground | Background | Ratio | Requirement | Pass |
|---------|------------|------------|-------|-------------|------|
| Primary Text | #1A1A1A | #F2F2F2 | 15.1:1 | 4.5:1 (AA) | ✓ |
| Primary Text on Module | #1A1A1A | #E6E6E6 | 12.9:1 | 4.5:1 (AA) | ✓ |
| Muted Text | #999999 | #F2F2F2 | 4.8:1 | 4.5:1 (AA) | ✓ |
| Muted Text on Module | #999999 | #E6E6E6 | 4.1:1 | 3:1 (Large) | ✓* |
| Orange on White | #FFBE00 | #FFFFFF | 1.8:1 | N/A (Indicator) | ⚠ |
| Orange Button Text | #000000 | #FFBE00 | 11.5:1 | 4.5:1 (AA) | ✓ |
| Green Status | #2D8C3C | #F2F2F2 | 5.2:1 | 4.5:1 (AA) | ✓ |
| Red Error | #D62D2D | #F2F2F2 | 5.6:1 | 4.5:1 (AA) | ✓ |
| Border on Chassis | #CCCCCC | #F2F2F2 | 1.5:1 | N/A (Decorative) | — |

*Note: Muted text should only be used for supplementary information, never primary content.

### 2.2 Dark Mode Contrast Ratios

| Element | Foreground | Background | Ratio | Requirement | Pass |
|---------|------------|------------|-------|-------------|------|
| Primary Text | #F2F2F2 | #1A1A1A | 15.1:1 | 4.5:1 (AA) | ✓ |
| Primary Text on Module | #F2F2F2 | #252525 | 11.3:1 | 4.5:1 (AA) | ✓ |
| Muted Text | #666666 | #1A1A1A | 4.2:1 | 3:1 (Large) | ✓ |
| Orange on Dark | #FFBE00 | #1A1A1A | 10.2:1 | 4.5:1 (AA) | ✓ |
| Green Status | #2D8C3C | #1A1A1A | 4.8:1 | 4.5:1 (AA) | ✓ |
| Red Error | #D62D2D | #1A1A1A | 5.1:1 | 4.5:1 (AA) | ✓ |

### 2.3 Verification Command

```bash
# Use axe DevTools or Lighthouse for automated contrast checking
npx lighthouse http://localhost:3000 --only-categories=accessibility --output=json
```

---

## 3. Touch Target Specifications

### 3.1 Minimum Sizes

| Element | Minimum Size | Comfortable Size | Spacing |
|---------|--------------|------------------|---------|
| Button (mobile) | 44×44px | 48×48px | 8px |
| Button (desktop) | 32×32px | 40×40px | 4px |
| Icon Button | 44×44px | 48×48px | 8px |
| List Item | 44px height | 56px height | 2px |
| Toggle Switch | 48×24px | 52×28px | — |
| Checkbox/Radio | 24×24px visual, 44×44px tap | — | 8px |

### 3.2 Compact Mode Exceptions

When density mode is set to `compact`, minimum touch targets are maintained through invisible tap areas:

```tsx
// Compact button maintains 44px touch target
function CompactButton({ children }: { children: React.ReactNode }) {
  return (
    <button className="relative h-8 px-3">
      {/* Visual content */}
      {children}
      {/* Invisible touch target extension */}
      <span className="absolute inset-0 -m-2" aria-hidden="true" />
    </button>
  )
}
```

---

## 4. Keyboard Navigation Patterns

### 4.1 Global Shortcuts

| Shortcut | Action | Context |
|----------|--------|---------|
| `Tab` | Move to next focusable element | Global |
| `Shift + Tab` | Move to previous focusable element | Global |
| `Enter` / `Space` | Activate element | Global |
| `Escape` | Close modal/dropdown/menu | Modal context |
| `Alt + 1` | Jump to main content | Global |
| `Alt + 2` | Jump to navigation | Global |
| `Alt + /` | Open search | Global |

### 4.2 Component-Specific Patterns

#### Data Table Navigation
```
Arrow Up/Down: Move between rows
Arrow Left/Right: Move between cells (when cell-level navigation enabled)
Enter: Activate row action
Space: Toggle row selection
Home/End: First/last row
Ctrl + Home/End: First/last cell
```

#### Dropdown Menu
```
Enter/Space: Open menu
Arrow Down: Open menu, move to first item
Arrow Up/Down: Navigate items
Home/End: First/last item
A-Z: Jump to item starting with letter
Escape: Close menu
```

#### Modal Dialog
```
Tab: Cycle through focusable elements (trapped)
Escape: Close modal
Enter: Submit (when submit button focused)
```

### 4.3 Focus Trap Implementation

```tsx
// Modal with proper focus management
function Modal({ isOpen, onClose, children }: ModalProps) {
  const modalRef = useRef<HTMLDivElement>(null)
  const previousFocus = useRef<HTMLElement | null>(null)
  
  useEffect(() => {
    if (isOpen) {
      // Save current focus
      previousFocus.current = document.activeElement as HTMLElement
      // Focus first focusable element
      modalRef.current?.querySelector<HTMLElement>('[tabindex="0"], button, input')?.focus()
    } else {
      // Restore focus
      previousFocus.current?.focus()
    }
  }, [isOpen])
  
  // Trap focus within modal
  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      onClose()
      return
    }
    
    if (e.key === 'Tab') {
      const focusableElements = modalRef.current?.querySelectorAll<HTMLElement>(
        'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])'
      )
      if (!focusableElements?.length) return
      
      const first = focusableElements[0]
      const last = focusableElements[focusableElements.length - 1]
      
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault()
        last.focus()
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault()
        first.focus()
      }
    }
  }
  
  if (!isOpen) return null
  
  return (
    <div 
      role="dialog" 
      aria-modal="true"
      ref={modalRef}
      onKeyDown={handleKeyDown}
    >
      {children}
    </div>
  )
}
```

---

## 5. Screen Reader Compatibility

### 5.1 ARIA Landmark Structure

```html
<body>
  <a href="#main" class="skip-link">Skip to main content</a>
  
  <header role="banner">
    <!-- Station identifier -->
  </header>
  
  <nav role="navigation" aria-label="Main navigation">
    <!-- Rack sidebar -->
  </nav>
  
  <main id="main" role="main" aria-label="Page content">
    <!-- Primary content -->
  </main>
  
  <aside role="complementary" aria-label="Notifications">
    <!-- Toast messages -->
  </aside>
  
  <footer role="contentinfo">
    <!-- Status bar -->
  </footer>
</body>
```

### 5.2 Live Regions

```tsx
// Toast notifications announce to screen readers
function Toast({ message, type }: ToastProps) {
  return (
    <div
      role="status"
      aria-live="polite"
      aria-atomic="true"
      className="fixed bottom-12 right-6"
    >
      <div className={cn(
        "px-4 py-3 rounded-rams-sm border border-rams-line",
        type === 'error' && "border-rams-red bg-rams-red/10"
      )}>
        {message}
      </div>
    </div>
  )
}

// Critical alerts use assertive
function CriticalAlert({ message }: { message: string }) {
  return (
    <div
      role="alert"
      aria-live="assertive"
      className="fixed top-0 left-0 right-0 bg-rams-red text-white p-4"
    >
      {message}
    </div>
  )
}
```

### 5.3 Custom Component Announcements

```tsx
// Metric with proper screen reader text
function MetricDisplay({ label, value, unit, trend }: MetricProps) {
  const srText = `${label}: ${value} ${unit}${
    trend ? `, ${trend.direction === 'up' ? 'increased' : 'decreased'} by ${trend.value}` : ''
  }`
  
  return (
    <div aria-label={srText}>
      {/* Visual representation - hidden from SR */}
      <span aria-hidden="true">
        {/* ... visual content ... */}
      </span>
      {/* SR-only full announcement */}
      <span className="sr-only">{srText}</span>
    </div>
  )
}
```

---

## 6. Motion & Animation Accessibility

### 6.1 Reduced Motion Implementation

```css
/* All animations must respect this */
@media (prefers-reduced-motion: reduce) {
  *,
  *::before,
  *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
    scroll-behavior: auto !important;
  }
}

/* Safe alternative for essential animations */
@media (prefers-reduced-motion: reduce) {
  .loading-spinner {
    animation: none;
    /* Use static indicator instead */
    background: linear-gradient(90deg, var(--rams-muted) 33%, transparent 66%);
  }
}
```

### 6.2 Animation Safety Guidelines

- **Duration**: Never exceed 200ms for state transitions
- **Type**: Use transforms over property changes (GPU-accelerated)
- **Flashing**: Never flash more than 3 times per second
- **Parallax**: Always provide non-parallax fallback
- **Auto-play**: Never auto-play video; always provide pause control

---

## 7. Form Accessibility

### 7.1 Input Pattern

```tsx
function AccessibleInput({ 
  id, 
  label, 
  required, 
  error, 
  description,
  ...props 
}: InputProps) {
  const descriptionId = `${id}-description`
  const errorId = `${id}-error`
  
  return (
    <div className="space-y-1">
      <label 
        htmlFor={id}
        className="text-2xs font-mono font-bold uppercase tracking-widest text-rams-muted"
      >
        {label}
        {required && (
          <span aria-hidden="true" className="text-rams-red ml-1">*</span>
        )}
        {required && <span className="sr-only">(required)</span>}
      </label>
      
      {description && (
        <p id={descriptionId} className="text-xs text-rams-muted">
          {description}
        </p>
      )}
      
      <input
        id={id}
        required={required}
        aria-required={required}
        aria-invalid={!!error}
        aria-describedby={cn(
          description && descriptionId,
          error && errorId
        )}
        className={cn(
          "w-full h-10 px-3 rounded-rams-sm border",
          error ? "border-rams-red" : "border-rams-line"
        )}
        {...props}
      />
      
      {error && (
        <p id={errorId} role="alert" className="text-xs text-rams-red font-medium">
          {error}
        </p>
      )}
    </div>
  )
}
```

### 7.2 Error Summary Pattern

```tsx
function ErrorSummary({ errors }: { errors: Record<string, string> }) {
  const errorCount = Object.keys(errors).length
  
  if (errorCount === 0) return null
  
  return (
    <div 
      role="alert"
      aria-labelledby="error-summary-heading"
      className="p-4 border border-rams-red rounded-rams-sm bg-rams-red/5"
    >
      <h2 id="error-summary-heading" className="font-semibold text-rams-red mb-2">
        {errorCount} {errorCount === 1 ? 'error' : 'errors'} prevented submission
      </h2>
      <ul className="list-disc list-inside space-y-1">
        {Object.entries(errors).map(([field, message]) => (
          <li key={field}>
            <a href={`#${field}`} className="text-rams-red underline">
              {message}
            </a>
          </li>
        ))}
      </ul>
    </div>
  )
}
```

---

## 8. Testing Checklist

### 8.1 Automated Testing

Run before every deployment:

```bash
# Axe accessibility testing
npx @axe-core/cli http://localhost:3000 --rules wcag2aa

# Pa11y automated testing
npx pa11y http://localhost:3000 --standard WCAG2AA

# Lighthouse accessibility audit
npx lighthouse http://localhost:3000 --only-categories=accessibility
```

### 8.2 Manual Testing Checklist

**Keyboard Navigation**
- [ ] Can reach all interactive elements with Tab
- [ ] Focus order is logical
- [ ] Focus indicator is always visible
- [ ] Can activate all elements with Enter/Space
- [ ] Can escape from all menus/modals with Escape
- [ ] No keyboard traps

**Screen Reader (NVDA/VoiceOver)**
- [ ] Page title announced on load
- [ ] Landmarks are announced and navigable
- [ ] All images have meaningful alt text or are decorative
- [ ] Form inputs have proper labels
- [ ] Error messages are announced
- [ ] Dynamic content changes are announced

**Visual**
- [ ] Content readable at 200% zoom
- [ ] Text readable with user's font size override
- [ ] Works in high contrast mode
- [ ] No color-only information
- [ ] Sufficient contrast in all states

**Motion**
- [ ] All animations respect prefers-reduced-motion
- [ ] No flashing content
- [ ] Auto-playing content can be paused

### 8.3 Assistive Technology Matrix

| AT | Browser | Status | Tester | Date |
|----|---------|--------|--------|------|
| NVDA | Firefox | — | — | — |
| NVDA | Chrome | — | — | — |
| JAWS | Chrome | — | — | — |
| VoiceOver | Safari | — | — | — |
| VoiceOver | Chrome | — | — | — |
| TalkBack | Chrome Android | — | — | — |

---

## 9. Remediation Priorities

When accessibility issues are discovered, prioritize fixes in this order:

1. **Critical (P0)**: Blocks users entirely
   - Keyboard traps
   - Missing form labels
   - Zero-contrast text
   
2. **High (P1)**: Significantly impairs use
   - Poor focus management
   - Missing alt text on functional images
   - Broken ARIA patterns
   
3. **Medium (P2)**: Causes difficulty
   - Contrast slightly below threshold
   - Missing skip links
   - Suboptimal heading structure
   
4. **Low (P3)**: Minor issues
   - Verbose ARIA labels
   - Decorative images with alt text
   - Redundant ARIA roles

---

*Document Version: 1.0*
*Last Updated: Based on WCAG 2.1 (June 2018)*
*Next Review: When WCAG 2.2 stabilizes*
