# Project skills final-review fix report

## Fixes

- Reworked project skill installation to stage each complete bundled skill directory as a sibling under each selected `skills` root.
- Validates all selected roots and bundled destination paths before creating staging directories or replacing any owned skill.
- Replaces owned skill directories through rename-based swaps, retaining rollback backups until every staged replacement succeeds.
- Preserves unrelated skill directories and rejects non-owned collisions or symlinked destinations.
- Added precise Click option help for repeatable targets, interactive/non-TTY selection, dry runs, and the limited scope of `--yes`.

## Regression coverage

- Seeds stale helper files in `hieronymus-read` under both `.agents` and `.claude`, then verifies installation removes those files while preserving custom skills.
- Asserts both install and uninstall help text documents the option contract.

## Verification

- `uv run pytest tests/test_project_skills.py tests/test_cli_project_skills.py -q` — 22 passed.
- Earlier combined feature run had 74 passing tests. A subsequent concurrent change in `agent_assets.py` made `tests/test_agent_assets.py::test_read_learn_remember_skills_keep_judgment_out_of_mcp` fail on changed read-skill wording.
- The same concurrent file currently fails `uv run ruff check .` with an E501 line-length error at `src/hieronymus/agent_assets.py:67`.
- `git diff --check` — passed before that concurrent change.
