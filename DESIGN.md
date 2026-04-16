# NetSentinel Design System — Material Design 3 (M3)

> Migration guide: CSS variable-based theming aligned with [Material Design 3](https://m3.material.io/) principles.
> This document defines the target design tokens, component patterns, and migration rules.

---

## 1. Color System

M3 uses a **tonal palette** generated from a single source (seed) color. NetSentinel's seed color is **Blue (#3B82F6)**.

### 1.1 Role-Based Tokens

Replace the current flat `--accent-*` / `--bg-*` tokens with M3 semantic roles.
All values are CSS custom properties defined in `globals.css` under `:root` (light) and `[data-theme="dark"]`.

```
Current Token              → M3 Token
──────────────────────────────────────────────────
--bg-primary               → --md-sys-color-surface
--bg-secondary             → --md-sys-color-surface-container
--bg-card                  → --md-sys-color-surface-container-low
--bg-card-hover            → --md-sys-color-surface-container-high
--bg-muted                 → --md-sys-color-surface-variant
--text-primary             → --md-sys-color-on-surface
--text-secondary           → --md-sys-color-on-surface-variant
--text-muted               → --md-sys-color-outline
--accent-blue              → --md-sys-color-primary
--accent-purple            → --md-sys-color-tertiary
--accent-red               → --md-sys-color-error
--accent-green             → --md-custom-color-success
--accent-yellow            → --md-custom-color-warning
--accent-cyan              → --md-custom-color-info
--border-subtle            → --md-sys-color-outline-variant
--border-glow              → --md-sys-color-primary (with opacity)
```

### 1.2 Light Theme Palette

```css
:root {
  /* Primary (Blue seed) */
  --md-sys-color-primary: #1B6EF3;
  --md-sys-color-on-primary: #FFFFFF;
  --md-sys-color-primary-container: #D6E3FF;
  --md-sys-color-on-primary-container: #001B3E;

  /* Secondary (Neutral blue) */
  --md-sys-color-secondary: #555F71;
  --md-sys-color-on-secondary: #FFFFFF;
  --md-sys-color-secondary-container: #D9E3F8;
  --md-sys-color-on-secondary-container: #121C2B;

  /* Tertiary (Purple) */
  --md-sys-color-tertiary: #6E5676;
  --md-sys-color-on-tertiary: #FFFFFF;
  --md-sys-color-tertiary-container: #F8D8FF;
  --md-sys-color-on-tertiary-container: #27132F;

  /* Error */
  --md-sys-color-error: #BA1A1A;
  --md-sys-color-on-error: #FFFFFF;
  --md-sys-color-error-container: #FFDAD6;
  --md-sys-color-on-error-container: #410002;

  /* Surfaces */
  --md-sys-color-surface: #F9F9FF;
  --md-sys-color-on-surface: #1A1C20;
  --md-sys-color-on-surface-variant: #44474E;
  --md-sys-color-surface-container-lowest: #FFFFFF;
  --md-sys-color-surface-container-low: #F3F3FA;
  --md-sys-color-surface-container: #EDEDF4;
  --md-sys-color-surface-container-high: #E7E8EE;
  --md-sys-color-surface-container-highest: #E2E2E9;

  /* Outline */
  --md-sys-color-outline: #74777F;
  --md-sys-color-outline-variant: #C4C6D0;

  /* Custom semantic (monitoring-specific) */
  --md-custom-color-success: #1B873B;
  --md-custom-color-on-success: #FFFFFF;
  --md-custom-color-warning: #8B6914;
  --md-custom-color-on-warning: #FFFFFF;
  --md-custom-color-info: #0E7490;
  --md-custom-color-on-info: #FFFFFF;
}
```

### 1.3 Dark Theme Palette

```css
[data-theme="dark"] {
  --md-sys-color-primary: #AAC7FF;
  --md-sys-color-on-primary: #002F65;
  --md-sys-color-primary-container: #00458E;
  --md-sys-color-on-primary-container: #D6E3FF;

  --md-sys-color-secondary: #BDC7DC;
  --md-sys-color-on-secondary: #283141;
  --md-sys-color-secondary-container: #3E4759;
  --md-sys-color-on-secondary-container: #D9E3F8;

  --md-sys-color-tertiary: #DDB9E7;
  --md-sys-color-on-tertiary: #3F2846;
  --md-sys-color-tertiary-container: #573E5D;
  --md-sys-color-on-tertiary-container: #F8D8FF;

  --md-sys-color-error: #FFB4AB;
  --md-sys-color-on-error: #690005;
  --md-sys-color-error-container: #93000A;
  --md-sys-color-on-error-container: #FFDAD6;

  --md-sys-color-surface: #121316;
  --md-sys-color-on-surface: #E2E2E9;
  --md-sys-color-on-surface-variant: #C4C6D0;
  --md-sys-color-surface-container-lowest: #0D0E11;
  --md-sys-color-surface-container-low: #1A1C20;
  --md-sys-color-surface-container: #1E2025;
  --md-sys-color-surface-container-high: #292A2F;
  --md-sys-color-surface-container-highest: #33353A;

  --md-sys-color-outline: #8E9099;
  --md-sys-color-outline-variant: #44474E;

  --md-custom-color-success: #73DB8E;
  --md-custom-color-on-success: #00391A;
  --md-custom-color-warning: #E5C04D;
  --md-custom-color-on-warning: #3D2E00;
  --md-custom-color-info: #4FD1E5;
  --md-custom-color-on-info: #003640;
}
```

### 1.4 Chart Palette

Charts retain distinct accent colors for multi-series readability, but mapped to M3-compatible tones:

```css
:root {
  --md-chart-1: var(--md-sys-color-primary);        /* CPU */
  --md-chart-2: var(--md-sys-color-tertiary);        /* Memory */
  --md-chart-3: var(--md-custom-color-success);      /* Network RX */
  --md-chart-4: var(--md-custom-color-info);         /* Network TX / Disk IO */
  --md-chart-5: var(--md-custom-color-warning);      /* Disk usage */
  --md-chart-6: var(--md-sys-color-error);           /* Temperature */
}
```

---

## 2. Typography

M3 type scale using **Inter** (existing). Mono: **JetBrains Mono** (existing).

```css
:root {
  /* Display */
  --md-sys-typescale-display-large: 600 2.25rem/2.75rem var(--font-inter);
  --md-sys-typescale-display-medium: 600 1.75rem/2.25rem var(--font-inter);

  /* Headline */
  --md-sys-typescale-headline-small: 600 1.25rem/1.75rem var(--font-inter);

  /* Title */
  --md-sys-typescale-title-large: 600 1.125rem/1.5rem var(--font-inter);
  --md-sys-typescale-title-medium: 600 0.875rem/1.25rem var(--font-inter);
  --md-sys-typescale-title-small: 500 0.8125rem/1.125rem var(--font-inter);

  /* Body */
  --md-sys-typescale-body-large: 400 0.875rem/1.375rem var(--font-inter);
  --md-sys-typescale-body-medium: 400 0.8125rem/1.25rem var(--font-inter);
  --md-sys-typescale-body-small: 400 0.75rem/1rem var(--font-inter);

  /* Label */
  --md-sys-typescale-label-large: 500 0.8125rem/1.125rem var(--font-inter);
  --md-sys-typescale-label-medium: 500 0.6875rem/1rem var(--font-inter);
  --md-sys-typescale-label-small: 500 0.625rem/0.875rem var(--font-inter);
}
```

### Usage mapping

| Context | Type scale | Example |
|---|---|---|
| Page title (`<h1>`) | `headline-small` | "Dashboard", host display name |
| Card title | `title-medium` | "CPU Usage", "Port Status" |
| Table header | `label-large` | "System", "CPU", "Memory" |
| Table body | `body-medium` | metric values, timestamps |
| Chart axis label | `label-small` | "10:30", "50%" |
| Monospace values | `body-medium` + `--font-mono` | IP addresses, percentages |
| Tooltip | `body-small` | chart hover detail |

---

## 3. Elevation & Surfaces

M3 replaces `box-shadow` elevation with **tonal surface colors**. Higher elevation = lighter surface-container.

| M3 Level | Token | Use case |
|---|---|---|
| Level 0 | `surface` | Page background |
| Level 1 | `surface-container-low` | Cards (`.glass-card`) |
| Level 2 | `surface-container` | Navbar, modals |
| Level 3 | `surface-container-high` | Card hover, dropdowns |
| Level 4 | `surface-container-highest` | Tooltips, popovers |

### Migration rule
```css
/* Before */
.glass-card {
  background: var(--bg-card);
  box-shadow: var(--shadow-sm);
}

/* After (M3) */
.glass-card {
  background: var(--md-sys-color-surface-container-low);
  border: 1px solid var(--md-sys-color-outline-variant);
  border-radius: var(--md-sys-shape-corner-large); /* 16px */
}
```

### Shape scale

```css
:root {
  --md-sys-shape-corner-none: 0;
  --md-sys-shape-corner-extra-small: 4px;
  --md-sys-shape-corner-small: 8px;
  --md-sys-shape-corner-medium: 12px;
  --md-sys-shape-corner-large: 16px;
  --md-sys-shape-corner-extra-large: 28px;
  --md-sys-shape-corner-full: 9999px;
}
```

| Component | Shape |
|---|---|
| Button | `corner-full` (pill) |
| Card | `corner-large` |
| Text input | `corner-small` |
| Chip / Badge | `corner-small` |
| Dialog | `corner-extra-large` |
| Tooltip | `corner-small` |
| Navbar | `corner-none` |

---

## 4. Component Patterns

### 4.1 Buttons

```
Filled    → primary action (Save, Create)
Tonal     → secondary action (Cancel, Filter)
Outlined  → tertiary (Settings, Export)
Text      → low-emphasis (Back, Learn more)
Icon      → toolbar (theme toggle, locale, logout)
FAB       → not used in monitoring context
```

```css
.btn-filled {
  background: var(--md-sys-color-primary);
  color: var(--md-sys-color-on-primary);
  border: none;
  border-radius: var(--md-sys-shape-corner-full);
  padding: 10px 24px;
  font: var(--md-sys-typescale-label-large);
}

.btn-tonal {
  background: var(--md-sys-color-secondary-container);
  color: var(--md-sys-color-on-secondary-container);
  border: none;
  border-radius: var(--md-sys-shape-corner-full);
  padding: 10px 24px;
}

.btn-outlined {
  background: transparent;
  color: var(--md-sys-color-primary);
  border: 1px solid var(--md-sys-color-outline);
  border-radius: var(--md-sys-shape-corner-full);
  padding: 10px 24px;
}

.btn-icon {
  background: transparent;
  color: var(--md-sys-color-on-surface-variant);
  border: none;
  border-radius: var(--md-sys-shape-corner-full);
  padding: 8px;
  width: 40px;
  height: 40px;
}
.btn-icon:hover {
  background: color-mix(in srgb, var(--md-sys-color-on-surface-variant) 8%, transparent);
}
```

### 4.2 Cards (Metric Cards)

```css
.metric-card {
  background: var(--md-sys-color-surface-container-low);
  border: 1px solid var(--md-sys-color-outline-variant);
  border-radius: var(--md-sys-shape-corner-large);
  padding: 20px;
}

.metric-card:hover {
  background: var(--md-sys-color-surface-container);
}
```

### 4.3 Status Badges

Map existing badge tokens to M3 custom colors:

```css
.badge-online {
  background: color-mix(in srgb, var(--md-custom-color-success) 12%, transparent);
  color: var(--md-custom-color-success);
  border-radius: var(--md-sys-shape-corner-small);
  font: var(--md-sys-typescale-label-small);
  padding: 2px 8px;
}

.badge-offline {
  background: color-mix(in srgb, var(--md-sys-color-error) 12%, transparent);
  color: var(--md-sys-color-error);
}

.badge-pending {
  background: color-mix(in srgb, var(--md-custom-color-warning) 12%, transparent);
  color: var(--md-custom-color-warning);
}
```

### 4.4 Navigation Bar

M3 top app bar pattern:

```css
.navbar {
  background: var(--md-sys-color-surface-container);
  border-bottom: 1px solid var(--md-sys-color-outline-variant);
  height: 64px;
  padding: 0 16px;
}

.navbar-title {
  font: var(--md-sys-typescale-title-large);
  color: var(--md-sys-color-on-surface);
}
```

### 4.5 Data Table

```css
.data-table thead {
  background: var(--md-sys-color-surface-container);
}

.data-table th {
  font: var(--md-sys-typescale-label-large);
  color: var(--md-sys-color-on-surface-variant);
  border-bottom: 1px solid var(--md-sys-color-outline-variant);
}

.data-table td {
  font: var(--md-sys-typescale-body-medium);
  color: var(--md-sys-color-on-surface);
  border-bottom: 1px solid var(--md-sys-color-outline-variant);
}

.data-table tr:hover {
  background: color-mix(in srgb, var(--md-sys-color-on-surface) 4%, transparent);
}
```

### 4.6 Text Input

```css
.text-field {
  background: transparent;
  border: 1px solid var(--md-sys-color-outline);
  border-radius: var(--md-sys-shape-corner-small);
  padding: 12px 16px;
  font: var(--md-sys-typescale-body-large);
  color: var(--md-sys-color-on-surface);
  caret-color: var(--md-sys-color-primary);
}

.text-field:focus {
  border-color: var(--md-sys-color-primary);
  outline: none;
  border-width: 2px;
  padding: 11px 15px; /* compensate for border width */
}

.text-field-label {
  font: var(--md-sys-typescale-body-small);
  color: var(--md-sys-color-on-surface-variant);
}
```

### 4.7 Toggle Switch

For alert config toggles (replaces current `<div>` toggle):

```css
.switch {
  width: 52px;
  height: 32px;
  background: var(--md-sys-color-surface-container-highest);
  border: 2px solid var(--md-sys-color-outline);
  border-radius: var(--md-sys-shape-corner-full);
  position: relative;
  cursor: pointer;
  transition: background 200ms, border-color 200ms;
}

.switch[aria-checked="true"] {
  background: var(--md-sys-color-primary);
  border-color: var(--md-sys-color-primary);
}

.switch::after {
  content: "";
  position: absolute;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: var(--md-sys-color-outline);
  top: 6px;
  left: 6px;
  transition: transform 200ms, width 200ms, background 200ms;
}

.switch[aria-checked="true"]::after {
  transform: translateX(20px);
  width: 24px;
  height: 24px;
  top: 2px;
  background: var(--md-sys-color-on-primary);
}
```

---

## 5. Motion

M3 easing and duration tokens:

```css
:root {
  /* Duration */
  --md-sys-motion-duration-short1: 50ms;
  --md-sys-motion-duration-short2: 100ms;
  --md-sys-motion-duration-medium1: 200ms;
  --md-sys-motion-duration-medium2: 300ms;
  --md-sys-motion-duration-long1: 450ms;

  /* Easing */
  --md-sys-motion-easing-standard: cubic-bezier(0.2, 0, 0, 1);
  --md-sys-motion-easing-emphasized: cubic-bezier(0.2, 0, 0, 1);
  --md-sys-motion-easing-emphasized-decelerate: cubic-bezier(0.05, 0.7, 0.1, 1);
  --md-sys-motion-easing-emphasized-accelerate: cubic-bezier(0.3, 0, 0.8, 0.15);
}
```

### Usage

| Interaction | Duration | Easing |
|---|---|---|
| Button ripple | `short2` | `standard` |
| Card hover | `medium1` | `standard` |
| Page transition | `medium2` | `emphasized` |
| Modal open | `medium2` | `emphasized-decelerate` |
| Modal close | `short2` | `emphasized-accelerate` |
| Tooltip show | `short2` | `standard` |
| Theme switch | `medium2` | `emphasized` |

---

## 6. Spacing & Layout

M3 uses a **4px grid**. Existing `--radius-sm/md/lg` migrate to shape tokens.

```css
:root {
  --md-sys-spacing-xs: 4px;
  --md-sys-spacing-sm: 8px;
  --md-sys-spacing-md: 12px;
  --md-sys-spacing-lg: 16px;
  --md-sys-spacing-xl: 24px;
  --md-sys-spacing-2xl: 32px;
  --md-sys-spacing-3xl: 48px;
}
```

### Page layout

```
┌─────────────────────────────────────┐
│  Navbar (64px, surface-container)   │
├─────────────────────────────────────┤
│                                     │
│  Content (max-width: 1280px)        │
│  padding: 24px                      │
│                                     │
│  ┌─────────┐ ┌─────────┐           │
│  │  Card    │ │  Card    │          │
│  │  (16px   │ │  (16px   │          │
│  │  radius) │ │  radius) │          │
│  └─────────┘ └─────────┘           │
│                                     │
└─────────────────────────────────────┘
```

### Responsive breakpoints

| Breakpoint | Width | Layout |
|---|---|---|
| Compact | `< 600px` | Single column, stacked cards |
| Medium | `600–960px` | 2-column chart grid |
| Expanded | `> 960px` | Full multi-column grid |

---

## 7. Migration Checklist

### Phase 1: Tokens (non-breaking)
- [ ] Add all `--md-sys-*` and `--md-custom-*` tokens to `globals.css`
- [ ] Create aliases: `--bg-primary: var(--md-sys-color-surface)` etc.
- [ ] Verify both light and dark themes render correctly

### Phase 2: Shape & Typography
- [ ] Replace `--radius-sm/md/lg` with `--md-sys-shape-corner-*`
- [ ] Apply type scale to headings, labels, body text
- [ ] Update card border-radius to `corner-large` (16px)

### Phase 3: Components
- [ ] Migrate buttons to M3 variants (filled, tonal, outlined, icon)
- [ ] Replace glass-card with M3 surface-container elevation
- [ ] Add proper toggle switch with `role="switch"` and `aria-checked`
- [ ] Update data tables to M3 pattern
- [ ] Update text inputs to M3 outlined field style

### Phase 4: Motion & Polish
- [ ] Add M3 easing tokens to transitions
- [ ] Implement state layers (hover: 8% overlay, pressed: 12%)
- [ ] Verify focus-visible rings use `--md-sys-color-primary`

### Rules
- **Never use hardcoded colors** — always reference `--md-sys-*` or `--md-custom-*` tokens.
- **State layers** — hover/pressed/focus use `color-mix()` with on-surface color at 8%/12%/12% opacity.
- **Dark mode** — M3 dark surfaces are NOT pure black. Use tonal `surface-container` values.
- **Accessibility** — minimum 4.5:1 contrast for body text, 3:1 for large text and UI components. All interactive elements must have focus-visible indicator.
