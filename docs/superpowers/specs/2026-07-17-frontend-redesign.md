# Frontend Redesign — DESIGN.md

> **Status:** Approved  
> **Design direction:** Editorial Codex  
> **Last updated:** 2026-07-17

## 1. Design Philosophy

Hieronymus is a **memory workbench for literary translators**, not a DevOps config panel nor a settings dashboard. Every design decision flows from this: translators spend hours reviewing, curating, and refining memory records. Configuration is set once and rarely touched. The UI must feel like a well-made journal — calm, focused, typographically rich — while remaining efficient for power users.

### Core principles

| Principle | What it means |
|---|---|
| Memory-first, config-second | Memory Views is the hero workspace. Config pages are supplementary. |
| Data deserves typography | Translated text is literary content. It gets the best font treatment. |
| Calm over clever | Subtle motion, generous whitespace, no gratuitous effects. |
| Local and private | The "All data is local" footer stays. The interface should feel self-contained, not cloud-connected. |
| Code-adjacent without cosplaying | The old monospace-everywhere approach is gone. Inconsolata LGC appears only where it adds meaning: IDs, keys, counts, config values. |

### User personas

1. **Working translator (primary)** — Opens the UI daily to check memory health, approve/reject proposals, browse concept drift. Needs the Memory Views workspace to be fast, scannable, and typographically comfortable for reading source/target text.
2. **Self-hosting translator (secondary)** — Sets up providers, configures dreaming schedules, adjusts ingest thresholds. Visits config pages once per project. Needs clear, unhurried forms with good defaults.

---

## 2. Typography System

### Font stack

| Voice | Font | Source | Role |
|---|---|---|---|
| Serif | **Literata** (Display weight for headings, Regular for body) | Google Fonts | Page titles, literary content (quoted source/target text), display numbers |
| Sans | **Geist** (Variable, 400/500/600) | Google Fonts | UI chrome: nav labels, form labels, descriptions, help text, buttons, captions |
| Mono | **Inconsolata LGC** (Regular) | [WOFF2 release](https://github.com/MihailJP/Inconsolata-LGC/releases/download/v3.200/InconsolataLGC-WOFF2-3.200.tar.xz) | Data: record IDs, confidence scores, config values, stats numbers, provider keys |

### Typographic scale

| Token | Font | Size / Weight | Line-height | Use |
|---|---|---|---|---|
| `--text-display` | Literata | 28-36px / Display | 1.15 | Major page titles in the editorial lead pane |
| `--text-h2` | Literata | 20-24px / Regular | 1.25 | Section headings |
| `--text-h3` | Geist | 15px / 600 | 1.3 | Card titles, subsection labels |
| `--text-body` | Geist | 14px / 400 | 1.55 | Descriptions, form labels, help text |
| `--text-body-sm` | Geist | 13px / 400 | 1.5 | Compact body (workflow headers, detail meta) |
| `--text-mono` | Inconsolata LGC | 13px / 400 | 1.5 | IDs, keys, counts, config values, scores |
| `--text-eyebrow` | Geist | 10px / 500 | 1.0 | Uppercase section tags, `letter-spacing: 0.12em` |
| `--text-caption` | Geist | 11px / 400 | 1.4 | Timestamps, metadata, footer |

**Font delivery:** All three fonts are bundled as static assets (WOFF2) and declared in `@font-face`. No runtime Google Fonts requests — this is a local-first tool, fonts should be local too.

### Content-type typography rules

| Content type | Font | Reasoning |
|---|---|---|
| Source/target text in memory detail | Literata, 15px, line-height 1.7 | This is literary content. It deserves the best reading experience. |
| Record labels, concept names | Geist | These are UI-generated descriptors, not the content itself. |
| IDs, keys, model names, URLs | Inconsolata LGC | Machine identifiers benefit from monospace alignment and distinction. |
| Stats numbers (dashboard) | Inconsolata LGC | Numbers align visually, scannable at a glance. |
| Form labels, descriptions | Geist | Labels are UI chrome. |
| Navigation, buttons, tabs | Geist | UI chrome. |
| Eyebrows, status badges | Geist, uppercase, tracked | Small UI annotations. |

---

## 3. Color System

### Dark theme — Warm Amber

Inspiration: manuscript on a desk under warm lamplight. Deep brown-blacks with amber/gold accent.

| CSS custom property | Hex | Role |
|---|---|---|
| `--color-bg-root` | `#0E0D0A` | Page background |
| `--color-bg-surface` | `#141310` | Cards, panels, sidebar background |
| `--color-bg-raised` | `#1A1814` | Inputs, selected rows, hover states |
| `--color-bg-overlay` | `#0E0D0A` / 0.85 | Modal/slide-out backdrop |
| `--color-border-default` | `#252218` | Card borders, table separators, sidebar edge |
| `--color-border-strong` | `#3A3628` | Input borders, focus rings, button borders |
| `--color-border-accent` | `#D4A84B` | Active tab underline, selected row accent bar, primary button border |
| `--color-text-primary` | `#E8DEC4` | Headings, body text |
| `--color-text-secondary` | `#9E9678` | Labels, muted descriptions, placeholder hierarchy |
| `--color-text-tertiary` | `#6B6450` | Placeholder text, disabled text, footnotes |
| `--color-accent` | `#D4A84B` | Borders, underlines, status dots, decorative (4.5:1 against bg-root) |
| `--color-accent-text` | `#D4A84B` | Text on buttons, active nav, eyebrows, tabs (same value; passes AA at 5.2:1 on bg-root) |
| `--color-accent-soft` | `#F0D78C` | Hover brightening, subtle highlights |
| `--color-accent-bg` | `#D4A84B` / 0.10 | Primary button hover fill, selected row tint |
| `--color-danger` | `#C84040` | Destructive action text, error borders, danger button |
| `--color-danger-bg` | `#C84040` / 0.10 | Danger hover, danger confirmation backgrounds |
| `--color-success` | `#5EA870` | Success text, confirmation borders |
| `--color-success-bg` | `#5EA870` / 0.10 | Toast success background |

### Light theme — Paper

Inspiration: cream paper, ink annotations. Warm off-white with deeper amber accent for contrast.

| CSS custom property | Hex | Role |
|---|---|---|
| `--color-bg-root` | `#FDFAF2` | Page background |
| `--color-bg-surface` | `#F5F0E5` | Cards, panels, sidebar background |
| `--color-bg-raised` | `#EDE7D8` | Inputs, selected rows, hover states |
| `--color-bg-overlay` | `#FDFAF2` / 0.90 | Modal/slide-out backdrop |
| `--color-border-default` | `#E0D9C5` | Card borders, table separators |
| `--color-border-strong` | `#C4B998` | Input borders, focus rings |
| `--color-border-accent` | `#B88820` | Active tab, selected row accent |
| `--color-text-primary` | `#2D2618` | Headings, body text |
| `--color-text-secondary` | `#6B6040` | Labels, descriptions |
| `--color-text-tertiary` | `#9E9470` | Placeholders, footnotes |
| `--color-accent` | `#B88820` | Borders, underlines, status dots (3.06:1 on bg-root, decorative only) |
| `--color-accent-text` | `#8B6914` | Text on buttons, active nav, eyebrows, tabs (4.55:1 on bg-root, passes AA) |
| `--color-accent-soft` | `#D4A84B` | Hover / highlight |
| `--color-accent-bg` | `#B88820` / 0.08 | Primary button hover fill |
| `--color-danger` | `#B83030` | Slightly darker for paper contrast |
| `--color-danger-bg` | `#B83030` / 0.06 | Danger hover |
| `--color-success` | `#4A8E5A` | Success text |
| `--color-success-bg` | `#4A8E5A` / 0.08 | Success backgrounds |

### Theme switching

- Stored in `localStorage` key `hiero-theme` with values `"dark"` | `"light"`.
- Default follows `prefers-color-scheme` on first visit; explicit toggle overrides and persists.
- CSS custom properties are set on `:root`; no class-based theme switching — a single `data-theme` attribute on `<html>` drives all tokens.
- Transition: `color` and `background-color` change with `transition: 200ms ease` to avoid a harsh flash on toggle.
- The toggle control lives in the sidebar footer as a sun/moon icon pair with a subtle press animation.

---

## 4. Spatial System

### Spacing scale

| Token | Value | Use |
|---|---|---|
| `--space-xs` | 4px | Tight inner gaps (icon-to-label, badge padding) |
| `--space-sm` | 8px | Standard gap between related items (button groups, form row gaps) |
| `--space-md` | 12px | Card internal padding, tab gaps |
| `--space-lg` | 16px | Standard padding (cards, cells), input padding |
| `--space-xl` | 24px | Section internal spacing, detail panel padding |
| `--space-2xl` | 32px | Sidebar padding, section-to-section gap |
| `--space-3xl` | 48px | Content area top/bottom padding, lead-to-stage gap |
| `--space-4xl` | 64px | Major section separation |
| `--space-5xl` | 96px | Hero/page-level breathing room (overview page) |

### Border radius

| Token | Value | Use |
|---|---|---|
| `--radius-none` | 0 | Tables, inline tabs |
| `--radius-sm` | 2px | Inputs, buttons, form controls |
| `--radius-md` | 4px | Cards, panels, the sidebar |
| `--radius-pill` | 9999px | Eyebrow badges, status dots |

**No rounded pill buttons or cards.** Editorial codex uses squared shapes — like a page, like a manuscript. Badges and dots are the exception.

---

## 5. Layout Architecture

### Shell (global)

```
<html data-theme="dark|light">
  <main>                    ← CSS grid, full viewport height
    ├── .sidebar            ← 200px fixed, no right border
    │   ├── Title ("Hieronymus")     ← Literata Display, 18px
    │   ├── Context label            ← Geist 11px, text-secondary
    │   ├── Nav links                ← Geist 13px
    │   └── Footer + theme toggle    ← Geist 11px, text-tertiary
    │
    └── .workspace           ← flex: 1, padding: 64px
        ├── (page content)
        └── (slide-out overlay, when active)
  </main>
```

### Editorial Split pattern (Memory Views, Overview)

```
.workspace
  └── .split                  ← CSS grid, 2 columns
      ├── .lead               ← 280-360px, position: sticky top
      │   ├── Eyebrow badge   ← 10px pill, accent bg, uppercase
      │   ├── Page title      ← Literata Display, 28-36px
      │   ├── Description     ← Geist 14px, text-secondary
      │   └── Meta bar        ← record count, status indicators
      │
      └── .stage              ← flex: 1 (scrollable)
          ├── View tabs       ← inline underlines, accent on active
          ├── Data area       ← table, card grid, or detail panel
          └── Detail panel    ← when record selected (Memory Views)
```

**Rules:**
- The lead pane announces "what this workspace is." It stays visible while the stage scrolls.
- Config pages (Providers, Dreaming, Ingest, Release) skip the split — they use full-workspace width, single-column, max-width 720px.
- Lead content is hand-written per page, not generated from route names.

### Per-page lead content

| Page | Eyebrow | Title | Description |
|---|---|---|---|
| `/admin` | Project · version | Hieronymus | Local translation memory for [book]. Memory statistics and service status. |
| `/admin/memory` | Memory administration | Memory views | Find a record, read its context, then curate only the memory that needs attention. |
| `/config` | Configuration | Providers | Manage LLM provider profiles. Create profiles for OpenAI, Anthropic, Google, or local Ollama endpoints. |
| `/config/dreaming` | Automation | Dreaming | Schedule automated memory crystallization and assign workflows to provider models. |
| `/config/ingest` | Quality control | Ingest | Set thresholds for incoming memory quality: sentence counts, symbol limits, block sizes. |
| `/config/release` | Updates | Release | Choose between stable and development update channels. |

### Sidebar behavior

- **Nav links:** Geist 13px, text-secondary. Active state: accent-text color + a 2px vertical accent bar on the left edge (via `border-left` or pseudo-element). No background fill on hover — just a text color shift to text-primary.
- **Footer:** `"All data is local. / No cloud. No tracking."` — Geist 11px, text-tertiary, pushed to bottom via `margin-top: auto`.
- **Theme toggle:** Icon button in the footer area. Sun icon in dark mode, moon icon in light mode. 28px hit target. Press animation: scale 0.95 on `:active`, 150ms spring-back.

### Responsive breakpoints

| Breakpoint | Behavior |
|---|---|
| > 1080px | Full editorial split: 300px lead + flexible stage |
| 960-1080px | Lead narrows to 240px. Stage adjusts. |
| 720-960px | Lead collapses above stage (vertical stack). Sidebar stays. |
| < 720px | Sidebar collapses to a top bar with hamburger drawer. All full-width. |

**No fixed `min-width: 760px`.** The new layout adapts down to 360px viewports.

---

## 6. Component Patterns

### 6.1 Cards (stats, status, workflow)

**Nested shell structure:**
```
.card
  ├── outer wrapper (bg-surface, border-default, radius-md, padding: 1px)
  └── inner body (bg-root, radius-sm, padding: 16px)
```

The 1px padding on the outer wrapper creates a subtle framed effect — the background shows through as a hairline rim. Cards sit on the page background, not on a different surface.

**Stats cards (Overview dashboard):**
- Grid: `repeat(auto-fill, minmax(160px, 1fr))`
- Number: Inconsolata LGC 24px, accent-text color
- Label: Geist 11px, text-secondary, uppercase
- No icon, no decoration — just the number and label

**Workflow cards (Dreaming settings):**
- Header: status dot (8px, accent=active, border-default=inactive) + title (Geist 15px/600)
- Body: 2-column form grid for provider + model selects
- Toggle: in header, right-aligned

### 6.2 Tables

**Header row:**
- Geist 10px, uppercase, `letter-spacing: 0.08em`, accent-text color
- 1px bottom border (border-default)

**Data rows:**
- Geist 14px for labels, Inconsolata 13px for codes/IDs
- Vertical padding: 12px (top + bottom)
- Horizontal padding: 14px
- Zebra: every even row gets `bg-surface` (very subtle, just enough to guide the eye)

**Interactive rows (Memory Views table):**
- Hover: background shifts to `bg-raised`, cursor becomes pointer
- Selected: accent 2px left bar on the row, label text bumps to text-primary
- Click on a row loads its detail in the adjacent panel

**Empty state:**
- Rendered inside the table body as a single cell spanning all columns
- Centered text: "No [view name] yet"
- No separate empty wrapper div

### 6.3 Forms

**Layout:**
- Labels above inputs (current pattern kept)
- Label: Geist 12px, text-secondary, `margin-bottom: 4px`
- Input: `bg-raised`, `border-strong`, `radius-sm`, 14px Geist, `padding: 10px 12px`
- Focus: border shifts to accent color. No box-shadow glow.
- Selects: same styling as inputs + custom chevron (CSS-only, no icon font)
- Textareas: same styling, `resize: vertical`, min-height: 80px

**Form grid:**
- `grid-template-columns: repeat(2, 1fr)`, gap: 16px
- Full-width fields (textarea, prompt) span both columns
- Max-width: 720px on the overall form — prevents absurdly wide single inputs on large screens

**Toggles (checkbox as switch):**
- Track: 32×18px, `bg-raised`, `border-strong`, `radius-pill`
- Thumb: 14px circle, `bg-text-secondary`, positioned with `transform`
- Checked: track border → accent, thumb → accent-text, thumb translates right
- The associated `<label>` wraps both the toggle and label text — the entire label area is clickable, not just the visual track. Minimum 44px touch target achieved via label padding.

**Fieldsets (current pattern kept for Dreaming thresholds):**
- Border: `border-strong`, padding: 16px
- Legend: accent-text color, Geist 12px

### 6.4 Buttons

| Variant | Background | Border | Text | Hover |
|---|---|---|---|---|
| Primary | `bg-raised` | `accent` | `accent` | bg → `accent-bg`, border → `accent-soft` |
| Secondary | `bg-surface` | `border-default` | `text-primary` | bg → `bg-raised` |
| Danger | `bg-raised` | `danger` | `danger` | bg → `danger-bg` |
| Icon-only | transparent | none | `text-secondary` | text → `text-primary` |

- Height: 34px (single-line), `padding: 0 16px`
- Border-radius: `radius-sm` (2px) — squared editorial style
- Font: Geist 13px / 500
- Active/press: `scale(0.98)` via `transform`, 100ms transition
- Disabled: `opacity: 0.4`, cursor default
- Button group spacing: 8px gap

### 6.5 Tabs (memory view selector)

- Current `view-tabs` pattern evolved: no background pills, uses underline indicators
- Tab: Geist 13px, text-secondary, `padding: 8px 0`, `margin-right: 20px`
- Active: accent-text color, 2px accent underline (bottom border)
- Hover: text-primary (no underline)
- Tabs wrap on small screens — no horizontal scroll

### 6.6 Slide-out editor (Provider CRUD)

**Behavior:**
- Triggered by "New provider" button or clicking a row
- Slides in from right edge: `transform: translateX(100%)` → `translateX(0)`, 200ms `cubic-bezier(0.32, 0.72, 0, 1)`
- Backdrop: `bg-overlay` with `backdrop-blur: 4px` (only on this fixed overlay, never on scrolling content)
- Width: 420px on larger screens, full-width on < 480px
- Close: X button (top-right) or click backdrop

**Internal layout:**
- Header: title (Geist 18px/600) + close button
- Form body: scrollable, same form patterns as above
- Footer bar: model list and danger-zone delete button at the bottom

### 6.7 Detail panel (Memory Views)

- Lives inside the `.stage` area, not a separate overlay
- When no record selected: empty state prompt ("Select a record...")
- When record selected:
  - Eyebrow: kind · status (Geist 10px, accent-text)
  - Title: Geist 18px/600
  - Subtitle: Geist 13px, text-secondary
  - Body (source/target text): Literata 15px, line-height 1.7, `bg-raised`, padding 16px, accent left border (3px)
  - Metadata: `<dl>` with Geist 12px labels (text-secondary) and Inconsolata 13px values
  - Action buttons: horizontal row, separated by a top border

### 6.8 Toast notifications

- Position: fixed, bottom 24px, right 24px
- Entry: `translateY(16px) + opacity(0)` → `translateY(0) + opacity(1)`, 250ms
- Exit: reverse animation, 200ms, then removed from DOM
- Auto-dismiss: 4 seconds, timer pauses on hover
- Width: 400px max, full-width on < 480px
- Structure: 3px left border (success/danger) + message text (Geist 13px, text-primary; or success-text/danger-text for the tinted tone) + close icon
- Background: success-bg / danger-bg respectively

---

## 7. Motion Language

### Principles

No gratuitous motion. Every animation has a purpose: guiding attention, confirming action, or smoothing a state change.

### Entry animations (staggered reveal)

When a page loads or data arrives, elements enter in sequence:

- **Lead pane content:** slides up 12px + fades in, 400ms, starts immediately
- **Stage content:** same animation, 100ms delay, 400ms duration
- **List rows / cards:** stagger by 30ms per item, 300ms each, fade-up 8px
- **Implementation:** `IntersectionObserver` + CSS `@starting-style` / Svelte transitions. Not scroll-driven — triggered once on mount.

### Transition easing

All transitions use `cubic-bezier(0.32, 0.72, 0, 1)` — a decelerating curve with gentle snap-in. This is the single motion curve for the entire UI:

```css
--ease-out-smooth: cubic-bezier(0.32, 0.72, 0, 1);
```

### Per-component motion

| Component | Trigger | Effect | Duration |
|---|---|---|---|
| Table rows | Hover | Background color shift | 150ms |
| Table rows | Mount / data change | Fade-up 8px + opacity | 300ms, staggered |
| Buttons | Press (`:active`) | `scale(0.98)` | 100ms |
| Tabs | Click | Underline slides to new tab (CSS-only via `::after` width transition) | 200ms |
| Slide-out editor | Open | Slide left + fade backdrop | 200ms |
| Slide-out editor | Close | Slide right + fade backdrop | 150ms |
| Toast | Appear | Slide up + fade in | 250ms |
| Toast | Dismiss | Fade out + slide down | 200ms |
| Theme toggle | Click | Crossfade background + text colors | 200ms |
| Toggle switch | Click | Thumb translate + color shift | 150ms |
| Detail panel | Content swap | Quick crossfade (out 100ms, in 150ms) | 250ms total |
| Backdrop | Open/close | Opacity fade | 200ms |

### What stays still

- Page transitions (routing is instant — no page-level animation)
- Sidebar (always visible, no collapse animation on desktop)
- Form validation errors (appear instantly — no delay on feedback)

---

## 8. Iconography

**No icon library dependency.** The UI uses minimal inline SVGs (or Unicode characters where SVG isn't justified):

| Icon | Implementation |
|---|---|
| Close (×) | Unicode `×` or 16px SVG |
| Sun/Moon (theme toggle) | 16px inline SVGs |
| Chevron (selects) | CSS-only triangle via border trick |
| Status dot (workflow cards) | 8px `<div>` with border-radius |
| External link / new tab | Unicode `↗` |

Icons are colored via `currentColor` and inherit from parent text color.

---

## 9. Accessibility

- All interactive elements have visible focus indicators (2px accent outline, offset 1px)
- Table rows as buttons: `role="button"`, `tabindex="0"`, `Enter`/`Space` keyboard activation
- Form inputs have associated `<label>` elements (current pattern kept)
- Toast notifications use `role="status"` and `aria-live="polite"`
- Slide-out editor traps focus when open (focus cycles inside the panel)
- Color contrast: all text/background pairs meet WCAG AA (4.5:1 for body text)
- Theme toggle announces via `aria-label="Switch to light/dark theme"`
- `prefers-reduced-motion`: all animations disabled, instant transitions only
- Content is zoomable to 200% without layout breakage

---

## 10. Implementation Approach

### Tech decisions

- **Continue Svelte 5 with runes** — no framework migration
- **Continue zero runtime dependencies** — no Tailwind, no component library
- **Adopt CSS custom properties for all design tokens** — single `:root` block in a new `tokens.css`
- **Reorganize CSS:** Replace the single 489-line `styles.css` with:
  - `tokens.css` — all CSS custom properties, theme variations
  - `base.css` — reset, typography, layout shell
  - `components.css` — cards, tables, forms, buttons, tabs, editor, toast
- **Add font files** to `frontend/src/web/fonts/` — Literata (WOFF2 subset), Geist (WOFF2 subset), Inconsolata LGC (WOFF2 from the GitHub release)
- **Add font-face declarations** in `fonts.css`
- **Theme toggle:** A small Svelte module (`theme.svelte.ts`) that reads/writes `localStorage` and sets `data-theme` on `<html>`
- **Animations:** Svelte `transition:` and `fly`/`fade` built-ins with the custom easing curve. No additional animation library.

### Migration strategy

1. **Tokens first** — Create `tokens.css` with all custom properties. Apply to `:root`. The old `styles.css` hardcoded colors remain functional during the transition.
2. **Layout shell** — Restructure `App.svelte` to the editorial split grid. Wire up the sidebar without the hard border.
3. **Typography** — Add fonts, replace the `font-family` stack. This immediately changes the feel even with the old colors.
4. **One component at a time** — Rebuild each section in priority order: Memory Views → Overview → Providers → Dreaming → Ingest → Release.
5. **Theme toggle** — Implement after the dark theme is stable. Add light tokens, then the toggle control.
6. **Remove old CSS** — Delete `styles.css` when all components are migrated.

### Verification

After migration:
```bash
uv run pytest                          # Python backend unchanged
bun run --cwd frontend build           # Clean production build
# Manual browser check: all 6 routes, dark + light, narrow viewport
```

---

## Appendix A: CSS Custom Properties Reference

```css
:root {
  /* Typography */
  --font-serif: 'Literata', Georgia, 'Times New Roman', serif;
  --font-sans: 'Geist', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
  --font-mono: 'Inconsolata LGC', 'Inconsolata', 'Courier New', monospace;

  --text-display: 400 clamp(28px, 4vw, 36px) / 1.15 var(--font-serif);
  --text-h2: 400 clamp(20px, 2.5vw, 24px) / 1.25 var(--font-serif);
  --text-h3: 600 15px / 1.3 var(--font-sans);
  --text-body: 400 14px / 1.55 var(--font-sans);
  --text-body-sm: 400 13px / 1.5 var(--font-sans);
  --text-mono: 400 13px / 1.5 var(--font-mono);
  --text-eyebrow: 500 10px / 1.0 var(--font-sans);
  --text-caption: 400 11px / 1.4 var(--font-sans);

  /* Spacing */
  --space-xs: 4px;
  --space-sm: 8px;
  --space-md: 12px;
  --space-lg: 16px;
  --space-xl: 24px;
  --space-2xl: 32px;
  --space-3xl: 48px;
  --space-4xl: 64px;
  --space-5xl: 96px;

  /* Radius */
  --radius-none: 0;
  --radius-sm: 2px;
  --radius-md: 4px;
  --radius-pill: 9999px;

  /* Motion */
  --ease-smooth: cubic-bezier(0.32, 0.72, 0, 1);
  --duration-fast: 150ms;
  --duration-normal: 200ms;
  --duration-slow: 300ms;
  --duration-entry: 400ms;

  /* Layout */
  --sidebar-width: 200px;
  --lead-width: 300px;
  --editor-width: 420px;
  --content-max: 720px;
}
```

## Appendix B: Dark Theme Custom Properties

```css
[data-theme="dark"] {
  --color-bg-root: #0E0D0A;
  --color-bg-surface: #141310;
  --color-bg-raised: #1A1814;
  --color-bg-overlay: rgba(14, 13, 10, 0.85);
  --color-border-default: #252218;
  --color-border-strong: #3A3628;
  --color-border-accent: #D4A84B;
  --color-text-primary: #E8DEC4;
  --color-text-secondary: #9E9678;
  --color-text-tertiary: #6B6450;
  --color-accent: #D4A84B;
  --color-accent-soft: #F0D78C;
  --color-accent-bg: rgba(212, 168, 75, 0.10);
  --color-danger: #C84040;
  --color-danger-bg: rgba(200, 64, 64, 0.10);
  --color-success: #5EA870;
  --color-success-bg: rgba(94, 168, 112, 0.10);
}
```

## Appendix C: Light Theme Custom Properties

```css
[data-theme="light"] {
  --color-bg-root: #FDFAF2;
  --color-bg-surface: #F5F0E5;
  --color-bg-raised: #EDE7D8;
  --color-bg-overlay: rgba(253, 250, 242, 0.90);
  --color-border-default: #E0D9C5;
  --color-border-strong: #C4B998;
  --color-border-accent: #B88820;
  --color-text-primary: #2D2618;
  --color-text-secondary: #6B6040;
  --color-text-tertiary: #9E9470;
  --color-accent: #B88820;
  --color-accent-text: #8B6914;
  --color-accent-soft: #D4A84B;
  --color-accent-bg: rgba(184, 136, 32, 0.08);
  --color-danger: #B83030;
  --color-danger-text: #B83030;
  --color-danger-bg: rgba(184, 48, 48, 0.06);
  --color-success: #4A8E5A;
  --color-success-text: #3A6E45;
  --color-success-bg: rgba(74, 142, 90, 0.08);
}
```

## Appendix D: Font Delivery

**Literata** (Google Fonts): Download `opsz` axis variable, subset to Latin + Latin Extended. Include Display weight axis for headings.

**Geist** (Google Fonts): Download variable, subset to Latin + Latin Extended. Weights used: 400, 500, 600.

**Inconsolata LGC**: Download from [Inconsolata-LGC WOFF2 release](https://github.com/MihailJP/Inconsolata-LGC/releases/download/v3.200/InconsolataLGC-WOFF2-3.200.tar.xz). The LGC variant includes Latin, Greek, and Cyrillic — important for translation workflows involving Bulgarian, Serbian, Russian source texts.

All fonts are placed in `frontend/src/web/fonts/` and referenced by `@font-face` in `fonts.css`. Vite imports them as assets and hashes them in the build output.
