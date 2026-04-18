# NetSentinel Design System — Material Design 3 (Adapted)

> Design token reference and component patterns based on [Material Design 3](https://m3.material.io/),
> adapted for monitoring dashboard information density.
>
> **Seed color:** Blue (#3B82F6) · **Theming:** CSS custom properties in `globals.css`

## ⚠️ Golden Rules — Read Before Any UI Work

These rules apply to every component, page, and style change. Violations must be corrected before submitting.

| ❌ Never use | ✅ Use instead |
|---|---|
| Hardcoded colors: `#3B82F6`, `blue`, `rgba(0,0,0,0.5)` | `var(--md-sys-color-*)` or `var(--md-custom-color-*)` |
| `box-shadow` for elevation | `background: var(--md-sys-color-surface-container-*)` |
| Raw border-radius: `border-radius: 12px` | `var(--md-sys-shape-corner-*)` |
| Raw font styles: `font-size: 14px; font-weight: 500` | `font: var(--md-sys-typescale-*)` |
| Raw transitions: `transition: 0.2s ease` | `var(--md-sys-motion-duration-*) var(--md-sys-motion-easing-*)` |
| Pure black in dark mode: `background: #000` | `var(--md-sys-color-surface)` or `surface-container-*` |
| State layers via `opacity` or `:hover { background: #eee }` | `color-mix(in srgb, var(--md-sys-color-on-*) 8%, transparent)` |
| New custom CSS variables outside the token system | Extend via `--md-custom-color-*` only if truly necessary |

> If you find yourself writing a raw value, stop and find the correct token in this document first.

---

## Adaptations from M3

This design system follows M3's structure, naming, and principles with these intentional adaptations for a monitoring dashboard context:

| Area | M3 Default | NetSentinel Adaptation | Rationale |
|---|---|---|---|
| Typography scale | Display 57px, Body 16px | Display 40px, Body 14px | Higher information density for metric grids |
| Typography weight | Display/Headline 400 | Display/Headline 400 (aligned) | — |
| Card shape | `corner-medium` (12px) | `corner-large` (16px) | Stronger visual grouping in card-heavy layouts |
| Text input shape | `corner-extra-small` (4px) | `corner-small` (8px) | Visual consistency with card-heavy UI |
| Breakpoints | 5 tiers (compact → extra-large) | 4 tiers (compact → large) | Dashboard doesn't benefit from large/extra-large split |

All other tokens and patterns follow M3 specification directly.

---

## 1. Color System

M3 uses a **tonal palette** generated from a single seed color. All values are CSS custom properties in `globals.css` under `:root` (light) and `[data-theme="dark"]`.

### 1.1 Token Migration

```
Current Token              → M3 Token
──────────────────────────────────────────────────
--bg-primary               → --md-sys-color-surface
--bg-secondary             → --md-sys-color-surface-container
--bg-card                  → --md-sys-color-surface-container-low
--bg-card-hover            → --md-sys-color-surface-container-high
--bg-muted                 → --md-sys-color-surface-container-highest
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
  /* Primary (Blue seed #3B82F6) */
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
  --md-sys-color-surface-dim: #D9D9E0;
  --md-sys-color-surface-bright: #F9F9FF;
  --md-sys-color-on-surface: #1A1C20;
  --md-sys-color-on-surface-variant: #44474E;
  --md-sys-color-surface-container-lowest: #FFFFFF;
  --md-sys-color-surface-container-low: #F3F3FA;
  --md-sys-color-surface-container: #EDEDF4;
  --md-sys-color-surface-container-high: #E7E8EE;
  --md-sys-color-surface-container-highest: #E2E2E9;
  --md-sys-color-surface-tint: #1B6EF3;

  /* Outline */
  --md-sys-color-outline: #74777F;
  --md-sys-color-outline-variant: #C4C6D0;

  /* Inverse (snackbar / toast) */
  --md-sys-color-inverse-surface: #2F3036;
  --md-sys-color-inverse-on-surface: #F1F0F7;
  --md-sys-color-inverse-primary: #AAC7FF;

  /* Utility */
  --md-sys-color-scrim: #000000;
  --md-sys-color-shadow: #000000;

  /* Custom semantic (monitoring-specific) */
  --md-custom-color-success: #1B873B;
  --md-custom-color-on-success: #FFFFFF;
  --md-custom-color-success-container: #C4F0CD;
  --md-custom-color-on-success-container: #00210B;
  --md-custom-color-warning: #8B6914;
  --md-custom-color-on-warning: #FFFFFF;
  --md-custom-color-warning-container: #FFDEA3;
  --md-custom-color-on-warning-container: #2B1E00;
  --md-custom-color-info: #0E7490;
  --md-custom-color-on-info: #FFFFFF;
  --md-custom-color-info-container: #B8EAFF;
  --md-custom-color-on-info-container: #001F2A;
}
```

### 1.3 Dark Theme Palette

```css
[data-theme="dark"] {
  /* Primary */
  --md-sys-color-primary: #AAC7FF;
  --md-sys-color-on-primary: #002F65;
  --md-sys-color-primary-container: #00458E;
  --md-sys-color-on-primary-container: #D6E3FF;

  /* Secondary */
  --md-sys-color-secondary: #BDC7DC;
  --md-sys-color-on-secondary: #283141;
  --md-sys-color-secondary-container: #3E4759;
  --md-sys-color-on-secondary-container: #D9E3F8;

  /* Tertiary */
  --md-sys-color-tertiary: #DDB9E7;
  --md-sys-color-on-tertiary: #3F2846;
  --md-sys-color-tertiary-container: #573E5D;
  --md-sys-color-on-tertiary-container: #F8D8FF;

  /* Error */
  --md-sys-color-error: #FFB4AB;
  --md-sys-color-on-error: #690005;
  --md-sys-color-error-container: #93000A;
  --md-sys-color-on-error-container: #FFDAD6;

  /* Surfaces */
  --md-sys-color-surface: #121316;
  --md-sys-color-surface-dim: #121316;
  --md-sys-color-surface-bright: #383A3F;
  --md-sys-color-on-surface: #E2E2E9;
  --md-sys-color-on-surface-variant: #C4C6D0;
  --md-sys-color-surface-container-lowest: #0D0E11;
  --md-sys-color-surface-container-low: #1A1C20;
  --md-sys-color-surface-container: #1E2025;
  --md-sys-color-surface-container-high: #292A2F;
  --md-sys-color-surface-container-highest: #33353A;
  --md-sys-color-surface-tint: #AAC7FF;

  /* Outline */
  --md-sys-color-outline: #8E9099;
  --md-sys-color-outline-variant: #44474E;

  /* Inverse */
  --md-sys-color-inverse-surface: #E2E2E9;
  --md-sys-color-inverse-on-surface: #2F3036;
  --md-sys-color-inverse-primary: #1B6EF3;

  /* Utility */
  --md-sys-color-scrim: #000000;
  --md-sys-color-shadow: #000000;

  /* Custom semantic */
  --md-custom-color-success: #73DB8E;
  --md-custom-color-on-success: #00391A;
  --md-custom-color-success-container: #005227;
  --md-custom-color-on-success-container: #C4F0CD;
  --md-custom-color-warning: #E5C04D;
  --md-custom-color-on-warning: #3D2E00;
  --md-custom-color-warning-container: #584400;
  --md-custom-color-on-warning-container: #FFDEA3;
  --md-custom-color-info: #4FD1E5;
  --md-custom-color-on-info: #003640;
  --md-custom-color-info-container: #004E5C;
  --md-custom-color-on-info-container: #B8EAFF;
}
```

### 1.4 Chart Palette

Charts use distinct accent colors for multi-series readability:

```css
:root {
  --md-chart-1: var(--md-sys-color-primary);        /* CPU */
  --md-chart-2: var(--md-sys-color-tertiary);        /* Memory */
  --md-chart-3: var(--md-custom-color-success);      /* Network RX */
  --md-chart-4: var(--md-custom-color-info);         /* Network TX */
  --md-chart-5: var(--md-custom-color-warning);      /* Disk usage */
  --md-chart-6: var(--md-sys-color-error);           /* Temperature */
  --md-chart-7: var(--md-sys-color-secondary);       /* Load average */
  --md-chart-8: #E07B53;                             /* Extended (Docker CPU) */
}
```

When series exceed available tokens, generate intermediate tones via `color-mix()`:
```css
--md-chart-extended: color-mix(in srgb, var(--md-sys-color-primary) 60%, var(--md-sys-color-tertiary));
```

---

## 2. Typography

M3 type scale using **Inter** (sans-serif) and **JetBrains Mono** (monospace).

> **Adaptation:** Display and Headline sizes scaled down ~30% from M3 defaults for dashboard information density. Body and Label sizes kept at or near M3 defaults for readability. Font weights follow M3 specification (Display/Headline: 400, Title/Label: 500, Body: 400).

### 2.1 Type Scale

```css
:root {
  /* Display — hero metric callouts, large numbers */
  --md-sys-typescale-display-large:  400 2.5rem/3rem var(--font-inter);      /* 40px (M3: 57px) */
  --md-sys-typescale-display-medium: 400 2rem/2.5rem var(--font-inter);      /* 32px (M3: 45px) */
  --md-sys-typescale-display-small:  400 1.75rem/2.25rem var(--font-inter);  /* 28px (M3: 36px) */

  /* Headline — page titles, section headers */
  --md-sys-typescale-headline-large:  400 1.5rem/2rem var(--font-inter);     /* 24px (M3: 32px) */
  --md-sys-typescale-headline-medium: 400 1.375rem/1.75rem var(--font-inter);/* 22px (M3: 28px) */
  --md-sys-typescale-headline-small:  400 1.25rem/1.625rem var(--font-inter);/* 20px (M3: 24px) */

  /* Title — card titles, nav items */
  --md-sys-typescale-title-large:  500 1.125rem/1.5rem var(--font-inter);    /* 18px (M3: 22px) */
  --md-sys-typescale-title-medium: 500 1rem/1.375rem var(--font-inter);      /* 16px (M3: 16px) */
  --md-sys-typescale-title-small:  500 0.875rem/1.25rem var(--font-inter);   /* 14px (M3: 14px) */

  /* Body — paragraphs, descriptions, table cells */
  --md-sys-typescale-body-large:  400 1rem/1.5rem var(--font-inter);         /* 16px (M3: 16px) */
  --md-sys-typescale-body-medium: 400 0.875rem/1.25rem var(--font-inter);    /* 14px (M3: 14px) */
  --md-sys-typescale-body-small:  400 0.75rem/1rem var(--font-inter);        /* 12px (M3: 12px) */

  /* Label — buttons, badges, metadata, table headers */
  --md-sys-typescale-label-large:  500 0.875rem/1.25rem var(--font-inter);   /* 14px (M3: 14px) */
  --md-sys-typescale-label-medium: 500 0.75rem/1rem var(--font-inter);       /* 12px (M3: 12px) */
  --md-sys-typescale-label-small:  500 0.6875rem/1rem var(--font-inter);     /* 11px (M3: 11px) */
}
```

### 2.2 Usage Mapping

| Context | Type scale | Example |
|---|---|---|
| Hero metric (uptime %) | `display-medium` | "99.97%" |
| Page title (`<h1>`) | `headline-small` | "Dashboard", "Host Details" |
| Section header | `title-large` | "CPU Usage Over Time" |
| Card title | `title-medium` | "CPU Usage", "Port Status" |
| Nav item | `title-small` | "Dashboard", "Hosts" |
| Table header | `label-large` | "System", "CPU", "Memory" |
| Table body | `body-medium` | metric values, timestamps |
| Metric value (mono) | `body-medium` + `--font-mono` | "78.5%", "192.168.1.10" |
| Chart axis label | `label-small` | "10:30", "50%" |
| Badge / chip | `label-small` | "Online", "3 ports" |
| Tooltip | `body-small` | chart hover detail |

---

## 3. Shape

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

### Component Shape Mapping

| Component | Shape | M3 Default | Deviation |
|---|---|---|---|
| Button (all types) | `corner-full` | `corner-full` | — |
| Card | `corner-large` (16px) | `corner-medium` (12px) | ⬆ visual grouping |
| Text input | `corner-small` (8px) | `corner-extra-small` (4px) | ⬆ consistency |
| Chip / Badge | `corner-small` | `corner-small` | — |
| Dialog / Modal | `corner-extra-large` | `corner-extra-large` | — |
| Tooltip | `corner-extra-small` | `corner-extra-small` | — |
| Navbar | `corner-none` | `corner-none` | — |
| Toggle switch track | `corner-full` | `corner-full` | — |
| Dropdown menu | `corner-extra-small` | `corner-extra-small` | — |

---

## 4. Elevation & Surfaces

M3 replaces `box-shadow` with **tonal surface colors**. Higher elevation = lighter surface-container.

| M3 Level | Token | Use case |
|---|---|---|
| Level 0 | `surface` | Page background |
| Level 1 | `surface-container-low` | Cards |
| Level 2 | `surface-container` | Navbar, modal backdrop |
| Level 3 | `surface-container-high` | Card hover, dropdowns, dialog |
| Level 4 | `surface-container-highest` | Tooltips, popovers |

Additional surface tokens:
- `surface-dim` — subdued/disabled area backgrounds
- `surface-bright` — elevated bright surfaces (useful in dark mode)
- `surface-tint` — equals primary; not used directly when using container hierarchy

### Migration Example

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
  border-radius: var(--md-sys-shape-corner-large);
}
```

---

## 5. State Layers

Interactive elements use **state layers** — semi-transparent overlays of the element's `on-*` color.

| State | Opacity | CSS Pattern |
|---|---|---|
| Hover | 8% | `color-mix(in srgb, var(--on-color) 8%, transparent)` |
| Focus | 12% | `color-mix(in srgb, var(--on-color) 12%, transparent)` |
| Pressed | 12% | `color-mix(in srgb, var(--on-color) 12%, transparent)` |
| Dragged | 16% | `color-mix(in srgb, var(--on-color) 16%, transparent)` |
| Disabled | — | Container 12% opacity, content 38% opacity |

The `--on-color` corresponds to the content color on the element's surface. For `surface`-colored containers, use `on-surface`. For `primary`-colored containers, use `on-primary`.

```css
/* Surface container hover (cards, table rows) */
.card:hover {
  background: color-mix(in srgb, var(--md-sys-color-on-surface) 8%, transparent);
}

/* Filled button hover — M3 elevates instead of tinting colored surfaces */
.btn-filled:hover {
  box-shadow: 0 1px 3px 1px color-mix(in srgb, var(--md-sys-color-shadow) 15%, transparent),
              0 1px 2px color-mix(in srgb, var(--md-sys-color-shadow) 30%, transparent);
}

/* Disabled state */
.btn-filled:disabled {
  background: color-mix(in srgb, var(--md-sys-color-on-surface) 12%, transparent);
  color: color-mix(in srgb, var(--md-sys-color-on-surface) 38%, transparent);
  cursor: not-allowed;
}
```

### Focus Visible

All interactive elements must show a visible focus ring for keyboard navigation:

```css
:focus-visible {
  outline: 2px solid var(--md-sys-color-primary);
  outline-offset: 2px;
}
```

---

## 6. Motion

### 6.1 Duration

```css
:root {
  --md-sys-motion-duration-short1: 50ms;    /* micro-interactions */
  --md-sys-motion-duration-short2: 100ms;   /* tooltip show, ripple */
  --md-sys-motion-duration-short3: 150ms;   /* small state changes */
  --md-sys-motion-duration-short4: 200ms;   /* icon transitions */
  --md-sys-motion-duration-medium1: 250ms;  /* card hover, button press */
  --md-sys-motion-duration-medium2: 300ms;  /* dropdown open */
  --md-sys-motion-duration-medium3: 350ms;  /* page transitions */
  --md-sys-motion-duration-medium4: 400ms;  /* modal open, theme switch */
  --md-sys-motion-duration-long1: 450ms;    /* complex animations */
  --md-sys-motion-duration-long2: 500ms;    /* large surface transforms */
}
```

### 6.2 Easing

```css
:root {
  /* Standard — most UI transitions */
  --md-sys-motion-easing-standard: cubic-bezier(0.2, 0, 0, 1);
  --md-sys-motion-easing-standard-decelerate: cubic-bezier(0, 0, 0, 1);
  --md-sys-motion-easing-standard-accelerate: cubic-bezier(0.3, 0, 1, 1);

  /* Emphasized — prominent/expressive transitions */
  --md-sys-motion-easing-emphasized: cubic-bezier(0.2, 0, 0, 1);
  --md-sys-motion-easing-emphasized-decelerate: cubic-bezier(0.05, 0.7, 0.1, 1);
  --md-sys-motion-easing-emphasized-accelerate: cubic-bezier(0.3, 0, 0.8, 0.15);
}
```

### 6.3 Usage

| Interaction | Duration | Easing |
|---|---|---|
| Tooltip show/hide | `short2` | `standard` |
| Button state change | `short3` | `standard` |
| Card hover elevation | `medium1` | `standard` |
| Dropdown open | `medium2` | `emphasized-decelerate` |
| Dropdown close | `short4` | `emphasized-accelerate` |
| Theme switch | `medium4` | `emphasized` |
| Modal open | `medium4` | `emphasized-decelerate` |
| Modal close | `medium2` | `emphasized-accelerate` |
| Page transition | `medium3` | `emphasized` |

---

## 7. Spacing & Layout

### 7.1 Spacing Scale (4px grid)

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

### 7.2 Page Layout

```
┌─────────────────────────────────────┐
│  Navbar (64px, surface-container)   │
├─────────────────────────────────────┤
│                                     │
│  Content (max-width: 1280px)        │
│  padding: var(--md-sys-spacing-xl)  │
│                                     │
│  ┌─────────┐ ┌─────────┐           │
│  │  Card    │ │  Card    │          │
│  │  (16px   │ │  (16px   │          │
│  │  radius) │ │  radius) │          │
│  └─────────┘ └─────────┘           │
│                                     │
└─────────────────────────────────────┘
```

### 7.3 Responsive Breakpoints

Based on M3 canonical breakpoints, consolidated for dashboard use:

| Name | Width | Layout | Columns |
|---|---|---|---|
| Compact | < 600px | Single column, stacked cards | 1 |
| Medium | 600–839px | 2-column card grid | 2 |
| Expanded | 840–1199px | Multi-column grid | 3 |
| Large | ≥ 1200px | Full grid, wider charts | 4 |

---

## 8. Component Patterns

### 8.1 Buttons

```
Filled    → primary action (Save, Create, Confirm)
Tonal     → secondary action (Cancel, Filter, Reset)
Outlined  → tertiary (Settings, Export)
Text      → low-emphasis (Back, Learn more, Skip)
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
  transition: box-shadow var(--md-sys-motion-duration-short3)
              var(--md-sys-motion-easing-standard);
}
.btn-filled:hover {
  box-shadow: 0 1px 3px 1px color-mix(in srgb, var(--md-sys-color-shadow) 15%, transparent),
              0 1px 2px color-mix(in srgb, var(--md-sys-color-shadow) 30%, transparent);
}

.btn-tonal {
  background: var(--md-sys-color-secondary-container);
  color: var(--md-sys-color-on-secondary-container);
  border: none;
  border-radius: var(--md-sys-shape-corner-full);
  padding: 10px 24px;
  font: var(--md-sys-typescale-label-large);
}
.btn-tonal:hover {
  background: color-mix(
    in srgb,
    var(--md-sys-color-on-secondary-container) 8%,
    var(--md-sys-color-secondary-container)
  );
}

.btn-outlined {
  background: transparent;
  color: var(--md-sys-color-primary);
  border: 1px solid var(--md-sys-color-outline);
  border-radius: var(--md-sys-shape-corner-full);
  padding: 10px 24px;
  font: var(--md-sys-typescale-label-large);
}
.btn-outlined:hover {
  background: color-mix(in srgb, var(--md-sys-color-primary) 8%, transparent);
}

.btn-icon {
  background: transparent;
  color: var(--md-sys-color-on-surface-variant);
  border: none;
  border-radius: var(--md-sys-shape-corner-full);
  padding: 8px;
  width: 40px;
  height: 40px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.btn-icon:hover {
  background: color-mix(in srgb, var(--md-sys-color-on-surface-variant) 8%, transparent);
}
```

### 8.2 Cards (Metric Cards)

```css
.metric-card {
  background: var(--md-sys-color-surface-container-low);
  border: 1px solid var(--md-sys-color-outline-variant);
  border-radius: var(--md-sys-shape-corner-large);
  padding: 20px;
  transition: background var(--md-sys-motion-duration-medium1)
              var(--md-sys-motion-easing-standard);
}
.metric-card:hover {
  background: var(--md-sys-color-surface-container);
}
```

### 8.3 Status Badges

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

### 8.4 Navigation Bar

M3 top app bar (small):

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

### 8.5 Data Table

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
  background: color-mix(in srgb, var(--md-sys-color-on-surface) 8%, transparent);
}
```

### 8.6 Text Input (Outlined)

```css
.text-field {
  background: transparent;
  border: 1px solid var(--md-sys-color-outline);
  border-radius: var(--md-sys-shape-corner-small);
  padding: 12px 16px;
  font: var(--md-sys-typescale-body-large);
  color: var(--md-sys-color-on-surface);
  caret-color: var(--md-sys-color-primary);
  transition: border-color var(--md-sys-motion-duration-short3)
              var(--md-sys-motion-easing-standard);
}
.text-field:hover:not(:focus) {
  border-color: var(--md-sys-color-on-surface);
}
.text-field:focus {
  border-color: var(--md-sys-color-primary);
  border-width: 2px;
  padding: 11px 15px; /* compensate for border width */
  outline: none;
}
.text-field-label {
  font: var(--md-sys-typescale-body-small);
  color: var(--md-sys-color-on-surface-variant);
}
.text-field-error {
  border-color: var(--md-sys-color-error);
  caret-color: var(--md-sys-color-error);
}
.text-field-error-text {
  font: var(--md-sys-typescale-body-small);
  color: var(--md-sys-color-error);
}
```

### 8.7 Toggle Switch

```css
.switch {
  width: 52px;
  height: 32px;
  background: var(--md-sys-color-surface-container-highest);
  border: 2px solid var(--md-sys-color-outline);
  border-radius: var(--md-sys-shape-corner-full);
  position: relative;
  cursor: pointer;
  transition: background var(--md-sys-motion-duration-medium1) var(--md-sys-motion-easing-standard),
              border-color var(--md-sys-motion-duration-medium1) var(--md-sys-motion-easing-standard);
}
.switch[aria-checked="true"] {
  background: var(--md-sys-color-primary);
  border-color: var(--md-sys-color-primary);
}
/* Handle */
.switch::after {
  content: "";
  position: absolute;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: var(--md-sys-color-on-surface-variant);
  top: 6px;
  left: 6px;
  transition: transform var(--md-sys-motion-duration-medium1) var(--md-sys-motion-easing-standard),
              width var(--md-sys-motion-duration-medium1) var(--md-sys-motion-easing-standard),
              height var(--md-sys-motion-duration-medium1) var(--md-sys-motion-easing-standard),
              background var(--md-sys-motion-duration-medium1) var(--md-sys-motion-easing-standard);
}
.switch[aria-checked="true"]::after {
  transform: translateX(20px);
  width: 24px;
  height: 24px;
  top: 2px;
  background: var(--md-sys-color-on-primary);
}
```

### 8.8 Dialog / Modal

```css
.dialog-scrim {
  background: color-mix(in srgb, var(--md-sys-color-scrim) 32%, transparent);
  position: fixed;
  inset: 0;
}
.dialog {
  background: var(--md-sys-color-surface-container-high);
  border-radius: var(--md-sys-shape-corner-extra-large);
  padding: 24px;
  min-width: 280px;
  max-width: 560px;
}
.dialog-title {
  font: var(--md-sys-typescale-headline-small);
  color: var(--md-sys-color-on-surface);
  margin-bottom: var(--md-sys-spacing-lg);
}
.dialog-body {
  font: var(--md-sys-typescale-body-medium);
  color: var(--md-sys-color-on-surface-variant);
}
.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--md-sys-spacing-sm);
  margin-top: var(--md-sys-spacing-xl);
}
```

### 8.9 Snackbar / Toast

Styling for `sonner` library — uses M3 inverse surface pattern:

```css
.toast {
  background: var(--md-sys-color-inverse-surface);
  color: var(--md-sys-color-inverse-on-surface);
  border-radius: var(--md-sys-shape-corner-extra-small);
  padding: 14px 16px;
  font: var(--md-sys-typescale-body-medium);
}
.toast-action {
  color: var(--md-sys-color-inverse-primary);
  font: var(--md-sys-typescale-label-large);
}
```

### 8.10 Progress Indicator

For loading states in metric fetching and SSE connection:

```css
/* Linear (indeterminate) */
.progress-linear {
  height: 4px;
  background: var(--md-sys-color-surface-container-highest);
  border-radius: var(--md-sys-shape-corner-full);
  overflow: hidden;
}
.progress-linear::after {
  content: "";
  display: block;
  height: 100%;
  background: var(--md-sys-color-primary);
  border-radius: var(--md-sys-shape-corner-full);
  animation: indeterminate 1.5s var(--md-sys-motion-easing-emphasized) infinite;
}

/* Circular (card-level loading) */
.progress-circular {
  width: 48px;
  height: 48px;
  border: 4px solid var(--md-sys-color-surface-container-highest);
  border-top-color: var(--md-sys-color-primary);
  border-radius: 50%;
  animation: spin 1s linear infinite;
}
```

---

## 9. Data Visualization

### 9.1 Chart Styling

```css
.chart-container {
  background: var(--md-sys-color-surface-container-low);
  border: 1px solid var(--md-sys-color-outline-variant);
  border-radius: var(--md-sys-shape-corner-large);
  padding: var(--md-sys-spacing-lg);
}

/* Recharts overrides */
.recharts-cartesian-axis-tick-value {
  font: var(--md-sys-typescale-label-small);
  fill: var(--md-sys-color-on-surface-variant);
}
.recharts-tooltip-wrapper .recharts-default-tooltip {
  background: var(--md-sys-color-surface-container-highest) !important;
  border: 1px solid var(--md-sys-color-outline-variant) !important;
  border-radius: var(--md-sys-shape-corner-extra-small) !important;
  font: var(--md-sys-typescale-body-small);
}
```

### 9.2 Threshold Lines

```css
.threshold-line {
  stroke: var(--md-sys-color-error);
  stroke-dasharray: 6 4;
  stroke-width: 1.5;
}
.threshold-label {
  fill: var(--md-sys-color-error);
  font: var(--md-sys-typescale-label-small);
}
```

---

## 10. Accessibility

### Contrast Requirements (WCAG 2.1 AA)

| Element | Minimum Ratio |
|---|---|
| Body text (`on-surface` / `surface`) | 4.5:1 |
| Label text (`on-surface-variant` / `surface`) | 4.5:1 |
| Large text (≥ 18px regular or ≥ 14px bold), UI components | 3:1 |
| Status badge text (e.g. `success` on `success@12%`) | 3:1 |
| Disabled text | exempt |

### Interactive Requirements

- All interactive elements: `:focus-visible` ring (2px `primary`, 2px offset)
- Touch targets: minimum 48×48px on compact, 40×40px on expanded
- Icon buttons: require `aria-label`
- Toggle switches: `role="switch"` + `aria-checked`
- Status changes: announced via `aria-live="polite"` regions

---

## 11. Migration Checklist

### Phase 1: Tokens (non-breaking)
- [ ] Add all `--md-sys-*` and `--md-custom-*` tokens to `globals.css`
- [ ] Include inverse, scrim, shadow, surface-dim/bright/tint tokens
- [ ] Add custom color container tokens (success/warning/info containers)
- [ ] Create backward-compat aliases: `--bg-primary: var(--md-sys-color-surface)` etc.
- [ ] Verify both light and dark themes render correctly

### Phase 2: Shape & Typography
- [ ] Replace `--radius-sm/md/lg` with `--md-sys-shape-corner-*`
- [ ] Apply type scale (verify Display/Headline weight is 400, not 600)
- [ ] Update card `border-radius` to `corner-large` (16px)
- [ ] Add all missing type scale levels (display-small, headline-large/medium)

### Phase 3: Components
- [ ] Migrate buttons to M3 variants (filled, tonal, outlined, icon)
- [ ] Replace glass-card with M3 surface-container elevation
- [ ] Add toggle switch with `role="switch"`, `aria-checked`, correct handle colors
- [ ] Update data table hover to 8% state layer (not 4%)
- [ ] Update text inputs: add hover state (`on-surface` border), error state
- [ ] Style dialogs with `scrim` overlay and `corner-extra-large`
- [ ] Map sonner toasts to `inverse-surface` colors
- [ ] Add progress indicators for loading states

### Phase 4: Motion & Polish
- [ ] Add M3 duration tokens (all short/medium/long)
- [ ] Add M3 easing tokens (including `standard-decelerate/accelerate`)
- [ ] Apply motion tokens to all component transitions
- [ ] Implement state layers consistently (hover 8%, focus 12%, pressed 12%)
- [ ] Verify `:focus-visible` rings on all interactive elements
- [ ] Ensure touch targets meet minimum size requirements

### Rules

All rules are consolidated at the top of this document under **⚠️ Golden Rules**. The checklist above assumes those rules are already satisfied — do not mark a phase complete if any Golden Rule is violated.