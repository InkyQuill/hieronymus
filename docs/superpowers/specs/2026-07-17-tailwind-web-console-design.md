# Tailwind Web Console Migration

## Goal

Migrate the Svelte 5 web console from component CSS to Tailwind CSS v4 while retaining its existing behavior, routes, API calls, local-first theme preference, and literary character. The visual direction is an editorial workbench: warm neutral surfaces, Literata display typography, restrained gold accents, and clear operational status.

## Architecture

- Install `tailwindcss` and `@tailwindcss/vite` with `bun add -D`, then add `tailwindcss()` to `vite.config.ts` alongside the Svelte plugin. Tailwind v4's automatic source detection must detect the `.svelte` files; add an explicit `@source` only if the build proves otherwise.
- Replace the separate `tokens.css`, `base.css`, and `components.css` imports with one Tailwind entry stylesheet.
- Keep `fonts.css` solely for local font-face declarations.
- Define Tailwind tokens in CSS with `@theme`, mapping utilities to the complete semantic variable set: root/surface/raised/overlay backgrounds; default/strong/accent borders; primary/secondary/tertiary text; accent/text/soft/background; danger/text/background; and success/text/background.
- Preserve the existing `data-theme="light"` and `data-theme="dark"` attribute, early boot script, and theme-toggle module. The variables provide the runtime theme change; Tailwind utilities consume them.
- Remove the legacy component stylesheet once every Svelte component uses Tailwind classes.

## Dark Mode and Tokens

- Use Tailwind v4's selector-based variant, not the default media-query variant: `@custom-variant dark (&:where([data-theme="dark"], [data-theme="dark"] *));`.
- Keep all light and dark semantic values in the corresponding `data-theme` selectors. Tailwind `--color-*` tokens reference these runtime variables so utilities respond immediately to the persisted preference.
- Define custom responsive tokens in `@theme` to preserve the established layout behavior: `xs` at 30rem (480px), `sm` at 45rem (720px), `md` at 60rem (960px), and `lg` at 67.5rem (1080px). The 1050px editor adjustment can use an arbitrary breakpoint only where it remains necessary after migration.

## Typography and Motion

- Preserve the editorial type scale with v4-native `@utility` directives for `text-display`, `text-h2`, `text-h3`, `text-body`, `text-body-sm`, `text-mono`, `text-eyebrow`, and `text-caption`. Each emits the current font shorthand so the clamp-based display scale and line-height tuning remain exact.
- Retain font loading and `font-display: swap` in `fonts.css`.
- Port `fade-in`, `slide-in`, and `toast-in` into `@theme inline` animation tokens. Apply `motion-reduce:animate-none` and transition fallbacks for reduced-motion users.

## Interface

- Retain current information architecture: top navigation, administration overview, memory views, configuration sections, provider editor, and toast feedback.
- Use responsive utility layouts: a readable editorial lead/stage layout on wide screens, single-column stacking on narrow screens, and horizontally scrollable tables where necessary.
- Give all controls consistent 44px minimum targets, visible `:focus-visible` rings, disabled states, and 150–300ms reduced-motion-aware transitions.
- Keep native semantic elements, existing keyboard activation, modal focus management, labels, and live-region behavior.
- Use status color together with text labels; do not use decorative icons or emoji as structural controls.

## Component Migration

- `App.svelte` owns page shell, navigation, responsive spacing, and modal mounting.
- Dashboard and memory views use Tailwind grids, cards, tables, and status treatments without changing data loading or actions.
- Use inline utilities for ordinary layout, spacing, cards, buttons, tabs, and editor sections. Do not introduce a Svelte component library.
- Keep a deliberately small `@layer components`/`@utility` layer for patterns that would otherwise require unreadable descendant selectors: table row cell states and selection borders, the peer-driven toggle track/thumb, dialog panel/backdrop treatment, and baseline form-control normalization. Use `sr-only` instead of the legacy `visually-hidden` helper.
- Standardize interactive targets with `min-h-11` and only add `min-w-11` where a compact control has no text label.
- The provider editor keeps the native `dialog` lifecycle and focus restoration already in place.

## Error Handling and Behavior Preservation

- No frontend API contract changes.
- Existing busy, error, confirmation, notification, and event-refresh behavior remains unchanged.
- `data-theme` remains the only theme state source; no duplicated dark-mode state or Tailwind configuration file is added.

## Validation

- Run `bun run typecheck`, `bun run format`, `bun run test`, and `bun run build` from `frontend/`.
- Run the project Python checks required by `AGENTS.md`. This is coupled to frontend delivery: packaging includes `frontend/dist`, and the hatch build command runs the Bun frontend build before `uv build`.
- Inspect the rendered interface at desktop and mobile widths, including both themes, dialog focus behavior, keyboard selection, table overflow, and reduced-motion behavior.

## Non-goals

- No backend, API, route, or data-model changes.
- No new icon dependency, visual asset generation, or redesign of the product’s information architecture.
- No reusable component-library extraction beyond the Tailwind migration itself.
