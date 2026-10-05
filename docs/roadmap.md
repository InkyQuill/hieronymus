# Roadmap

Hieronymus is an alpha local memory service for writing and translation agents.
The runtime is Rust; the language rewrite is complete. Current work improves
behavior and operation against [the product principles](../PRODUCT.md).
This page contains unresolved work, not a parallel implementation log.

## Current priorities

- Improve recall, correction propagation and Dreaming with representative story
  work, preserving evidence, authority, uncertainty and user data.
- Qualify generated bundles and session lifecycle in actual installed
  Claude/Codex/Pi workflows. Keep zCode evidence separate and explicitly scoped;
  active Pi trusted ingress remains deferred. See [host checks](agent-host-acceptance.md).
- Complete native desktop observations where unavailable: login, real tray/panel,
  manager behavior, active update/rollback and uninstall. Intel runtime acquisition
  has reviewed pins; this does not qualify Intel desktop use. See [desktop checks](desktop-qualification.md).
- Measure full serving-path/build/artifact costs before further optimization.
  Exact SQLite vectors, static native icons and the local decision protocol are
  already implemented. Keep PDF extraction: disabling its current defaults
  removes no useful dependency. See [storage](adr/0013-semantic-index-and-platform-support.md)
  and [provider/model decisions](adr/0007-provider-catalog-and-workflow-assignments.md).
- Investigate inference/model/tokenizer footprint separately from storage.
  Preserve current pinned inference inputs until a measured replacement is accepted.

## Known business-logic limits

- Natural-language trusted correction input uses a limited grammar; unsupported
  phrasing stays tentative. A structured console route is available.
- Conceptless factual correction can remain tentative with `AmbiguousIdentity`;
  generic conceptless invalidation is not established behavior.
- Terminology substring matching does not define Unicode token boundaries or
  morphology. Define that policy before changing matching behavior; keep approved
  terminology deterministic.
- Small Jev pilots do not establish production calibration. Do not add automatic
  reranking, entity alignment, source selection, layout repair or support enforcement
  from those studies alone. The accepted limits are in ADR 0007.
- Preserve safe internal storage causes at decision-error boundaries without
  exposing private payloads or changing the public error contract.

These are follow-up scopes, not claims that historical failures still reproduce
on the current revision. Use existing issues for execution and reproduction details.

## Before an explicitly approved 1.0

Retire LanceDB transition compatibility only with a documented upgrade path for
older derived generations. Preserve authoritative SQLite data and generic
missing/corrupt-index recovery. Define cleanup without silently deleting user files
or restoring Lance dependencies. During 0.x, old derived artifacts remain intact.

Missing native evidence and other non-P0 findings are release warnings under
[AGENTS.md](../AGENTS.md#release-checks-and-blockers); integrity and data-ownership
safeguards remain runtime requirements. Completed investigations and old plans
belong in local archives/Git history, not this backlog.
