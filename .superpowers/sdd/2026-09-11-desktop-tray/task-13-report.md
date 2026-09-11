# Task 13 report: share readiness with CLI and console

## Outcome

Implemented the bounded Task 13 integration on baseline
`65fc633243eabd4755daefaf5520198304108127`.

- `hiero status` human output now displays the daemon-owned readiness level,
  exact aggregate reasons, and each provider's explicit
  `Untested`/`Healthy`/`Failed` condition. It deserializes only the established
  readiness DTO and never formats arbitrary response objects.
- `hiero status --json` retains the complete authenticated status value,
  including the full additive readiness DTO.
- The authenticated browser-session dashboard response now contains the same
  live readiness snapshot used by native status. No browser bearer token or
  independent provider probe was added.
- The console validates the current unversioned readiness shape at its
  presentation boundary and shows `Ready`, `Degraded`, `Starting`, or
  `Unknown`. It displays the daemon's reasons verbatim and does not derive
  aggregate health from `dream_status`.
- The existing Local service area gained the readiness summary and provider
  rows. No surrounding console flow or visual structure was redesigned.

## DTO policy

The reviewed Task 1 DTO is intentionally unversioned. A value is recognized
only when it has a known level, string-array `reasons`, an array of completely
valid provider entries, and no explicit `schema_version` marker. Missing,
malformed, unknown-level, or explicitly version-marked values are `Unknown`,
never green. Additive unknown fields do not enter human output. Provider
capabilities remain optional because the Rust DTO has `#[serde(default)]`.

This preserves compatibility with the current daemon and the required
unversioned degraded fixture. A future schema version needs an explicit parser
migration before either human client accepts it.

## Authentication and authority

`/status` requires the native bearer credential, while the web console holds
only an HttpOnly browser session. Fetching `/status` directly from frontend
code would cross the established authentication boundary. Instead,
`GET /api/admin/dashboard`, already guarded by valid Host, browser session, and
Origin, adds `runtime.dream.readiness().snapshot_with_semantic(...)`. A
regression compares this field with the authenticated native status snapshot
and proves an unauthenticated dashboard response returns no readiness data.

No historical dreaming errors, network probes, model calls, real daemon,
login, book workspace, or user runtime data were used.

## Files changed

- `crates/hiero/src/lifecycle.rs`
  - validates the current readiness DTO for human status;
  - renders safe line-oriented level/reason/provider fields;
  - replaces control characters before terminal presentation.
- `crates/hiero/src/daemon/rest/admin.rs`
  - adds authoritative runtime readiness to the authenticated dashboard.
- `crates/hiero/tests/readiness_contract.rs`
  - covers CLI known/unknown presentation, additive JSON preservation,
    unsupported explicit schema markers, secret-like unknown fields, and the
    authenticated console data path.
- `crates/hiero/tests/daemon_rest_routes.rs`
  - records readiness as the one additive dashboard field while retaining the
    frozen oracle for every existing field.
- `frontend/src/web/lib/types.ts`
  - adds the current readiness DTO types and the optional untrusted dashboard
    boundary field.
- `frontend/src/web/lib/readiness.ts` (new)
  - validates and formats readiness into display-only known fields.
- `frontend/src/web/lib/readiness.test.ts` (new)
  - covers missing/malformed/unknown/version-marked data, untested providers,
    exact degraded semantic reason, and omission of unknown secret-like data.
- `frontend/src/web/components/AdminDashboard.svelte`
  - consumes formatted readiness in the existing Local service section.
- `.superpowers/sdd/2026-09-11-desktop-tray/task-13-report.md` (new)
  - this implementation and verification record.

## TDD evidence

Rust CLI RED:

```text
source .superpowers/sdd/2026-09-11-desktop-tray/environment.sh
cargo test -p hiero --locked --test readiness_contract lifecycle_human_status -- --nocapture

2 tests failed
human output ended after `data root` and contained no readiness line
```

Authenticated dashboard RED:

```text
source .superpowers/sdd/2026-09-11-desktop-tray/environment.sh
cargo test -p hiero --locked --test readiness_contract \
  authenticated_console_dashboard_carries_daemon_readiness -- --exact --nocapture

FAILED: dashboard["readiness"].is_object()
```

The first frontend RED exposed the missing production module. That initial
attempt also used the repository-relative source path from inside `frontend`
and therefore logged a failed `source`; all subsequent Bun evidence used the
correct `../.superpowers/.../environment.sh` path. A correctly sourced mutation
check changed the degraded label to Ready and proved the exact dashboard
regression fails for the intended behavior:

```text
source ../.superpowers/sdd/2026-09-11-desktop-tray/environment.sh
bun run test -- src/web/lib/readiness.test.ts \
  -t "dashboard displays the server's exact degraded semantic reason"

1 failed: Unable to find an element with the text: Degraded
```

After restoring the production mapping, the same command passed: `1 passed`,
`8 skipped`.

Focused GREEN:

```text
cargo test -p hiero --locked --test readiness_contract -- --nocapture
16 passed; 0 failed

bun run test -- src/web/lib/readiness.test.ts
9 passed; 0 failed

cargo test -p hiero --locked --test daemon_rest_routes \
  api_admin_dashboard_matches_frozen_target_shape -- --exact --nocapture
1 passed; 0 failed

cargo test -p hiero --locked --test runtime_shutdown hiero_status_ -- --nocapture
2 passed; 0 failed
```

## Validation

Every Cargo/Bun command after the noted initial path mistake sourced
`.superpowers/sdd/2026-09-11-desktop-tray/environment.sh`; Cargo used the shared
target with two jobs and serialized invocations.

```text
npx @sveltejs/mcp svelte-autofixer \
  ./src/web/components/AdminDashboard.svelte --svelte-version 5
issues: []; suggestions: []

bun run typecheck
PASS (application and test TypeScript configurations)

bun run test
9 files passed; 85 tests passed

bun run build
PASS; 124 modules transformed

cargo clippy -p hiero --all-targets --all-features --locked -- -D warnings
PASS

cargo fmt --all -- --check
PASS

git diff --check
PASS
```

Task 14 owns the full repository matrix and embedded console/artifact hash
rebuild, so this task did not repeat those package-wide qualification steps.

## Self-review

- Authority: both CLI and console consume the daemon's `ReadinessSummary`; no
  competing aggregate or provider probe exists.
- Compatibility: the unversioned current DTO remains valid; malformed and
  unsupported marked shapes fail closed.
- Security: browser readiness remains behind the existing session guard;
  human presentation uses only typed fields and normalizes terminal control
  characters. Existing daemon-side secret redaction remains the source of the
  JSON contract.
- Regression scope: the historical dashboard oracle remains exact for all
  pre-existing fields and records only the approved additive readiness field.
- Scope: changes are limited to CLI human status, authenticated dashboard
  transport, readiness formatting/rendering, and their regression tests.
- External gap: no native host, installed artifact, or real model acceptance is
  claimed; Task 14 retains those broader qualification responsibilities.
