# Rust Distribution And Cutover Design

**Status:** Proposed for review on 2026-08-31.

## Goal

Build, install, upgrade, and if necessary roll back a self-contained Rust release
without requiring language runtimes on the user's machine.

## Build Ownership

The Cargo workspace is virtual. Package build scripts live in a package, not at
the virtual workspace root. Frontend production assets are built in an explicit
release orchestration step before compiling `hiero-bin`; the binary package
embeds the resulting directory. Ordinary library tests do not invoke Bun.

CI pins Rust and Bun versions and uses the repository lockfiles. A missing
frontend bundle is allowed only for explicitly frontend-free developer targets;
release builds fail if required assets are absent. Embedded-asset tests verify
index fallback, MIME type, hashed assets, and absence of source-map secrets.

Native release jobs run on each supported target. The pipeline produces the
binary/archive, checksums, software bill of materials, build provenance, and
signed release metadata. Static TLS configuration avoids a system OpenSSL
dependency, but native semantic dependencies are validated separately.

## Installer

The bootstrap installer:

1. resolves a supported OS/architecture without executing downloaded content;
2. downloads signed release metadata and the matching archive;
3. verifies checksum and signature;
4. installs atomically into a versioned application directory;
5. updates the stable command link;
6. installs/updates the per-user daemon service;
7. runs non-mutating doctor checks;
8. starts the daemon only when the existing schema is already compatible.

If database upgrade is required, installation completes but the daemon remains
stopped until the user runs or confirms the separately reported migration. The
installer never performs a hidden destructive schema upgrade.

No Python, Node, or Bun is required on the target machine. Uninstall removes
the binary, service definition, and generated integration entries only after
confirmation; it preserves databases, configuration, models, backups, and
audit data by default.

## Update

Update downloads alongside the current version, verifies it, checks protocol
and schema compatibility, stops the service, switches the version link, and
starts/health-checks the new daemon. If health fails before a schema upgrade,
the link returns to the prior binary. After a schema upgrade, rollback follows
the database backup procedure rather than launching an older binary against a
newer schema.

Agent integration generation uses discovered endpoints and supported transport
capabilities. Update refreshes generated entries only after the new daemon is
healthy and keeps backups of modified host configuration.

## Release Rehearsal

For each release candidate, CI or a controlled matrix performs:

- clean install and first launch;
- install over the last Python managed release;
- migration dry-run and confirmed upgrade on representative copied databases;
- CLI, MCP HTTP, MCP stdio, web, dreaming fake-provider, FTS, and semantic smoke
  tests;
- forced daemon crash and restart;
- failed update before schema change;
- full restore of the pre-upgrade database and Python release;
- uninstall with user-data preservation.

## Support Matrix

A target is listed as supported only when its native release rehearsal passes.
Targets that compile but fail lifecycle or semantic tests are experimental and
do not receive the default installer path. The published matrix records whether
semantic retrieval is native, optional, or FTS-only.

## Acceptance Criteria

- Release artifacts run on clean machines without Python, Node, or Bun.
- Checksums/signatures and atomic install/update failure paths are tested.
- A required database upgrade is always explicit and backed up.
- Failed pre-schema updates restore the previous binary automatically.
- Post-schema rollback is documented and rehearsed with the recorded backup.
- Uninstall preserves user data unless a separate explicit delete-data action
  identifies the exact target.
