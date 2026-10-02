# Hieronymus icon sources

`icon-color.svg` and `icon-mono.svg` are the unmodified originals supplied for
the Folio H artwork. Their SHA-256 checksums are:

```text
fa543f2ee4f1ab2acedc951e782c683f65b0b353db1deee368d0116fa62d8d08  icon-color.svg
78eff50b0e91b1a6e9b53c73642f0fc13a2eaaa80f47a71dd73e789ab86968a5  icon-mono.svg
```

Both originals use `viewBox="0 0 75 75"` and the same two path geometries.
The color source assigns copper `#a45734` to the center path and navy
`#102134` to the outer path. `icon-tray.svg` preserves those paths, gives each
region an explicit ID, and makes the outer foreground `currentColor` so native
adapters can resolve it against the panel appearance. The center region is
replaced with the current semantic status accent during rasterization.

## Native assets and UI icons

The console uses individually imported `@lucide/svelte` components (ISC license):
https://lucide.dev/guide/svelte/getting-started . Do not import a dynamic icon catalog.
The supplied Folio H logo remains the application identity.

`native/masks/*.png` freezes that logo's raster geometry at supported native sizes.
White pixels identify foreground coverage, black identifies the status accent,
and alpha preserves the original silhouette. Runtime code only decodes PNG and
recolors the two regions; it no longer parses/renders SVG. `native/hieronymus.iconset`
contains the ready-made macOS package images. Ordinary builds do not generate icons.

These assets were exported from the unchanged `icon-tray.svg` using the former
resvg 0.48.1 renderer: foreground white/accent black for masks, foreground #102134
and accent #a45734 for the macOS iconset. If the brand geometry changes, export the
replacement sizes with an external graphics tool and run the native icon tests.
Keep the original SVGs and their checksums. Do not add an image toolchain to the
application just to regenerate static assets on every build.
