# Keep Semantic Releases on the 0.x Line

**Status:** Approved for implementation.

## Problem

Hieronymus is intentionally in alpha on the `0.x` version line. The release workflow asks `python-semantic-release` for the next version and then validates it with `hieronymus.release_guard`. In a checkout without release tags, `python-semantic-release` currently proposes `1.0.0`, so the guard correctly blocks the release.

The root cause is the library defaults: `allow_zero_version` is false, which promotes a tagless `0.0.0` base directly to `1.0.0`, and `major_on_zero` is true, which lets a breaking change promote an existing `0.x` release to `1.0.0`.

## Version Policy

Until a stable release is explicitly approved:

- semantic releases may create normal SemVer tags on the `0.x` line;
- feature changes increment the minor version when appropriate;
- fixes increment the patch version;
- breaking changes increment the minor version, for example `0.2.0` to `0.3.0`;
- no conventional commit may automatically create `1.0.0`.

The existing release guard remains the final enforcement boundary.

## Design

Configure `python-semantic-release` in `pyproject.toml` with:

```toml
allow_zero_version = true
major_on_zero = false
```

This uses the release tool's native policy controls. No version-rewriting wrapper, workflow branching, or artificial bootstrap tag is introduced. The release workflow continues to compute the next version, validate it before mutation, run semantic-release, validate the resulting metadata, and publish.

## Validation

Extend the release configuration tests to require both settings. Add an isolated tagless-repository regression that runs the configured semantic-release version calculation and proves it returns a `0.x` version rather than `1.0.0`. Preserve the existing guard and workflow tests.

Before completion, run the focused release tests and the repository verification required by `AGENTS.md`:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

## Scope

Expected production change: `pyproject.toml` only. Tests may change under `tests/` to lock the configuration and tagless calculation behavior. Release guard, workflow structure, package display conventions, and stable-release approval policy remain unchanged.
