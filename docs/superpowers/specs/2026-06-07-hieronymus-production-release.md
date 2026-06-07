# Hieronymus Production Release Scaffolding

**Date:** 2026-06-07
**Status:** Approved for planning

## Purpose

Hieronymus needs a production release path that lets users install, update, and remove the app with
small predictable commands while keeping the CLI entrypoints `hiero` and `hieronymus` as the main
operator interface.

The first release scaffold should prioritize deterministic installs from GitHub tags over package
registry publishing. It should be easy to move to PyPI later without changing the user-facing CLI
contract.

## GitHub Repository

The canonical upstream repository is:

```text
https://github.com/InkyQuill/hieronymus.git
```

The local repository should add this as `origin` and push `main` after the release scaffold lands.
The GitHub repository is currently empty, so the first push will establish the default branch and
workflow files.

## Installation Model

Use a self-managed checkout plus `uv tool install` behind the scenes.

Default paths:

- managed checkout: `~/.local/share/hieronymus/app`
- user data/config: existing Hieronymus config resolution, normally `~/.config/hieronymus`
- command shims: wherever `uv tool install` places user tools, normally `~/.local/bin`

The public installer should be a one-liner:

```bash
curl -fsSL https://raw.githubusercontent.com/InkyQuill/hieronymus/main/install.sh | sh
```

Installer behavior:

1. Require `git`.
2. Install `uv` only if it is missing, using Astral's official installer.
3. Clone the GitHub repo into the managed checkout, or fetch it if the checkout already exists.
4. Check out the latest stable GitHub tag. If no tag exists yet, check out `main`.
5. Run `uv tool install --force <managed-checkout>` so `hiero`, `hieronymus`,
   `hieronymus-mcp`, and `hieronymus-agent-hook` are callable from the user's shell.
6. Print the installed version, install path, and PATH guidance if the tool directory is not on
   `PATH`.

The installer must be idempotent. Re-running it should update the managed checkout and reinstall the
console scripts.

## Removal Model

Provide an `uninstall.sh` script that can be run through a one-liner or from the managed checkout.

Removal behavior:

1. Run `uv tool uninstall hieronymus` when `uv` is available.
2. Remove the managed checkout under `~/.local/share/hieronymus/app`.
3. Ask interactively whether to remove settings and data under the resolved config/data root.
4. Support a non-interactive flag for automation:
   - `--keep-data`: remove the app but keep settings/data.
   - `--purge-data`: remove the app and settings/data without prompting.

The removal script must not delete translation workspaces. It only removes Hieronymus-owned install
and config/data paths.

## In-Place Updates

Add a CLI command:

```bash
hiero update
```

Default behavior:

1. Determine the current installed version using package metadata.
2. Fetch the latest stable GitHub tag from `https://github.com/InkyQuill/hieronymus.git`.
3. If the current version is already latest, print a concise up-to-date message and exit zero.
4. If a newer version exists, update the managed checkout to that tag and run
   `uv tool install --force <managed-checkout>`.
5. Print old version, new version, and the updated checkout path.

Options:

- `hiero update --check`: report whether an update is available without changing files.
- `hiero update --json`: print machine-readable status.
- `hiero update --target main`: allow developer installs to update to `main` instead of the latest
  tag.

If Hieronymus was not installed through the managed checkout, `hiero update` should fail cleanly with
instructions to run the installer. It should not guess at unrelated package managers or mutate an
unknown installation.

## Update Checks

Add a small release module responsible for:

- parsing semantic versions and tags of the form `vMAJOR.MINOR.PATCH`;
- discovering the managed checkout path;
- reading the current package version;
- querying GitHub tags via `git ls-remote --tags`;
- selecting the highest stable semantic version;
- reporting update status without side effects.

The update check should ignore prerelease tags in the first release scaffold. Prerelease support can
be added later with an explicit flag.

## Semantic Release

Use GitHub Actions for release automation on pushes to `main`.

Workflow behavior:

1. Run `uv run pytest`, `uv run ruff check .`, and `uv run ruff format --check .`.
2. Use Conventional Commits to determine the next semantic version:
   - `fix:` increments patch.
   - `feat:` increments minor.
   - breaking changes increment major.
3. Update `pyproject.toml` version.
4. Create/update changelog or GitHub release notes.
5. Commit the version bump with the GitHub Actions bot.
6. Create and push tag `vMAJOR.MINOR.PATCH`.
7. Create a GitHub Release.

The first implementation can use `python-semantic-release` to avoid custom release scripting.

## Documentation

Update README and usage docs with:

- GitHub repository URL;
- install one-liner;
- update command;
- uninstall one-liner;
- managed checkout location;
- data removal warning;
- troubleshooting note for `~/.local/bin` not being on `PATH`.

## Non-Goals

- No PyPI publishing in this pass.
- No Homebrew, AUR, Nix, or platform package manager distribution in this pass.
- No Windows PowerShell installer in this pass.
- No automatic background update checks.
- No deletion of translation workspace directories during uninstall.

## Acceptance Criteria

- `origin` points to `https://github.com/InkyQuill/hieronymus.git`.
- `install.sh` performs an idempotent managed checkout install.
- `uninstall.sh` removes the app and can optionally purge Hieronymus settings/data.
- `hiero update --check` reports current/latest versions.
- `hiero update` updates managed installs in place.
- GitHub Actions can tag semantic releases from `main`.
- README and usage docs show install/update/uninstall commands.
- The required verification commands pass:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```
