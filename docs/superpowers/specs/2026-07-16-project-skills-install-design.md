# Project-local Hieronymus skills installation

## Goal

Provide a small, project-local CLI for installing the complete Hieronymus
workflow skill bundle into one or both supported agent directories in the
current workspace. This is separate from `hiero install <agent>`, which owns
global host integration, MCP registration, and plugin manifests.

## Commands

```text
hiero skills install [--target agents|claude]... [--dry-run] [--yes]
hiero skills uninstall [--target agents|claude]... [--dry-run] [--yes]
```

- `install` writes all bundled `hieronymus-*` skills.
- `uninstall` removes only directories owned by Hieronymus.
- `--target` may be repeated, permitting one or both destinations.
- With an interactive terminal and no `--target`, the command presents a
  multi-select prompt for `.agents` and `.claude`.
- Without a terminal, no target is inferred: the command fails with a clear
  instruction to supply at least one `--target`.
- `--yes` accepts the interactive selection confirmation. It has no effect
  when explicit targets are already supplied.
- `--dry-run` reports paths and actions without modifying the workspace.

## Layout and ownership

The command resolves the workspace as the current directory and writes:

```text
<workspace>/.agents/skills/hieronymus-*/SKILL.md
<workspace>/.claude/skills/hieronymus-*/SKILL.md
```

Skill content comes from the existing `agent_assets.asset_map()` bundle. The
install command copies content rather than creating symlinks, so a repository
remains portable and can pin the workflow text it contains.

Ownership is determined by the `hieronymus-` directory prefix and an expected
`SKILL.md` file. Uninstall never removes `.agents`, `.claude`, their `skills`
parents, or non-Hieronymus directories. Existing owned skill directories are
replaced atomically during install so updates cannot leave mixed content.

## User experience and failure handling

The command prints the selected destination names and each affected skill
directory. It reports a concise summary of installed, updated, skipped, or
removed skills. Invalid targets and a missing noninteractive target produce a
Click usage error. Filesystem failures identify the path and preserve unrelated
workspace content.

The new command does not patch MCP files, agent manifests, or global agent
configuration. Users who need MCP registration continue to use `hiero install
<agent>`.

## Testing

Tests cover both destinations, the complete skill set, idempotent updates,
dry-run behavior, multi-target selection through repeated flags, non-TTY
failure without targets, and uninstall preservation of unrelated skills and
parent directories. CLI tests verify the rendered summary and errors.
