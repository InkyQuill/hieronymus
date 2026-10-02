---
name: "Hieronymus — Thoth"
description: "Warm, restrained memory console for authors and literary translators."
colors:
  light-background: "#f7f3ed"
  light-surface: "#fffdf9"
  light-surfaceRaised: "#f1eae0"
  light-foreground: "#29231d"
  light-muted: "#65594c"
  light-border: "#e5d9ca"
  light-accent: "#9b4c13"
  light-keyword: "#78567d"
  light-string: "#547241"
  light-number: "#8b620d"
  dark-background: "#211d19"
  dark-surface: "#29231e"
  dark-surfaceRaised: "#352d25"
  dark-foreground: "#f7f0e7"
  dark-muted: "#c9b9a7"
  dark-border: "#4a3d31"
  dark-accent: "#e2a46f"
  dark-keyword: "#d2a3d2"
  dark-string: "#b8c990"
  dark-number: "#e8c97a"
  light-danger: "#b83030"
  dark-danger: "#ff8a8a"
  light-success: "#3a6e45"
  dark-success: "#5ea870"
typography:
  display:
    fontFamily: "Literata, Georgia, serif"
    fontSize: "2rem"
    fontWeight: 400
    lineHeight: 1.2
  headline:
    fontFamily: "Literata, Georgia, serif"
    fontSize: "1.5rem"
    fontWeight: 400
    lineHeight: 1.25
  title:
    fontFamily: "Geist, sans-serif"
    fontSize: "15px"
    fontWeight: 600
    lineHeight: 1.3
  body:
    fontFamily: "Geist, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.55
  body-small:
    fontFamily: "Geist, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.5
  caption:
    fontFamily: "Geist, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.5
  label:
    fontFamily: "Geist, sans-serif"
    fontSize: "10px"
    fontWeight: 500
    lineHeight: 1
    letterSpacing: "0.12em"
  mono:
    fontFamily: "Inconsolata LGC, Inconsolata, monospace"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.5
rounded:
  control: "4px"
  panel: "6px"
spacing:
  compact: "8px"
  control: "12px"
  standard: "16px"
  panel: "20px"
  section: "24px"
  major: "32px"
  wide: "48px"
components:
  button-primary:
    backgroundColor: "{colors.light-accent}"
    textColor: "{colors.light-background}"
    rounded: "{rounded.control}"
    padding: "8px 16px"
    height: "44px"
  button-secondary:
    textColor: "{colors.light-foreground}"
    rounded: "{rounded.control}"
    padding: "8px 16px"
  input:
    backgroundColor: "{colors.light-surface}"
    textColor: "{colors.light-foreground}"
    rounded: "{rounded.control}"
    padding: "8px 12px"
  panel:
    backgroundColor: "{colors.light-surface}"
    rounded: "{rounded.panel}"
    padding: "20px"
---

# Hieronymus Design System

## Overview

**Creative North Star: "The Warm Memory Desk"**

A quiet reading desk for inspecting agent memory. Warm paper and brown-black surfaces support precise tables and plain-language settings; serif headings give the console an editorial identity without turning controls into book typography.

The approved Thoth palette supplies color; the existing Hieronymus implementation supplies typography, layout, interaction and component shapes. This is a color adaptation and documentation of the incumbent interface, not a new component system.

**Key Characteristics:**
- Warm paired themes with one copper-amber accent.
- Editorial headings, compact sans-serif controls and monospaced source data.
- Flat bordered surfaces; details appear when the author needs them.

Product truth lives in [PRODUCT.md](PRODUCT.md). Runtime sources are [app.css](frontend/src/web/app.css), [fonts.css](frontend/src/web/fonts.css) and [components](frontend/src/web/components). The token frontmatter records approved values; keep it synchronized with runtime CSS when changing the system.

## Colors

Primary: Thoth copper-amber is used for primary actions, links, focus and selection. Neutral: background is the page canvas; surface contains forms and details; surfaceRaised marks hover, selection and grouped controls. Foreground and muted are the two text strengths; tertiary currently aliases muted.

Frontmatter keys are paired by light/dark prefix. Component frontmatter illustrates light-mode roles; runtime semantic variables select the matching dark roles automatically. Dark mode is warm brown-black, not pure black. Light surfaces are brighter than the canvas: preserve this deliberate Thoth layering.

**The Semantic Color Rule.** Use Hieronymus role tokens in components; map those roles to Thoth in the theme layer. Never scatter palette hex values across components.

Thoth's keyword, string and number tokens are available for syntax only; they do not define success or error. Existing Hieronymus danger/success colors remain local extensions. Overlay, stronger borders, soft accent and accent tint are derived with color-mix in app.css. Do not treat illustrative sidecar tonal ramps as additional runtime tokens.

Provenance: [Thoth palette snapshot](frontend/src/web/theme/thoth-palette.json), source repository /home/inky/Development/thoth-palette, revision 04bdc30dadf1ee11832fdefa190429caeeb07203, MIT [license](frontend/src/web/theme/THOTH-LICENSE). Values are copied unchanged from the web palette; only CSS selectors are adapted to data-theme. No build-time dependency on that external checkout. To update, review the upstream palette, refresh the snapshot and thoth.css together, update this document and its sidecar, and verify both themes.

## Typography

**The Three Voices Rule.** Use Literata for page and section headings, Geist for interaction and explanations, and Inconsolata LGC for code, identifiers and structured data.

Fonts are bundled WOFF2 assets with font-display: swap. Display and headline roles are regular serif; title is semibold sans. Body, small body and caption follow the frontmatter scale. The small uppercase label role is for table headings and metadata, not a decorative pre-heading. Preserve full fallback stacks in fonts.css/app.css. Explanatory paragraphs generally use max-w-prose or a 70ch cap; data tables may be wider.

## Layout

The shell uses the available width, with horizontal padding stepping from 16px to 32px to 48px. Main content has 24px vertical padding. Spacing follows the extracted scale; component groups use 8–16px and section gaps use 24–32px.

Custom breakpoints are xs 480px, sm 720px, md 960px and lg 1080px. Tailwind's inherited xl is 1280px. At lg, overview separates context from status and memory uses a list/detail grid (1.45fr / .8fr, with a 20rem detail minimum). Below this, detail stacks. Memory navigation becomes a select below sm; configuration navigation wraps. Tables scroll inside their container instead of widening the page. The provider table has a 42rem minimum width.

## Elevation & Depth

**The Flat Surface Rule.** Use borders and surface tones for ordinary grouping. Reserve the existing offset shadow for the modal editor drawer.

The provider editor sits at the right edge, up to 420px wide and 100dvh tall, with 24px padding and a -16px 0 40px rgb(0 0 0 / 15%) shadow. Its existing one-pixel boundary separates it from the scrim. Ordinary tables, settings and status panels remain flat.

## Shapes

Controls have small corners; panels are only slightly softer. Use the frontmatter radii, thin borders and rectangular geometry. There is no universal pill-card treatment. Preserve the existing book mark and authored SVG controls.

## Components

### Buttons and fields

Primary actions use accent with root-colored text; secondary actions use neutral or accent outlines and raised hover backgrounds. Controls commonly have a 44px minimum height. Inputs use surface fill and visible borders. Disabled controls retain their existing reduced opacity and native disabled behavior. Focus is a 2px accent outline offset by 2px. Errors pair color with explanatory text; destructive actions require the existing confirmation.

### Navigation and memory table

Main navigation names Connect, Overview, Memory and Settings; settings have their own local navigation. Selected items use a filled raised surface and aria-current. Memory rows lead to a bordered detail panel with provenance, corrections and more actions under disclosure. Keep deep links, filtering and pagination intact.

### Status panels and disclosures

Overview panels group operational status, without ornamental charts. TechnicalDetails keeps structured diagnostics available with copying. SettingsSaveState communicates unsaved and saved state near the relevant action. Loading, empty and error states explain what happened and what to do next.

### Motion

Existing fade and drawer entrances take 200ms; toast entrance takes 250ms. Drawer/toast easing is cubic-bezier(0.32, 0.72, 0, 1). Reduced-motion CSS suppresses animation and transitions. Preserve this restrained vocabulary; no new decorative animation is implied by the palette.

## Do's and Don'ts

### Do
- Do support both themes and preserve the saved theme preference.
- Do keep technical detail available through disclosure while leading with author-facing language.
- Do check text contrast on root, surface, raised and accent backgrounds.
- Do reuse the existing spacing, type utilities and semantic color roles.

### Don't
- Don't use syntax colors as arbitrary UI status colors.
- Don't introduce a second component library or remote font dependency for visual consistency.
- Don't replace real provenance or readiness information with decorative metrics.
- Don't make a subtle border the only indication of an interactive control or selected state.
