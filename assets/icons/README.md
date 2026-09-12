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
