## Hieronymus v0.9.0

Hieronymus stores several types of agent memory for writing projects. Connect
your agent by adding its MCP connection and Hieronymus skills, then keep writing
in your agent. The web interface lets you inspect memory, add pointers, and flag
stale or wrong entries. Local browser authentication is optional and off by default.

Exact native Linux x86_64, Windows x86_64, Apple Silicon and Intel macOS archives
share one pinned multilingual model archive. Install using the current standalone
desktop installer with the matching `release-<target>.json` and both archives.
The existing v0.8 single-archive updater cannot read v2 output; bootstrap with the
current installer. Legacy `release.json` is supported as input only.

Intel macOS is built but **unqualified for native desktop use**. Testing is
limited to available machines and sessions; a successful build does not establish
native acceptance. The attached `native-qualification.json` lists each session,
its actual passed checks, and explicit qualification gaps for these exact bytes.

Published bytes are the same retained candidates bound to those evidence records;
no rebuild occurs at promotion. Semantic inference remains mandatory. The
release uses an explicit unsigned/checksum waiver, with no claim of platform
signing, notarization, SBOM or signed provenance. Diagnostic symbols are supplied
only where actually generated. No deployed channel update feed is implied.
