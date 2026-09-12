## Desktop Rust alpha

Exact native Linux x86_64, Windows x86_64, Apple Silicon and Intel macOS archives
share one pinned multilingual model archive. Install using the current standalone
desktop installer with the matching `release-<target>.json` and both archives.
The existing v0.8 single-archive updater cannot read v2 output; bootstrap with the
current installer. Legacy `release.json` is supported as input only.

Published bytes are the same retained candidates certified by native evidence;
no rebuild occurs at promotion. Semantic inference remains mandatory. The
release uses an explicit unsigned/checksum waiver, with no claim of platform
signing, notarization, SBOM or signed provenance. Diagnostic symbols are supplied
only where actually generated. No deployed channel update feed is implied.
