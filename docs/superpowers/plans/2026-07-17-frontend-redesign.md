# Frontend Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rewrite the Hieronymus web console UI from hand-written monospace CSS to the Editorial Codex design system — Literata/Geist/Inconsolata fonts, Warm Amber dark + Paper light themes, editorial split layout, disciplined motion.

**Architecture:** Modular CSS replaces single 489-line `styles.css`. New files: `tokens.css` (design tokens), `base.css` (reset + typography + shell), `components.css` (card/table/form/button/tab/editor/toast patterns), `fonts.css` (font-face), `theme.svelte.ts` (theme toggle rune). Each component updated one at a time in priority order: Memory Views → Overview → Providers → Dreaming → Ingest → Release.

**Tech Stack:** Svelte 5 (runes), Vite 6, TypeScript, CSS custom properties, zero runtime dependencies.

**Verification gate:** `bun run --cwd frontend build` must succeed after every task. Visual verification on all 6 routes in dark + light modes at desktop and 360px narrow viewport at major checkpoints.

---

### Task 1: Download and install fonts

**Files:**
- Create: `frontend/src/web/fonts/` (directory)
- Create: `frontend/src/web/fonts/literata.woff2`
- Create: `frontend/src/web/fonts/geist.woff2`
- Create: `frontend/src/web/fonts/inconsolatalgc.woff2`

- [ ] **Step 1: Create fonts directory**

```bash
mkdir -p frontend/src/web/fonts
```

- [ ] **Step 2: Download Literata**

Download Literata variable (opsz axis) from Google Fonts, subset to Latin + Latin Extended, WOFF2 format. Use the Google Fonts download URL or `google-fonts-downloader`:

```bash
# Download Literata variable WOFF2 (Latin + Latin Extended subset)
# From: https://fonts.google.com/specimen/Literata
# Select: Regular 400, Display weight for headings
# Place as: frontend/src/web/fonts/literata.woff2
```

Manual download steps:
1. Go to https://fonts.google.com/specimen/Literata
2. Select "Regular 400" weight
3. Download family as WOFF2
4. Extract the variable `.woff2` file
5. Copy to `frontend/src/web/fonts/literata.woff2`

- [ ] **Step 3: Download Geist**

Download Geist variable from Google Fonts, subset to Latin + Latin Extended, WOFF2:

```bash
# Download Geist variable WOFF2 (Latin + Latin Extended subset)
# From: https://fonts.google.com/specimen/Geist
# Select: Regular 400, Medium 500, SemiBold 600
# Place as: frontend/src/web/fonts/geist.woff2
```

- [ ] **Step 4: Download Inconsolata LGC**

Download from the GitHub release:

```bash
curl -L -o /tmp/inconsolatalgc.tar.xz \
  "https://github.com/MihailJP/Inconsolata-LGC/releases/download/v3.200/InconsolataLGC-WOFF2-3.200.tar.xz"

tar -xf /tmp/inconsolatalgc.tar.xz -C /tmp/

# Copy the regular weight WOFF2 (InconsolataLGC-Regular.woff2)
cp /tmp/InconsolataLGC/InconsolataLGC-Regular.woff2 \
  frontend/src/web/fonts/inconsolatalgc.woff2

rm -rf /tmp/inconsolatalgc.tar.xz /tmp/InconsolataLGC
```

- [ ] **Step 5: Verify files exist**

```bash
ls -la frontend/src/web/fonts/
```

Expected: `literata.woff2`, `geist.woff2`, `inconsolatalgc.woff2` all present and non-zero.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/web/fonts/
git commit -m "feat: add Literata, Geist, Inconsolata LGC font files"
```

---

### Task 2: Create fonts.css with @font-face declarations

**Files:**
- Create: `frontend/src/web/fonts.css`

- [ ] **Step 1: Create fonts.css**

Write `frontend/src/web/fonts.css`:

```css
@font-face {
  font-family: 'Literata';
  font-style: normal;
  font-weight: 300 700;
  font-display: swap;
  src: url('./fonts/literata.woff2') format('woff2');
}

@font-face {
  font-family: 'Geist';
  font-style: normal;
  font-weight: 300 700;
  font-display: swap;
  src: url('./fonts/geist.woff2') format('woff2');
}

@font-face {
  font-family: 'Inconsolata LGC';
  font-style: normal;
  font-weight: 400;
  font-display: swap;
  src: url('./fonts/inconsolatalgc.woff2') format('woff2');
}
```

- [ ] **Step 2: Commit**

```bash
git add frontend/src/web/fonts.css
git commit -m "feat: add @font-face declarations for Literata, Geist, Inconsolata LGC"
```

---

### Task 3: Create tokens.css with all design tokens

**Files:**
- Create: `frontend/src/web/tokens.css`

- [ ] **Step 1: Create tokens.css**

Write `frontend/src/web/tokens.css` with all CSS custom properties from Appendix A, B, and C of the design spec:

```css
/* === HIERONYMUS DESIGN TOKENS === */

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

/* Dark theme (default) */
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
  --color-accent-text: #D4A84B;
  --color-accent-soft: #F0D78C;
  --color-accent-bg: rgba(212, 168, 75, 0.10);
  --color-danger: #C84040;
  --color-danger-text: #FF8A8A;
  --color-danger-bg: rgba(200, 64, 64, 0.10);
  --color-success: #5EA870;
  --color-success-text: #5EA870;
  --color-success-bg: rgba(94, 168, 112, 0.10);

  color: var(--color-text-primary);
  background: var(--color-bg-root);
  font-family: var(--font-sans);
}

/* Light theme (Paper) */
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

  color: var(--color-text-primary);
  background: var(--color-bg-root);
  font-family: var(--font-sans);
}

/* Smooth theme transitions */
[data-theme] {
  transition: color var(--duration-normal) ease,
              background-color var(--duration-normal) ease;
}

@media (prefers-reduced-motion: reduce) {
  [data-theme] { transition: none; }
}
```

- [ ] **Step 2: Commit**

```bash
git add frontend/src/web/tokens.css
git commit -m "feat: add design tokens CSS with dark + light theme custom properties"
```

---

### Task 4: Create base.css (reset, typography, layout shell)

**Files:**
- Create: `frontend/src/web/base.css`

- [ ] **Step 1: Create base.css**

Write `frontend/src/web/base.css`:

```css
/* === HIERONYMUS BASE STYLES === */

*,
*::before,
*::after {
  box-sizing: border-box;
}

body {
  margin: 0;
}

main {
  display: grid;
  grid-template-columns: var(--sidebar-width) minmax(0, 1fr);
  min-height: 100dvh;
}

/* --- Sidebar --- */

.sidebar {
  background: var(--color-bg-surface);
  display: flex;
  flex-direction: column;
  padding: var(--space-2xl);
}

.sidebar h1 {
  font: var(--text-display);
  font-size: 18px;
  margin: 0;
}

.sidebar p {
  color: var(--color-text-secondary);
  font: var(--text-caption);
  margin: var(--space-xs) 0 0;
}

.sidebar nav {
  display: grid;
  gap: var(--space-xs);
  margin-top: var(--space-3xl);
}

.sidebar a {
  color: var(--color-text-secondary);
  border-left: 2px solid transparent;
  font: var(--text-body-sm);
  padding: var(--space-sm) var(--space-md);
  text-decoration: none;
  transition: color var(--duration-fast) var(--ease-smooth),
              border-color var(--duration-fast) var(--ease-smooth);
}

.sidebar a:hover {
  color: var(--color-text-primary);
}

.sidebar a.active {
  border-left-color: var(--color-accent);
  color: var(--color-accent-text);
}

.sidebar footer {
  border-top: 1px solid var(--color-border-default);
  color: var(--color-text-tertiary);
  font: var(--text-caption);
  margin-top: auto;
  padding-top: var(--space-lg);
}

/* Theme toggle in sidebar footer */
.theme-toggle {
  align-items: center;
  background: none;
  border: none;
  color: var(--color-text-tertiary);
  cursor: pointer;
  display: flex;
  font: var(--text-caption);
  gap: var(--space-sm);
  margin-top: var(--space-md);
  padding: var(--space-sm) var(--space-md);
  min-width: 44px;
  min-height: 44px;
  transition: color var(--duration-fast) var(--ease-smooth);
}

.theme-toggle:hover {
  color: var(--color-text-secondary);
}

.theme-toggle:active {
  transform: scale(0.95);
  transition: transform 150ms var(--ease-smooth);
}

.theme-toggle svg {
  width: 20px;
  height: 20px;
}

/* --- Workspace --- */

.workspace {
  padding: var(--space-4xl);
}

/* --- Editorial Split --- */

.editorial-split {
  display: grid;
  gap: var(--space-3xl);
  grid-template-columns: var(--lead-width) minmax(0, 1fr);
}

.lead {
  position: sticky;
  top: var(--space-4xl);
  align-self: start;
}

.lead .eyebrow {
  color: var(--color-accent-text);
  background: var(--color-accent-bg);
  border: 1px solid var(--color-accent);
  border-radius: var(--radius-pill);
  display: inline-block;
  font: var(--text-eyebrow);
  letter-spacing: 0.12em;
  margin: 0 0 var(--space-md);
  padding: 2px 10px;
  text-transform: uppercase;
}

.lead h2 {
  font: var(--text-display);
  margin: 0;
}

.lead > p {
  color: var(--color-text-secondary);
  font: var(--text-body);
  margin: var(--space-md) 0 0;
  max-width: 36ch;
}

.lead .lead-meta {
  border-top: 1px solid var(--color-border-default);
  color: var(--color-text-secondary);
  font: var(--text-caption);
  margin-top: var(--space-xl);
  padding-top: var(--space-md);
}

.stage {
  min-width: 0;
}

/* --- Config pages (no split) --- */

.settings {
  max-width: var(--content-max);
}

.settings-page .page-header {
  margin-bottom: var(--space-2xl);
}

.settings-page .page-header h2 {
  font: var(--text-h2);
  margin: 0;
}

.settings-page .page-header p {
  color: var(--color-text-secondary);
  font: var(--text-body);
  margin: var(--space-sm) 0 0;
}

.settings-grid {
  display: grid;
  gap: var(--space-lg);
  grid-template-columns: repeat(2, minmax(0, 1fr));
}

/* --- Responsive --- */

@media (max-width: 1080px) {
  .editorial-split {
    grid-template-columns: 240px minmax(0, 1fr);
  }
}

@media (max-width: 960px) {
  .editorial-split {
    grid-template-columns: 1fr;
  }
  .lead {
    position: static;
  }
  .lead > p {
    max-width: none;
  }
}

@media (max-width: 720px) {
  main {
    grid-template-columns: 1fr;
    grid-template-rows: auto 1fr;
  }
  .sidebar {
    padding: var(--space-lg);
  }
  .workspace {
    padding: var(--space-xl);
  }
  .settings-grid {
    grid-template-columns: 1fr;
  }
}
```

- [ ] **Step 2: Commit**

```bash
git add frontend/src/web/base.css
git commit -m "feat: add base.css with reset, typography, sidebar, editorial split, responsive"
```

---

### Task 5: Create theme.svelte.ts (theme toggle rune)

**Files:**
- Create: `frontend/src/web/lib/theme.svelte.ts`

- [ ] **Step 1: Create theme.svelte.ts**

Write `frontend/src/web/lib/theme.svelte.ts`:

```typescript
type Theme = "dark" | "light";

const STORAGE_KEY = "hiero-theme";

function systemPreference(): Theme {
  if (typeof window === "undefined") return "dark";
  return window.matchMedia("(prefers-color-scheme: light)").matches
    ? "light"
    : "dark";
}

function storedTheme(): Theme | null {
  if (typeof localStorage === "undefined") return null;
  const value = localStorage.getItem(STORAGE_KEY);
  if (value === "dark" || value === "light") return value;
  return null;
}

function applyTheme(theme: Theme): void {
  document.documentElement.setAttribute("data-theme", theme);
}

let current = $state<Theme>(storedTheme() ?? systemPreference());

applyTheme(current);

export function createThemeToggle() {
  function toggle(): void {
    current = current === "dark" ? "light" : "dark";
    localStorage.setItem(STORAGE_KEY, current);
    applyTheme(current);
  }

  return {
    get theme(): Theme {
      return current;
    },
    toggle,
  };
}
```

- [ ] **Step 2: Verify TypeScript compilation**

```bash
bun run --cwd frontend build
```

Expected: Build succeeds. The theme module is type-checked as part of the Svelte/Vite build pipeline.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/web/lib/theme.svelte.ts
git commit -m "feat: add theme toggle rune module with localStorage persistence"
```

---

### Task 6: Create components.css (all component patterns)

**Files:**
- Create: `frontend/src/web/components.css`

- [ ] **Step 1: Create components.css**

Write `frontend/src/web/components.css`:

```css
/* === HIERONYMUS COMPONENTS === */

/* --- Cards (nested shell) --- */

.card {
  background: var(--color-bg-surface);
  border: 1px solid var(--color-border-default);
  border-radius: var(--radius-md);
  padding: 1px;
}

.card > * {
  background: var(--color-bg-root);
  border-radius: var(--radius-sm);
  padding: var(--space-lg);
}

.stats {
  display: grid;
  gap: var(--space-md);
  grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
}

.stat-card strong {
  color: var(--color-accent-text);
  display: block;
  font: var(--text-mono);
  font-size: 24px;
  font-weight: 400;
}

.stat-card span {
  color: var(--color-text-secondary);
  display: block;
  font: var(--text-eyebrow);
  letter-spacing: 0.08em;
  margin-top: var(--space-xs);
  text-transform: uppercase;
}

/* Service status card */
.status-card {
  margin-top: var(--space-lg);
}

.status-card h3 {
  font: var(--text-h3);
  margin: 0 0 var(--space-md);
}

.status-card dl {
  display: flex;
  gap: var(--space-3xl);
  margin: 0;
}

.status-card dt {
  color: var(--color-text-secondary);
  font: var(--text-caption);
}

.status-card dd {
  font: var(--text-body);
  margin: var(--space-xs) 0 0;
}

/* --- Tables --- */

.table-wrap {
  border: 1px solid var(--color-border-default);
  border-radius: var(--radius-md);
  overflow: hidden;
}

table {
  border-collapse: collapse;
  margin: 0;
  width: 100%;
}

th {
  border-bottom: 1px solid var(--color-border-default);
  color: var(--color-accent-text);
  font: var(--text-eyebrow);
  font-weight: 500;
  letter-spacing: 0.08em;
  padding: var(--space-md) var(--space-lg);
  text-align: left;
  text-transform: uppercase;
}

td {
  border-bottom: 1px solid var(--color-border-default);
  font: var(--text-body);
  padding: var(--space-md) var(--space-lg);
}

tr:nth-child(even) td {
  background: var(--color-bg-surface);
}

tbody tr {
  cursor: default;
  transition: background var(--duration-fast) var(--ease-smooth);
}

tbody tr:hover td {
  background: var(--color-bg-raised);
}

tbody tr.selected td {
  background: var(--color-bg-raised);
}

tbody tr.selected td:first-child {
  border-left: 2px solid var(--color-accent);
  padding-left: calc(var(--space-lg) - 2px);
}

td small {
  color: var(--color-text-secondary);
  display: block;
  font: var(--text-caption);
  margin-top: var(--space-xs);
}

/* Table empty state */
td.empty-cell {
  color: var(--color-text-secondary);
  font: var(--text-body);
  padding: var(--space-3xl) var(--space-lg);
  text-align: center;
}

/* Table-based row selection (Memory Views) */
tbody tr[role="button"] {
  cursor: pointer;
}

/* --- Forms --- */

form {
  display: grid;
  gap: var(--space-lg);
}

label {
  color: var(--color-text-secondary);
  display: grid;
  font: var(--text-caption);
  gap: var(--space-xs);
}

input,
select,
textarea {
  background: var(--color-bg-raised);
  border: 1px solid var(--color-border-strong);
  border-radius: var(--radius-sm);
  color: var(--color-text-primary);
  font: var(--text-body);
  font-size: 16px;
  padding: 10px 12px;
  transition: border-color var(--duration-fast) var(--ease-smooth);
}

input:focus,
select:focus,
textarea:focus {
  border-color: var(--color-accent);
  outline: none;
}

textarea {
  min-height: 80px;
  resize: vertical;
}

fieldset {
  border: 1px solid var(--color-border-strong);
  display: grid;
  gap: var(--space-lg);
  padding: var(--space-lg);
}

legend {
  color: var(--color-accent-text);
  font: var(--text-body-sm);
  font-weight: 500;
}

/* Toggle switch */
.toggle-label {
  align-items: center;
  cursor: pointer;
  display: flex;
  gap: var(--space-sm);
  min-height: 44px;
}

.toggle-track {
  background: var(--color-bg-raised);
  border: 1px solid var(--color-border-strong);
  border-radius: var(--radius-pill);
  flex-shrink: 0;
  height: 18px;
  position: relative;
  width: 32px;
  transition: border-color var(--duration-fast) var(--ease-smooth);
}

.toggle-thumb {
  background: var(--color-text-secondary);
  border-radius: 50%;
  height: 14px;
  left: 1px;
  position: absolute;
  top: 1px;
  width: 14px;
  transition: transform var(--duration-fast) var(--ease-smooth),
              background var(--duration-fast) var(--ease-smooth);
}

.toggle-label input:checked + .toggle-track {
  border-color: var(--color-accent);
}

.toggle-label input:checked + .toggle-track .toggle-thumb {
  background: var(--color-accent-text);
  transform: translateX(14px);
}

/* Radio buttons */
.radio-label {
  align-items: center;
  cursor: pointer;
  display: flex;
  font: var(--text-body);
  gap: var(--space-sm);
}

input[type="checkbox"],
input[type="radio"] {
  accent-color: var(--color-accent-text);
}

/* --- Buttons --- */

button,
.btn {
  align-items: center;
  border-radius: var(--radius-sm);
  cursor: pointer;
  display: inline-flex;
  font: var(--text-body-sm);
  font-weight: 500;
  gap: var(--space-sm);
  height: 34px;
  justify-content: center;
  padding: 0 var(--space-lg);
  transition: background var(--duration-fast) var(--ease-smooth),
              border-color var(--duration-fast) var(--ease-smooth),
              color var(--duration-fast) var(--ease-smooth),
              transform 100ms var(--ease-smooth);
}

button:active:not(:disabled),
.btn:active:not(:disabled) {
  transform: scale(0.98);
}

button:disabled,
.btn:disabled {
  cursor: default;
  opacity: 0.4;
}

.btn-primary {
  background: var(--color-bg-raised);
  border: 1px solid var(--color-accent);
  color: var(--color-accent-text);
}

.btn-primary:hover:not(:disabled) {
  background: var(--color-accent-bg);
  border-color: var(--color-accent-soft);
}

.btn-secondary {
  background: var(--color-bg-surface);
  border: 1px solid var(--color-border-default);
  color: var(--color-text-primary);
}

.btn-secondary:hover:not(:disabled) {
  background: var(--color-bg-raised);
}

.btn-danger {
  background: var(--color-bg-raised);
  border: 1px solid var(--color-danger);
  color: var(--color-danger-text);
}

.btn-danger:hover:not(:disabled) {
  background: var(--color-danger-bg);
}

.btn-icon {
  background: none;
  border: none;
  color: var(--color-text-secondary);
  font-size: 20px;
  height: 32px;
  min-width: 32px;
  padding: 0;
  width: 32px;
}

.btn-icon:hover:not(:disabled) {
  color: var(--color-text-primary);
}

/* --- Tabs --- */

.view-tabs {
  border-bottom: 1px solid var(--color-border-default);
  display: flex;
  flex-wrap: wrap;
  gap: 0;
  margin-bottom: var(--space-xl);
  padding: 0 0 var(--space-sm);
}

.tab-btn {
  background: none;
  border: none;
  border-bottom: 2px solid transparent;
  border-radius: 0;
  color: var(--color-text-secondary);
  cursor: pointer;
  font: var(--text-body-sm);
  height: auto;
  margin-right: var(--space-xl);
  padding: var(--space-sm) 0;
  transition: color var(--duration-fast) var(--ease-smooth),
              border-color var(--duration-fast) var(--ease-smooth);
}

.tab-btn:hover {
  color: var(--color-text-primary);
}

.tab-btn.active {
  border-bottom-color: var(--color-accent);
  color: var(--color-accent-text);
}

/* --- Slide-out editor --- */

.editor-backdrop {
  animation: fadeIn var(--duration-normal) var(--ease-smooth);
  background: var(--color-bg-overlay);
  backdrop-filter: blur(4px);
  inset: 0;
  position: fixed;
  z-index: 10;
}

.editor-panel {
  animation: slideIn var(--duration-normal) var(--ease-smooth);
  background: var(--color-bg-root);
  border-left: 1px solid var(--color-border-default);
  box-shadow: -16px 0 40px rgba(0, 0, 0, 0.15);
  display: flex;
  flex-direction: column;
  height: 100dvh;
  padding: var(--space-xl);
  position: fixed;
  right: 0;
  top: 0;
  width: var(--editor-width);
  z-index: 11;
}

.editor-panel header {
  align-items: center;
  border-bottom: 1px solid var(--color-border-default);
  display: flex;
  justify-content: space-between;
  margin-bottom: var(--space-xl);
  padding-bottom: var(--space-lg);
}

.editor-panel h2 {
  font: var(--text-h3);
  margin: 0;
}

.editor-panel form {
  flex: 1;
  overflow-y: auto;
}

.editor-panel .editor-footer {
  border-top: 1px solid var(--color-border-default);
  display: flex;
  gap: var(--space-sm);
  margin-top: var(--space-lg);
  padding-top: var(--space-lg);
}

.editor-panel .models-section {
  border-top: 1px solid var(--color-border-default);
  margin-top: var(--space-xl);
  padding-top: var(--space-lg);
}

.editor-panel .models-section h3 {
  font: var(--text-h3);
  margin: 0 0 var(--space-md);
}

.editor-panel .models-section ul {
  list-style: none;
  margin: 0;
  max-height: 180px;
  overflow-y: auto;
  padding: 0;
}

.editor-panel .models-section li {
  border-bottom: 1px solid var(--color-border-default);
  font: var(--text-mono);
  padding: var(--space-sm) 0;
}

.editor-panel .models-section p {
  color: var(--color-text-secondary);
  font: var(--text-body-sm);
}

.editor-panel .btn-danger {
  margin-top: var(--space-lg);
}

@keyframes fadeIn {
  from { opacity: 0; }
  to { opacity: 1; }
}

@keyframes slideIn {
  from { transform: translateX(100%); }
  to { transform: translateX(0); }
}

@media (max-width: 480px) {
  .editor-panel {
    width: 100%;
  }
}

/* --- Detail panel (Memory Views) --- */

.detail-panel {
  background: var(--color-bg-surface);
  border: 1px solid var(--color-border-default);
  border-radius: var(--radius-md);
  display: flex;
  flex-direction: column;
  min-height: 360px;
  overflow: hidden;
}

.detail-panel > * {
  padding: var(--space-xl);
}

.detail-panel .eyebrow {
  color: var(--color-accent-text);
  font: var(--text-eyebrow);
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.detail-panel h3 {
  font: var(--text-h3);
  margin: var(--space-xs) 0 0;
}

.detail-panel .detail-subtitle {
  color: var(--color-text-secondary);
  font: var(--text-body-sm);
  margin: var(--space-xs) 0 0;
}

.detail-panel .detail-body {
  background: var(--color-bg-raised);
  border-left: 3px solid var(--color-accent);
  font: var(--font-serif);
  font-size: 15px;
  line-height: 1.7;
  margin: var(--space-xl);
  min-height: 120px;
  overflow: auto;
  padding: var(--space-lg);
  white-space: pre-wrap;
}

.detail-panel dl {
  display: grid;
  gap: var(--space-sm);
  margin: 0;
}

.detail-panel dl div {
  border-top: 1px solid var(--color-border-default);
  padding-top: var(--space-sm);
}

.detail-panel dt {
  color: var(--color-text-secondary);
  font: var(--text-caption);
}

.detail-panel dd {
  font: var(--text-mono);
  margin: var(--space-xs) 0 0;
  overflow-wrap: anywhere;
}

.detail-actions {
  border-top: 1px solid var(--color-border-default);
  margin-top: auto;
}

.detail-actions h4 {
  font: var(--text-body-sm);
  font-weight: 500;
  margin: 0 0 var(--space-sm);
}

.detail-actions > div {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-sm);
}

/* Confirm destructive action */
.confirm-action {
  background: var(--color-danger-bg);
  border: 1px solid var(--color-danger);
  margin-top: var(--space-lg);
  padding: var(--space-lg);
}

.confirm-action p {
  font: var(--text-body-sm);
  margin: 0 0 var(--space-md);
}

.confirm-action > div {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-sm);
}

/* --- Toast notifications --- */

.toast {
  align-items: center;
  animation: toastIn 250ms var(--ease-smooth);
  border-left: 3px solid var(--color-success);
  background: var(--color-success-bg);
  bottom: var(--space-xl);
  color: var(--color-text-primary);
  display: flex;
  font: var(--text-body-sm);
  gap: var(--space-lg);
  max-width: 400px;
  padding: var(--space-md) var(--space-lg);
  position: fixed;
  right: var(--space-xl);
  z-index: 20;
}

.toast.toast-error {
  border-left-color: var(--color-danger);
  background: var(--color-danger-bg);
}

.toast.toast-success .toast-msg {
  color: var(--color-success-text);
}

.toast.toast-error .toast-msg {
  color: var(--color-danger-text);
}

@keyframes toastIn {
  from {
    opacity: 0;
    transform: translateY(16px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@media (max-width: 480px) {
  .toast {
    left: var(--space-lg);
    max-width: none;
    right: var(--space-lg);
  }
}

/* --- Workflow cards --- */

.workflow-card {
  margin-top: var(--space-lg);
}

.workflow-card header {
  display: flex;
  justify-content: space-between;
  margin-bottom: var(--space-lg);
}

.workflow-card header > div {
  align-items: center;
  display: flex;
  gap: var(--space-lg);
}

.workflow-card h4 {
  font: var(--text-h3);
  margin: 0;
  text-transform: capitalize;
}

/* --- Prompt textarea --- */

.prompt {
  margin-top: var(--space-2xl);
}

/* --- Danger zone (providers) --- */

.danger-zone {
  margin-top: var(--space-lg);
}

/* --- Loading --- */

.loading {
  color: var(--color-text-secondary);
  font: var(--text-body);
}

/* --- Error --- */

.error-msg {
  color: var(--color-danger-text);
  font: var(--text-body-sm);
}

/* --- Reduced motion --- */

@media (prefers-reduced-motion: reduce) {
  *,
  *::before,
  *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
  }
}
```

- [ ] **Step 2: Commit**

```bash
git add frontend/src/web/components.css
git commit -m "feat: add components.css with all component patterns"
```

---

### Task 7: Update main.ts and index.html to wire up new CSS

**Files:**
- Modify: `frontend/src/web/main.ts`
- Modify: `frontend/index.html`

- [ ] **Step 1: Update main.ts**

Replace the single `styles.css` import with the modular CSS imports. Edit `frontend/src/web/main.ts`:

```typescript
import { mount } from "svelte";
import App from "./App.svelte";
import "./fonts.css";
import "./tokens.css";
import "./base.css";
import "./components.css";

mount(App, { target: document.getElementById("app")! });
```

- [ ] **Step 2: Update index.html**

Add `data-theme="dark"` as the default to prevent a flash of unstyled content. Edit `frontend/index.html`:

```html
<!doctype html>
<html lang="en" data-theme="dark">
  <head><meta charset="UTF-8" /><title>Hieronymus Web Console</title></head>
  <body><div id="app"></div><script type="module" src="/src/web/main.ts"></script></body>
</html>
```

- [ ] **Step 3: Verify build**

```bash
bun run --cwd frontend build
```

Expected: Build succeeds. CSS files are bundled and hashed. The old `styles.css` is still imported (not yet removed) but the new CSS should also be present.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/web/main.ts frontend/index.html
git commit -m "feat: wire up modular CSS and default dark theme attribute"
```

---

### Task 8: Redesign App.svelte (layout shell + editorial split)

**Files:**
- Modify: `frontend/src/web/App.svelte`

This is the largest single task. Replace the current `App.svelte` markup with the editorial split layout while keeping all script logic intact.

- [ ] **Step 1: Update App.svelte markup**

The `<script>` section stays identical (all imports, state, functions, onMount). Only the template section changes. Replace the template in `frontend/src/web/App.svelte` starting at the `<main>` tag. The `<script>` block remains unchanged.

Replace lines 169-192 (the `<main>` block) with:

```svelte
<main>
  <aside class="sidebar">
    <h1>Hieronymus</h1>
    <p>{section === "admin" || section === "memory" ? "local administration" : "local configuration"}</p>
    <nav>
      <a class:active={section === "admin"} href="/admin">Overview</a>
      <a class:active={section === "memory"} href="/admin/memory">Memory views</a>
      <a class:active={section === "providers"} href="/config">Providers</a>
      <a class:active={section === "dreaming"} href="/config/dreaming">Dreaming</a>
      <a class:active={section === "ingest"} href="/config/ingest">Ingest</a>
      <a class:active={section === "release"} href="/config/release">Release</a>
    </nav>
    <footer>
      All data is local.<br />No cloud. No tracking.
      <button class="theme-toggle" aria-label={themeToggle.theme === "dark" ? "Switch to light theme" : "Switch to dark theme"} onclick={themeToggle.toggle}>
        {#if themeToggle.theme === "dark"}
          <svg viewBox="0 0 20 20" fill="currentColor"><path d="M10 2a1 1 0 011 1v1a1 1 0 11-2 0V3a1 1 0 011-1zm4 8a4 4 0 11-8 0 4 4 0 018 0zm-.464 4.95l.707.707a1 1 0 001.414-1.414l-.707-.707a1 1 0 00-1.414 1.414zm2.12-10.607a1 1 0 010 1.414l-.706.707a1 1 0 11-1.414-1.414l.707-.707a1 1 0 011.414 0zM17 11a1 1 0 100-2h-1a1 1 0 100 2h1zm-7 4a1 1 0 011 1v1a1 1 0 11-2 0v-1a1 1 0 011-1zM5.05 6.464A1 1 0 106.465 5.05l-.708-.707a1 1 0 00-1.414 1.414l.707.707zm1.414 8.486l-.707.707a1 1 0 01-1.414-1.414l.707-.707a1 1 0 011.414 1.414zM4 11a1 1 0 100-2H3a1 1 0 000 2h1z" /></svg>
        {:else}
          <svg viewBox="0 0 20 20" fill="currentColor"><path d="M17.293 13.293A8 8 0 016.707 2.707a8.001 8.001 0 1010.586 10.586z" /></svg>
        {/if}
        {themeToggle.theme === "dark" ? "Light" : "Dark"}
      </button>
    </footer>
  </aside>

  <div class="workspace">
    {#if section === "admin" && adminDashboard}
      <AdminDashboard dashboard={adminDashboard} {error} />
    {:else if section === "memory" && adminDashboard}
      <MemoryViews dashboard={adminDashboard} onNotice={({ message, tone }) => showNotice(message, tone)} />
    {:else if section === "providers"}
      <ProvidersPage
        {providers}
        {selected}
        {busy}
        {error}
        {models}
        onCreate={() => { createOpen = true; selected = null; models = []; }}
        onSelect={(provider) => { selected = provider; createOpen = false; models = []; }}
      />
    {:else if section === "dreaming" && dreamSettings}
      {#key "dreaming"}<DreamingEditor initial={dreamSettings} providers={dreamProviders} {modelCache} {busy} {error} onSave={saveDream} />{/key}
    {:else if section === "ingest" && ingestSettings}
      {#key "ingest"}<IngestEditor initial={ingestSettings} {busy} {error} onSave={saveIngest} />{/key}
    {:else if section === "release" && releaseSettings}
      {#key "release"}<ReleaseEditor initial={releaseSettings} {busy} {error} onSave={saveRelease} />{/key}
    {:else if error}
      <p class="error-msg">{error}</p>
    {:else}
      <p class="loading">Loading settings…</p>
    {/if}
  </div>

  {#if section === "providers" && (selected || createOpen)}
    {#key selected?.id ?? "new"}
      <div class="editor-backdrop" onclick={() => { selected = null; createOpen = false; error = ""; }} role="presentation"></div>
      <ProviderEditor provider={selected} {models} {busy} {error} onSave={save} onDelete={remove} onCheck={check} onRefreshModels={refresh} onClose={() => { selected = null; createOpen = false; error = ""; }} />
    {/key}
  {/if}

  {#if notice}
    <Toast message={notice.message} tone={notice.tone} onDismiss={() => { notice = null; }} />
  {/if}
</main>
```

- [ ] **Step 2: Add theme toggle import to script block**

Add the theme toggle import at the top of the `<script>` block in `App.svelte`, after the existing imports:

```typescript
import { createThemeToggle } from "./lib/theme.svelte.ts";
```

And add the theme toggle instance after the `let notice` state declaration (around line 58):

```typescript
const themeToggle = createThemeToggle();
```

- [ ] **Step 3: Add the ProvidersPage inline component**

The Providers list was previously rendered inline in App.svelte. To not require a separate file, add it inline after the imports but before the existing state declarations. Add this `<script>` snippet:

```typescript
// Providers page is an inline component — no separate file needed.
// It renders via <ProvidersPage ... /> in the template below.
```

Add a Svelte `<svelte:component>` reference. Actually, convert the inline provider list to use the new component classes. The inline template for providers becomes:

Replace this section of the template (the `{:else if section === "providers"}` block) with:

```svelte
    {:else if section === "providers"}
      <div class="settings-page">
        <header class="page-header">
          <div>
            <h2>Providers</h2>
            <p>Manage LLM provider profiles. Create profiles for OpenAI, Anthropic, Google, or local Ollama endpoints.</p>
          </div>
          <button class="btn-primary" onclick={() => { createOpen = true; selected = null; models = []; }}>New provider</button>
        </header>

        {#if error}<p class="error-msg">{error}</p>{/if}

        {#if busy && providers.length === 0}
          <p class="loading">Loading profiles…</p>
        {:else if providers.length === 0}
          <div class="table-wrap">
            <table><tbody><tr><td class="empty-cell">No provider profiles yet. Create one to connect an LLM.</td></tr></tbody></table>
          </div>
        {:else}
          <div class="table-wrap">
            <table>
              <thead><tr><th>Display name</th><th>Type</th><th>Endpoint</th><th>Key</th></tr></thead>
              <tbody>
                {#each providers as provider (provider.id)}
                  <tr class:selected={selected?.id === provider.id} role="button" tabindex="0" onclick={() => { selected = provider; createOpen = false; models = []; }} onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { selected = provider; createOpen = false; models = []; } }}>
                    <td>{provider.name}</td>
                    <td>{provider.type}</td>
                    <td>{provider.url}</td>
                    <td>{provider.key_configured ? "Configured" : "Missing"}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
      </div>
```

- [ ] **Step 4: Verify build**

```bash
bun run --cwd frontend build
```

Expected: Build succeeds. The old styles.css still provides fallback for components not yet migrated.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/web/App.svelte
git commit -m "feat: redesign App.svelte with editorial split layout, theme toggle, and provider list"
```

---

### Task 9: Redesign Toast.svelte

**Files:**
- Modify: `frontend/src/web/components/Toast.svelte`

- [ ] **Step 1: Update Toast.svelte**

The script logic is unchanged. Update the template to use new class names:

```svelte
<script lang="ts">
  type Props = {
    message: string;
    tone?: "success" | "error";
    onDismiss: () => void;
  };
  let { message, tone = "success", onDismiss }: Props = $props();
</script>

<aside class="toast" class:toast-error={tone === "error"} role="status" aria-live="polite">
  <span class="toast-msg">{message}</span>
  <button class="btn-icon" aria-label="Dismiss notification" onclick={onDismiss}>&times;</button>
</aside>
```

- [ ] **Step 2: Verify build**

```bash
bun run --cwd frontend build
```

- [ ] **Step 3: Commit**

```bash
git add frontend/src/web/components/Toast.svelte
git commit -m "feat: redesign Toast with new component classes"
```

---

### Task 10: Redesign AdminDashboard.svelte

**Files:**
- Modify: `frontend/src/web/components/AdminDashboard.svelte`

- [ ] **Step 1: Update AdminDashboard.svelte**

The script block (lines 1-7) stays unchanged. Replace the template (lines 9-19) with:

```svelte
<section class="editorial-split" aria-label="Administration overview">
  <div class="lead">
    <p class="eyebrow">{dashboard.header.version}</p>
    <h2>{dashboard.header.product}</h2>
    <p>{dashboard.header.tagline}</p>
    <div class="lead-meta">
      <a class="btn-primary" href="/admin/memory">Open memory views</a>
      <a class="btn-secondary" href="/config" style="margin-left: 8px;">Open configuration</a>
    </div>
  </div>

  <div class="stage">
    <div class="stats" aria-label="Memory statistics">
      {#each Object.entries(dashboard.stats) as [name, value] (name)}
        <div class="card stat-card">
          <strong>{value}</strong>
          <span>{name.replaceAll("_", " ")}</span>
        </div>
      {/each}
    </div>

    <div class="card status-card">
      <h3>Local service</h3>
      <dl>
        <div>
          <dt>Dreaming</dt>
          <dd>{String(dashboard.dream_status.state ?? "unknown")}</dd>
        </div>
        <div>
          <dt>Short-term memory</dt>
          <dd>{String(dashboard.short_term_status.state ?? "unknown")}</dd>
        </div>
      </dl>
    </div>

    {#if error}<p class="error-msg" style="margin-top: var(--space-lg)">{error}</p>{/if}
  </div>
</section>
```

- [ ] **Step 2: Verify build**

```bash
bun run --cwd frontend build
```

- [ ] **Step 3: Commit**

```bash
git add frontend/src/web/components/AdminDashboard.svelte
git commit -m "feat: redesign AdminDashboard with editorial split and nested stat cards"
```

---

### Task 11: Redesign MemoryViews.svelte

**Files:**
- Modify: `frontend/src/web/components/MemoryViews.svelte`

This is the most complex component. The script block (lines 1-113) stays entirely unchanged. Only the template section changes.

- [ ] **Step 1: Replace the MemoryViews template**

Replace lines 116-179 (the template section `<section class="memory-workspace" ...>` through `</section>`) with:

```svelte
<section class="editorial-split" aria-label="Memory views">
  <div class="lead">
    <p class="eyebrow">Memory administration</p>
    <h2>Memory views</h2>
    <p>Find a record, read its context, then curate only the memory that needs attention.</p>
    <div class="lead-meta">{snapshot?.rows.length ?? 0} records</div>
  </div>

  <div class="stage">
    <nav class="view-tabs" aria-label="Memory view selector">
      {#each dashboard.views as view (view)}
        <button class="tab-btn" class:active={selectedView === view} onclick={() => void load(view)}>{view}</button>
      {/each}
    </nav>

    {#if error}<p class="error-msg">{error}</p>{/if}

    {#if loading}
      <p class="loading">Loading {selectedView}…</p>
    {:else if snapshot}
      <div class="memory-split">
        <div class="table-wrap" aria-label={`${selectedView} records`}>
          {#if snapshot.rows.length}
            <table class="memory-table">
              <thead><tr><th>Record</th><th>Kind</th><th>Status</th><th>Scope</th></tr></thead>
              <tbody>
                {#each snapshot.rows as row (row.id)}
                  <tr class:selected={snapshot.selected?.id === row.id} role="button" tabindex="0" onclick={() => void load(selectedView, row.id)} onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') void load(selectedView, row.id); }}>
                    <td><strong>{row.label}</strong><small>{row.language_pair}</small></td>
                    <td>{row.kind}</td>
                    <td>{row.status}</td>
                    <td>{row.scope}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {:else}
            <table><tbody><tr><td class="empty-cell">No {selectedView.toLowerCase()} yet. {snapshot.detail.subtitle}</td></tr></tbody></table>
          {/if}
        </div>

        <div class="detail-panel" aria-label="Selected memory record">
          {#if snapshot.selected}
            <div>
              <p class="eyebrow">{snapshot.selected.kind} &middot; {snapshot.selected.status}</p>
              <h3>{snapshot.detail.title}</h3>
              <p class="detail-subtitle">{snapshot.detail.subtitle}</p>
            </div>
            {#if snapshot.detail.body}
              <pre class="detail-body">{snapshot.detail.body}</pre>
            {/if}
            {#if snapshot.detail.fields.length}
              <dl>
                {#each snapshot.detail.fields as [name, value] (name)}
                  <div>
                    <dt>{name}</dt>
                    <dd>{value}</dd>
                  </div>
                {/each}
              </dl>
            {/if}
            {#if actionsFor(snapshot.selected).length}
              <div class="detail-actions">
                <h4>Actions</h4>
                <div>
                  {#each actionsFor(snapshot.selected) as action (action)}
                    <button
                      class={destructiveActions.has(action) ? 'btn-danger' : 'btn-secondary'}
                      disabled={runningAction !== null}
                      onclick={() => requestAction(action)}
                    >
                      {runningAction === action ? "Working…" : actionLabels[action]}
                    </button>
                  {/each}
                </div>
              </div>
            {/if}
            {#if pendingAction}
              <div class="confirm-action" aria-live="polite">
                <p><strong>{actionLabels[pendingAction]} "{snapshot.selected.label}"?</strong> This action changes the stored memory.</p>
                <div>
                  <button class="btn-danger" disabled={runningAction !== null} onclick={() => void perform(pendingAction, true)}>Confirm {actionLabels[pendingAction]}</button>
                  <button class="btn-secondary" disabled={runningAction !== null} onclick={() => { pendingAction = null; }}>Cancel</button>
                </div>
              </div>
            {/if}
          {:else}
            <div style="display:flex;align-items:center;justify-content:center;height:100%;color:var(--color-text-secondary);font:var(--text-body)">
              <p>Select a record to view its source, status, and available actions.</p>
            </div>
          {/if}
        </div>
      </div>
    {/if}
  </div>
</section>
```

- [ ] **Step 2: Add CSS for memory-split layout**

The `memory-split` class needs to be in `components.css` (or scoped in the Svelte component). Add to the end of `frontend/src/web/components.css`:

```css
/* --- Memory split layout --- */

.memory-split {
  display: grid;
  gap: var(--space-xl);
  grid-template-columns: minmax(480px, 1.45fr) minmax(320px, 0.8fr);
}

@media (max-width: 1050px) {
  .memory-split {
    grid-template-columns: 1fr;
  }
}
```

- [ ] **Step 3: Verify build**

```bash
bun run --cwd frontend build
```

- [ ] **Step 4: Commit**

```bash
git add frontend/src/web/components/MemoryViews.svelte frontend/src/web/components.css
git commit -m "feat: redesign MemoryViews with editorial split, styled table, and detail panel"
```

---

### Task 12: Redesign ProviderEditor.svelte

**Files:**
- Modify: `frontend/src/web/components/ProviderEditor.svelte`

- [ ] **Step 1: Update ProviderEditor template**

Script block (lines 1-47) stays unchanged. Replace lines 49-62 (the `<aside>` template) with:

```svelte
<aside class="editor-panel" aria-label="Provider editor" role="dialog" aria-modal="true">
  <header>
    <h2>{provider ? `Edit ${provider.name}` : "New provider"}</h2>
    <button class="btn-icon" aria-label="Close editor" onclick={onClose}>&times;</button>
  </header>
  <form onsubmit={(event) => { event.preventDefault(); submit(); }}>
    <label>Profile ID<input bind:value={draft.id} disabled={provider !== null} required pattern="[A-Za-z0-9_-]+" /></label>
    <label>Display name<input bind:value={draft.name} required /></label>
    <label>Provider type
      <select bind:value={draft.type}>
        <option value="openai">OpenAI compatible</option>
        <option value="google">Google GenAI</option>
        <option value="anthropic">Anthropic</option>
        <option value="ollama">Ollama</option>
      </select>
    </label>
    <label>Endpoint<input bind:value={draft.url} required placeholder="https://api.example.com/v1" /></label>
    <label>API key<input bind:value={draft.key} type="password" placeholder={provider?.key_configured ? "Stored key (leave blank to keep)" : "Required for remote providers"} /></label>
    <label>Timeout (seconds)<input bind:value={draft.timeout_seconds} inputmode="numeric" required /></label>
    {#if error}<p class="error-msg">{error}</p>{/if}
    <div class="editor-footer">
      <button class="btn-primary" disabled={busy}>Save profile</button>
      {#if provider}
        <button class="btn-secondary" type="button" onclick={onCheck} disabled={busy}>Check connection</button>
        <button class="btn-secondary" type="button" onclick={onRefreshModels} disabled={busy}>Refresh models</button>
      {/if}
    </div>
  </form>
  {#if provider}
    <div class="models-section">
      <h3>Discovered models</h3>
      {#if models.length}
        <ul>{#each models as model (model)}<li>{model}</li>{/each}</ul>
      {:else}
        <p>No cached models. Refresh after testing the connection.</p>
      {/if}
    </div>
    <button class="btn-danger" onclick={onDelete} disabled={busy}>Delete provider</button>
  {/if}
</aside>
```

- [ ] **Step 2: Verify build**

```bash
bun run --cwd frontend build
```

- [ ] **Step 3: Commit**

```bash
git add frontend/src/web/components/ProviderEditor.svelte
git commit -m "feat: redesign ProviderEditor slide-out with backdrop and updated form"
```

---

### Task 13: Redesign DreamingEditor.svelte

**Files:**
- Modify: `frontend/src/web/components/DreamingEditor.svelte`

- [ ] **Step 1: Update DreamingEditor template**

Script block (lines 1-43) stays unchanged. Replace the template (lines 46-79) with:

```svelte
<section class="settings-page" aria-label="Dreaming settings">
  <header class="page-header">
    <div>
      <h2>Dreaming</h2>
      <p>Schedule automated memory crystallization and assign workflows to provider models.</p>
    </div>
    <button class="btn-primary" disabled={busy} onclick={() => onSave($state.snapshot(settings))}>Save dreaming</button>
  </header>

  <div class="settings-grid">
    <label class="toggle-label">
      <input type="checkbox" bind:checked={settings.dreaming.enabled} />
      <span class="toggle-track"><span class="toggle-thumb"></span></span>
      Enable scheduled dreaming
    </label>
    <label>Interval (minutes)<input type="number" min="1" bind:value={settings.dreaming.schedule_interval_minutes} /></label>
    <label>Minimum pending memories<input type="number" min="1" bind:value={settings.dreaming.min_pending_short_term_memories} /></label>
    <label>Maximum pending memories<input type="number" min="1" bind:value={settings.dreaming.max_pending_short_term_memories} /></label>
    <label>Maximum memories per cycle<input type="number" min="1" bind:value={settings.dreaming.max_short_term_memories_per_cycle} /></label>
  </div>

  <div style="border-top: 1px solid var(--color-border-default); margin-top: var(--space-3xl); padding-top: var(--space-xl);">
    <h3>Workflows</h3>
    {#each Object.entries(settings.workflows) as [name, workflow] (name)}
      <div class="card workflow-card">
        <header>
          <div>
            <span style="display:inline-block;width:8px;height:8px;border-radius:50%;background:{workflow.enabled ? 'var(--color-accent)' : 'var(--color-border-default)'};flex-shrink:0;"></span>
            <h4>{name.replaceAll("_", " ")}</h4>
          </div>
          <label class="toggle-label">
            <input type="checkbox" checked={workflow.enabled} onchange={(event) => updateWorkflow(name, { enabled: event.currentTarget.checked })} />
            <span class="toggle-track"><span class="toggle-thumb"></span></span>
          </label>
        </header>
        <div class="settings-grid">
          <label>Provider
            <select value={workflow.provider} onchange={(event) => updateWorkflow(name, { provider: event.currentTarget.value, model: "" })}>
              <option value="">Choose profile</option>
              {#each providers as provider (provider.id)}<option value={provider.id}>{provider.name} &middot; {provider.type}</option>{/each}
            </select>
          </label>
          <label>Model
            {#if modelsFor(workflow.provider).length}
              <select value={workflow.model} onchange={(event) => updateWorkflow(name, { model: event.currentTarget.value })}>
                <option value="">Choose model</option>
                {#each modelsFor(workflow.provider) as model (model)}<option value={model}>{model}</option>{/each}
              </select>
            {:else}
              <input value={workflow.model} placeholder="Model ID" oninput={(event) => updateWorkflow(name, { model: event.currentTarget.value })} />
            {/if}
          </label>
        </div>
      </div>
    {/each}
  </div>

  <label class="prompt">General prompt<textarea bind:value={settings.dreaming.general_prompt} rows="5"></textarea></label>

  {#if error}<p class="error-msg">{error}</p>{/if}
</section>
```

- [ ] **Step 2: Verify build**

```bash
bun run --cwd frontend build
```

- [ ] **Step 3: Commit**

```bash
git add frontend/src/web/components/DreamingEditor.svelte
git commit -m "feat: redesign DreamingEditor with workflow cards and toggle switches"
```

---

### Task 14: Redesign IngestEditor and ReleaseEditor

**Files:**
- Modify: `frontend/src/web/components/IngestEditor.svelte`
- Modify: `frontend/src/web/components/ReleaseEditor.svelte`

- [ ] **Step 1: Update IngestEditor.svelte**

Script block (lines 1-17) stays unchanged. Replace template (lines 19-29) with:

```svelte
<section class="settings-page" aria-label="Ingest settings">
  <header class="page-header">
    <div>
      <h2>Ingest</h2>
      <p>Set thresholds for incoming memory quality: sentence counts, symbol limits, block sizes.</p>
    </div>
    <button class="btn-primary" disabled={busy} onclick={() => onSave($state.snapshot(settings))}>Save ingest</button>
  </header>
  <div class="settings-grid">
    <label>Warning sentence count<input type="number" min="1" bind:value={settings.short_memory.warning_sentence_count} /></label>
    <label>Reject after sentences<input type="number" min="1" bind:value={settings.short_memory.rejection_sentence_count} /></label>
    <label>Warning symbol count<input type="number" min="0" bind:value={settings.short_memory.warning_symbol_count} /></label>
    <label>Reject after symbols<input type="number" min="0" bind:value={settings.short_memory.rejection_symbol_count} /></label>
    <label>Maximum learn block characters<input type="number" min="1" bind:value={settings.learn.max_block_chars} /></label>
  </div>
  {#if error}<p class="error-msg">{error}</p>{/if}
</section>
```

- [ ] **Step 2: Update ReleaseEditor.svelte**

Script block (lines 1-9) stays unchanged. Replace template (lines 11-15) with:

```svelte
<section class="settings-page" aria-label="Release settings">
  <header class="page-header">
    <div>
      <h2>Release</h2>
      <p>Choose between stable and development update channels.</p>
    </div>
    <button class="btn-primary" disabled={busy} onclick={() => onSave($state.snapshot(settings))}>Save release</button>
  </header>
  <fieldset>
    <legend>Update channel</legend>
    <label class="radio-label">
      <input type="radio" bind:group={settings.update_channel} value="stable" />
      Stable — published releases only
    </label>
    <label class="radio-label">
      <input type="radio" bind:group={settings.update_channel} value="dev" />
      Development — updates from main
    </label>
  </fieldset>
  {#if error}<p class="error-msg">{error}</p>{/if}
</section>
```

- [ ] **Step 3: Verify build**

```bash
bun run --cwd frontend build
```

- [ ] **Step 4: Commit**

```bash
git add frontend/src/web/components/IngestEditor.svelte frontend/src/web/components/ReleaseEditor.svelte
git commit -m "feat: redesign IngestEditor and ReleaseEditor with new form patterns"
```

---

### Task 15: Remove old styles.css and final verification

**Files:**
- Modify: `frontend/src/web/main.ts`
- Delete: `frontend/src/web/styles.css`

- [ ] **Step 1: Remove the old CSS import from main.ts**

Edit `frontend/src/web/main.ts` to remove the reference (if it was left as a safety net). The current state should already not import `styles.css` — verify:

```bash
grep "styles.css" frontend/src/web/main.ts
```

If the import still exists, remove it. The file should only import `fonts.css`, `tokens.css`, `base.css`, and `components.css`.

- [ ] **Step 2: Delete styles.css**

```bash
rm frontend/src/web/styles.css
```

- [ ] **Step 3: Run production build**

```bash
bun run --cwd frontend build
```

Expected: Build succeeds with no CSS import errors. Output in `frontend/dist/`.

- [ ] **Step 4: Verify Python backend unchanged**

```bash
uv run pytest
```

Expected: All Python tests pass — no backend changes in this plan.

- [ ] **Step 5: Commit**

```bash
git rm frontend/src/web/styles.css
git add frontend/src/web/main.ts
git commit -m "feat: remove old styles.css — migration to modular CSS complete"
```

---

### Post-Migration: Visual Verification Checklist

After all tasks complete and the build passes, manually verify in browser:

- [ ] Open `hiero admin` — Overview page loads with editorial split, stats cards, theme toggle
- [ ] Click theme toggle — switches between dark and light, persists on reload
- [ ] Navigate to Memory Views — table styled, tabs work, select a row shows detail panel
- [ ] Navigate to Providers — table styled, "New provider" opens slide-out editor with backdrop
- [ ] Navigate to Dreaming — toggle switches work, workflow cards styled
- [ ] Navigate to Ingest — form grid styled, inputs 16px
- [ ] Navigate to Release — radio buttons styled, fieldset styled
- [ ] Resize to 360px width — all pages collapse to single column, no horizontal overflow
- [ ] Test keyboard — Tab through all interactive elements, focus rings visible
- [ ] Test toast — trigger a save, verify toast slides in and auto-dismisses
- [ ] Test `prefers-reduced-motion` — verify all animations are instant

---
