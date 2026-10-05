# 0006 — Alpha versioning and release authority

Status: accepted. The current product remains on 0.x until Pavel explicitly
approves 1.0. Earlier premature 1.x tags and their remapping are historical.

## Decision

Keep package metadata, tags and update comparisons SemVer-compatible. Human-facing
identity can mark alpha status without changing machine version ordering.
No 1.0 release is authorized by a conventional commit or a green workflow alone.

Merging the maintained version PR is the publication decision for the current
0.x automation; see [automatic releases](../automatic-releases.md). No job auto-
approves or auto-merges that PR. Only open P0 issues block release; non-P0 failures
and missing optional evidence stay disclosed warnings with actionable issues.
Integrity and data-ownership safeguards remain runtime requirements.

## Consequences

Alpha releases can evolve without implying stable 1.x compatibility. Release
notes describe actual behavior and qualification limits. Version/tag identity and
retained artifact integrity are checked independently of optional native evidence.
