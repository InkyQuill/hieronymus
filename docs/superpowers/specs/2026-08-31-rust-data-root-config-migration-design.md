# Rust Data-Root And Configuration Migration Design

**Status:** Proposed for review on 2026-08-31.

## Goal

Preserve the existing isolated data root and migrate authoritative local files
without losing credentials, workflow assignments, permissions, or generated
agent integrations.

## Owned Paths

The selected root remains the current default `~/.config/hieronymus`, overridden
as a unit by `--data-root` or `HIERONYMUS_DATA_ROOT`. Rust keeps these paths:

- `hieronymus.sqlite`: authoritative database, owned by the database spec;
- `provider.conf`: provider profiles, defaults, endpoints, timeouts, and keys;
- `dream.conf`: dream settings, prompts, caps, and workflow assignments;
- `ingest.conf`: short-memory and Learn ingestion policy;
- `release.conf`: update channel state until replaced by a later ADR;
- `llmcache.tmp`: derived provider-model cache, safe to invalidate;
- `backups/`: immutable migration backups and receipts;
- `agent-plugins/`: generated local integration artifacts.

The Rust cutover does not introduce an XDG data/config split, rename
`hieronymus.sqlite`, or create a second default root.

## Preflight

Read-only preflight inventories every known file, parses authoritative TOML,
records permission/ownership state, detects legacy `dream.conf.providers`,
validates workflow references, identifies unknown keys, checks generated-plugin
targets, and reports whether cache invalidation is required. It never prints
secret values; values enter the report only as `configured: true|false`.

Missing optional files use documented defaults. Invalid authoritative config,
unsafe credential permissions, a provider-id collision, or an enabled workflow
that cannot resolve fails closed before database mutation.

## Conversion

ADR 0007 remains authoritative. If legacy provider blocks exist under
`dream.conf.providers`, the converter:

1. derives stable provider ids and detects collisions;
2. writes profiles and exact keys into the staged `provider.conf`;
3. rewrites workflow provider references in staged `dream.conf`;
4. validates every enabled workflow against the staged catalog;
5. removes legacy provider blocks only from the staged dream file;
6. preserves existing `provider.conf` entries unless the legacy value is
   identical or the report contains an explicit conflict resolution.

Unchanged authoritative files remain byte-identical. Converted TOML is edited
with `toml_edit` so comments, ordering, and unknown supported keys survive.
`ingest.conf` and `release.conf` round-trip through typed schemas.
`llmcache.tmp` is deleted or ignored when its schema/provider identity is
incompatible. Generated plugin files are not
trusted as configuration input; they are regenerated after daemon health using
the versioned discovery contract.

## Atomic File Protocol

All target config files are rendered before database mutation into a sibling
staging directory with user-only permissions for secret-bearing files, parsed
back, cross-validated, fsynced, and checksummed. The shared cutover journal
records those checksums as `prepared`. Database conversion then commits before
config promotion.
Promotion takes the data-root ownership lock, renames each original to the
immutable backup set, then atomically renames staged files into place and fsyncs
the directory. The journal moves through `database_committed` to `complete`; a
receipt lists old/new checksums and schema versions.

Failure before the database commit restores the original file set and may delete
staging. Failure after the database commit retains verified staging and marks
`config_promotion_required`; the daemon refuses startup. Rerunning the migration
verifies checksums and resumes promotion without rerunning database conversion.
After successful one-way cutover, Python config rollback is unsupported; Rust
recovery can rebuild staged current-format files from the immutable backup.

## Secret Handling

Provider keys are parsed directly into `Secret<String>`. Only the config writer
and outbound provider header builder may expose them. Migration reports, diffs,
errors, tracing, audit, CLI JSON, MCP, HTTP, WebSocket, and frontend DTOs use
redacted projections. Sentinel-secret tests cover success and every failure
branch of preflight, conversion, and promotion.

## Acceptance Criteria

- Default and overridden roots remain fully isolated.
- Current config round-trips without semantic change.
- ADR 0007 legacy provider migration preserves exact keys and workflow routing
  or fails before mutation on collision.
- Invalid config prevents database mutation and daemon startup.
- Failure injection before database commit restores the original files; failure
  after commit produces resumable `config_promotion_required`, never a running
  daemon with mismatched database/config versions.
- Cache invalidation never deletes authoritative configuration.
- Generated plugins are refreshed only after the authenticated Rust daemon is
  healthy.
- No sentinel secret appears in reports, errors, logs, receipts, or DTOs.
