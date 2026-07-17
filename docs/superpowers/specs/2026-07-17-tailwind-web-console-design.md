# Tailwind Web Console Migration

## Goal

Migrate the Svelte 5 web console from component CSS to Tailwind CSS v4 while retaining its existing behavior, routes, API calls, local-first theme preference, and literary character. The visual direction is an editorial workbench: warm neutral surfaces, Literata display typography, restrained gold accents, and clear operational status.

## Architecture

- Add Tailwind v4 and the Vite integration to the frontend build.
- Replace the separate `tokens.css`, `base.css`, and `components.css` imports with one Tailwind entry stylesheet.
- Keep `fonts.css` solely for local font-face declarations.
- Define Tailwind tokens in CSS with `@theme`, mapping utility names to semantic CSS variables such as `--color-surface`, `--color-text`, and `--color-accent`.
- Preserve the existing `data-theme="light"` and `data-theme="dark"` attribute, early boot script, and theme-toggle module. The variables provide the runtime theme change; Tailwind utilities consume them.
- Remove the legacy component stylesheet once every Svelte component uses Tailwind classes.

## Interface

- Retain current information architecture: top navigation, administration overview, memory views, configuration sections, provider editor, and toast feedback.
- Use responsive utility layouts: a readable editorial lead/stage layout on wide screens, single-column stacking on narrow screens, and horizontally scrollable tables where necessary.
- Give all controls consistent 44px minimum targets, visible `:focus-visible` rings, disabled states, and 150–300ms reduced-motion-aware transitions.
- Keep native semantic elements, existing keyboard activation, modal focus management, labels, and live-region behavior.
- Use status color together with text labels; do not use decorative icons or emoji as structural controls.

## Component Migration

- `App.svelte` owns page shell, navigation, responsive spacing, and modal mounting.
- Dashboard and memory views use Tailwind grids, cards, tables, and status treatments without changing data loading or actions.
- Editors use shared visual conventions implemented with repeated Tailwind utility patterns, not a new component abstraction layer.
- The provider editor keeps the native `dialog` lifecycle and focus restoration already in place.

## Error Handling and Behavior Preservation

- No frontend API contract changes.
- Existing busy, error, confirmation, notification, and event-refresh behavior remains unchanged.
- `data-theme` remains the only theme state source; no duplicated dark-mode state or Tailwind configuration file is added.

## Validation

- Run frontend type checking, formatting, tests, and Vite build.
- Run the project Python checks required by `AGENTS.md` where frontend build setup affects repository verification.
- Inspect the rendered interface at desktop and mobile widths, including both themes, dialog focus behavior, keyboard selection, table overflow, and reduced-motion behavior.

## Non-goals

- No backend, API, route, or data-model changes.
- No new icon dependency, visual asset generation, or redesign of the product’s information architecture.
- No reusable component-library extraction beyond the Tailwind migration itself.
