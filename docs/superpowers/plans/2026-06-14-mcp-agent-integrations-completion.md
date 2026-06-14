# MCP Agent Integrations Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the MCP and agent integrations roadmap slice by making every known, writable host integration real and tested, representing every installed local agent target explicitly, keeping unknown or undocumented host protocols reserved, and preserving the boundary that Read, Learn, and Remember stay agent skill workflows rather than MCP judgment wrappers.

**Architecture:** The Python plugin registry remains the single source of truth for agent targets. Installable integrations write only managed host configuration through `AgentPlugin.install()` implementations and generated plugin assets from `agent_assets.py`. Reserved integrations return an explicit reserved result and do not write host files. MCP exposes deterministic primitives; workflow judgment remains in packaged skills.

**Tech Stack:** Python 3.12, `uv`, `pytest`, `ruff`, TOML/JSON host config patchers, existing Hieronymus agent plugin assets.

---

## Current State

- `src/hieronymus/agent_plugins/` already has real installers for Claude Code, Codex, OpenCode, OpenClaw, and Gemini CLI.
- `pi` and `hermes` are present as stub-style reserved targets through `reserved.py`.
- Xiaomi MiMo is installed locally under `~/.mimocode`; `.zshrc` adds `~/.mimocode/bin` to `PATH`.
- `mimo --help` exposes `mimo mcp` and `mimo plugin`, but `mimo mcp add --help` does not document a stable noninteractive argument contract, and the current local `~/.config/mimocode` contains no writable config schema to patch.
- MiMo therefore must be targeted explicitly, but it should be reserved until Hieronymus has a verified host protocol that can be tested without mutating a real user profile.
- `BaseAgentPlugin.install()` still returns `result_kind="stub"`, so reserved targets are not semantically distinct from an unimplemented target.
- Tests cover most install paths, but they do not enforce a complete install/reinstall matrix for every integration that writes host configuration.
- `docs/roadmap.md` still lists MCP and Agent Integrations as remaining work.

## Target Agent Matrix

| Target | Canonical CLI Target | Host Surface | Status |
| --- | --- | --- | --- |
| Claude Code | `claude` | `~/.claude.json`, `~/.claude` detection | Installable |
| Codex | `codex` | `~/.codex/config.toml` | Installable |
| OpenCode | `opencode` | `~/.config/opencode/plugin.json` | Installable |
| OpenClaw | `openclaw` | `~/.openclaw/openclaw.json` | Installable |
| Gemini CLI | `gemini` | `~/.gemini/settings.json` | Installable |
| Xiaomi MiMo | `mimo` with aliases `xiaomi-mimo`, `xiaomi_mimo`, `mimocode` | `~/.mimocode`, `~/.config/mimocode`, `mimo mcp` discovery | Reserved |
| Pi | `pi` | `~/.pi` detection | Reserved |
| Hermes | `hermes` | `~/.hermes` detection | Reserved |

Reserved does not mean ignored. Reserved targets must appear in status, doctor, CLI errors, docs, and tests, and their install command must explain why no host config was written.

---

## Task 1: Make Reserved Targets Explicit And Add Plugin Aliases

**Files:**
- `src/hieronymus/agent_plugins/base.py`
- `src/hieronymus/agent_plugins/reserved.py`
- `src/hieronymus/agent_plugins/__init__.py`
- `src/hieronymus/cli.py`
- `tests/test_agent_plugins.py`
- `tests/test_cli_agent_install.py`

**Steps:**

- [ ] Extend the `AgentPlugin` protocol and `BaseAgentPlugin` with aliases.

  In `src/hieronymus/agent_plugins/base.py`:

  ```python
  class AgentPlugin(Protocol):
      name: str
      aliases: tuple[str, ...]
      display_name: str
      ...

  class BaseAgentPlugin:
      name = "base"
      aliases: tuple[str, ...] = ()
      display_name = "Base"
      ...
  ```

- [ ] Update plugin resolution to match aliases while keeping `available_plugins()` canonical.

  In `src/hieronymus/agent_plugins/__init__.py`:

  ```python
  def resolve_plugin(name: str) -> AgentPlugin:
      wanted = name.lower()
      for plugin in available_plugins():
          if plugin.name == wanted or wanted in plugin.aliases:
              return plugin
      supported = ", ".join(plugin.name for plugin in available_plugins())
      raise ValueError(f"Unsupported agent target {name!r}; supported targets: {supported}")
  ```

- [ ] Replace reserved stub behavior with explicit reserved behavior.

  In `src/hieronymus/agent_plugins/reserved.py`, introduce a reserved base class:

  ```python
  class ReservedAgentPlugin(BaseAgentPlugin):
      reserved_reason = "No safe host protocol is implemented for this target."

      def plan(self, config: HieronymusConfig, *, force: bool = False) -> InstallPlan:
          return InstallPlan(
              target=self.name,
              display_name=self.display_name,
              config_paths=tuple(self._expand(path) for path in self.config_paths),
              asset_root=config.agent_plugins_root / self.name,
              availability=self.availability(config),
              steps=(),
              result_kind="reserved",
              protocol_note=self.protocol_note or self.reserved_reason,
          )

      def install(self, config: HieronymusConfig, *, force: bool = False) -> InstallPlan:
          return self.plan(config, force=force)
  ```

  Make `PiPlugin` and `HermesPlugin` inherit `ReservedAgentPlugin`. Keep their detection paths so doctor/status can explain that the host exists but Hieronymus will not write config for it.

- [ ] Update CLI install/status messaging so reserved targets do not print stub wording.

  In `src/hieronymus/cli.py`, wherever `InstallPlan.result_kind` is rendered, add a reserved branch:

  ```python
  if plan.result_kind == "reserved":
      _echo_json_or_line(
          json_output,
          {"target": plan.target, "result": "reserved", "protocol_note": plan.protocol_note},
          f"{plan.display_name}: reserved target; {plan.protocol_note}",
      )
      return
  ```

  Preserve existing JSON behavior and existing output for installable integrations.

- [ ] Update tests.

  In `tests/test_agent_plugins.py`:

  ```python
  def test_available_plugins_lists_canonical_targets_in_order() -> None:
      assert [plugin.name for plugin in available_plugins()] == [
          "claude",
          "codex",
          "openclaw",
          "opencode",
          "gemini",
          "mimo",
          "pi",
          "hermes",
      ]

  def test_resolve_plugin_supports_aliases() -> None:
      assert resolve_plugin("xiaomi-mimo").name == "mimo"
      assert resolve_plugin("xiaomi_mimo").name == "mimo"
      assert resolve_plugin("mimocode").name == "mimo"

  def test_reserved_plugins_report_reserved_install_plan(tmp_path: Path) -> None:
      config = HieronymusConfig(data_root=tmp_path)
      plugin = resolve_plugin("pi")

      plan = plugin.install(config)

      assert plan.result_kind == "reserved"
      assert plan.steps == ()
      assert "protocol" in plan.protocol_note.lower()
      assert not (config.agent_plugins_root / "pi").exists()
  ```

  In `tests/test_cli_agent_install.py`, add a reserved-output test for `pi` that asserts `--json` returns `result` or `result_kind` as `reserved`, and a human-output test that asserts the word `reserved` appears.

**Verification:**

```bash
uv run pytest tests/test_agent_plugins.py tests/test_cli_agent_install.py
uv run ruff check src/hieronymus/agent_plugins src/hieronymus/cli.py tests/test_agent_plugins.py tests/test_cli_agent_install.py
```

---

## Task 2: Add Xiaomi MiMo As A Reserved, Detected Target

**Files:**
- `src/hieronymus/agent_plugins/reserved.py`
- `src/hieronymus/agent_plugins/__init__.py`
- `tests/test_agent_plugins.py`
- `tests/test_cli_agent_install.py`
- `tests/test_doctor_agent_plugins.py`

**Steps:**

- [ ] Add `MimoPlugin` as a reserved integration.

  In `src/hieronymus/agent_plugins/reserved.py`:

  ```python
  class MimoPlugin(ReservedAgentPlugin):
      name = "mimo"
      aliases = ("xiaomi-mimo", "xiaomi_mimo", "mimocode")
      display_name = "Xiaomi MiMo"
      detect_paths = ("~/.mimocode", "~/.config/mimocode")
      config_paths = ("~/.config/mimocode",)
      protocol_note = (
          "Xiaomi MiMo is detected through ~/.mimocode and ~/.config/mimocode, "
          "but Hieronymus does not write MiMo configuration until a stable "
          "noninteractive MCP or plugin configuration contract is implemented."
      )
  ```

  Do not add generated assets for `mimo` yet. Without a writable host protocol, assets would be orphaned.

- [ ] Register MiMo in the plugin registry.

  In `src/hieronymus/agent_plugins/__init__.py`, import `MimoPlugin` from `reserved.py` and add it after `GeminiPlugin()` and before `PiPlugin()`.

- [ ] Add MiMo availability and reserved install tests.

  In `tests/test_agent_plugins.py`:

  ```python
  def test_mimo_availability_detects_mimocode_home(isolated_home: Path, tmp_path: Path) -> None:
      (isolated_home / ".mimocode").mkdir()
      config = HieronymusConfig(data_root=tmp_path)

      availability = resolve_plugin("mimo").availability(config)

      assert availability.detected is True
      assert availability.installed is False
      assert availability.reason == "reserved target"
  ```

  If the current `availability()` helper does not return `"reserved target"`, update `ReservedAgentPlugin.availability()` to preserve detection while making the reserved reason explicit.

- [ ] Add CLI tests for MiMo aliases.

  In `tests/test_cli_agent_install.py`, add a test that `hiero agent install mimocode --json` resolves canonical target `mimo` and returns `reserved`.

- [ ] Update doctor plugin tests to include MiMo in target listings and to assert reserved targets do not produce missing-config failures.

**Verification:**

```bash
uv run pytest tests/test_agent_plugins.py tests/test_cli_agent_install.py tests/test_doctor_agent_plugins.py
uv run ruff check src/hieronymus/agent_plugins tests/test_agent_plugins.py tests/test_cli_agent_install.py tests/test_doctor_agent_plugins.py
```

---

## Task 3: Enforce Install And Reinstall Coverage For Every Writable Integration

**Files:**
- `tests/test_agent_assets.py`
- `tests/test_agent_plugin_installers.py`
- `tests/test_doctor_agent_plugins.py`
- `tests/test_agent_plugins.py`

**Steps:**

- [ ] Add a canonical list of writable plugin targets in tests.

  In `tests/test_agent_plugin_installers.py`:

  ```python
  WRITABLE_PLUGIN_TARGETS = ["claude", "codex", "openclaw", "opencode", "gemini"]
  RESERVED_PLUGIN_TARGETS = ["mimo", "pi", "hermes"]
  ```

- [ ] Add a registry coverage test so new writable integrations cannot miss installer coverage.

  ```python
  def test_writable_plugin_targets_match_registry() -> None:
      assert [
          plugin.name
          for plugin in available_plugins()
          if plugin.installs_managed_config
      ] == WRITABLE_PLUGIN_TARGETS
  ```

- [ ] Add an update-style reinstall test that runs every writable installer twice.

  Use per-host config setup helpers already present in `tests/test_agent_plugin_installers.py`. For targets whose host config is JSON, assert the parsed JSON object is identical across reinstall. For Codex, assert TOML remains parseable and the managed entries are unchanged.

  Example shape:

  ```python
  @pytest.mark.parametrize("target", WRITABLE_PLUGIN_TARGETS)
  def test_writable_plugin_reinstall_is_idempotent(
      target: str,
      isolated_home: Path,
      tmp_path: Path,
  ) -> None:
      config = HieronymusConfig(data_root=tmp_path)
      plugin = resolve_plugin(target)

      first_plan = plugin.install(config)
      first_snapshot = _host_config_snapshot(target, isolated_home)
      second_plan = plugin.install(config)
      second_snapshot = _host_config_snapshot(target, isolated_home)

      assert first_plan.result_kind == "installed"
      assert second_plan.result_kind == "installed"
      assert second_snapshot == first_snapshot
      assert plugin.availability(config).installed is True
  ```

  Implement `_host_config_snapshot()` in the test file using structured parsing:

  - Codex: parse `~/.codex/config.toml` with `tomllib.loads`.
  - Claude: parse `~/.claude.json`.
  - OpenCode: parse `~/.config/opencode/plugin.json`.
  - OpenClaw: parse `~/.openclaw/openclaw.json`.
  - Gemini: parse `~/.gemini/settings.json`.

- [ ] Add a test that reserved targets are excluded from writable install matrix.

  ```python
  def test_reserved_targets_do_not_write_host_configuration(tmp_path: Path) -> None:
      config = HieronymusConfig(data_root=tmp_path)

      for target in RESERVED_PLUGIN_TARGETS:
          plugin = resolve_plugin(target)
          plan = plugin.install(config)
          assert plan.result_kind == "reserved"
          assert not (config.agent_plugins_root / plugin.name).exists()
  ```

- [ ] Add asset coverage for every writable target manifest.

  In `tests/test_agent_assets.py`:

  ```python
  @pytest.mark.parametrize(
      ("target", "manifest_path"),
      [
          ("codex", ".codex-plugin/plugin.json"),
          ("claude", ".claude-plugin/plugin.json"),
          ("gemini", "gemini-extension.json"),
          ("opencode", "opencode/plugin.json"),
          ("openclaw", "openclaw/plugin.json"),
      ],
  )
  def test_render_agent_plugin_assets_includes_writable_target_manifest(
      target: str,
      manifest_path: str,
  ) -> None:
      assets = render_agent_plugin_assets(target)

      assert manifest_path in assets
      assert "skills/hieronymus-recall/SKILL.md" in assets
      assert "skills/hieronymus-learn/SKILL.md" in assets
      assert "skills/hieronymus-read/SKILL.md" in assets
      assert "skills/hieronymus-remember/SKILL.md" in assets
  ```

- [ ] Ensure doctor tests remain aligned.

  In `tests/test_doctor_agent_plugins.py`, update expected target names to include `mimo`, and update any result-kind expectations for reserved targets from `stub` to `reserved`.

**Verification:**

```bash
uv run pytest tests/test_agent_assets.py tests/test_agent_plugin_installers.py tests/test_agent_plugins.py tests/test_doctor_agent_plugins.py
uv run ruff check tests/test_agent_assets.py tests/test_agent_plugin_installers.py tests/test_agent_plugins.py tests/test_doctor_agent_plugins.py
```

---

## Task 4: Update Agent Integration Documentation And Roadmap

**Files:**
- `docs/agent-workflows.md`
- `docs/service-toolkit.md`
- `docs/roadmap.md`
- `docs/superpowers/plans/2026-06-14-mcp-agent-integrations-completion.md`

**Steps:**

- [ ] Update `docs/agent-workflows.md`.

  Add or update the supported target list:

  ```markdown
  Supported install targets:

  - `claude` writes Claude Code MCP registration into `~/.claude.json`.
  - `codex` writes Codex MCP and plugin registration into `~/.codex/config.toml`.
  - `opencode` writes OpenCode MCP and plugin registration into `~/.config/opencode/plugin.json`.
  - `openclaw` writes OpenClaw MCP and plugin registration into `~/.openclaw/openclaw.json`.
  - `gemini` writes Gemini CLI MCP and extension registration into `~/.gemini/settings.json`.

  Reserved targets:

  - `mimo` detects Xiaomi MiMo through `~/.mimocode` and `~/.config/mimocode`, but does not write host configuration until a stable noninteractive MCP or plugin configuration contract is implemented. Aliases: `xiaomi-mimo`, `xiaomi_mimo`, `mimocode`.
  - `pi` is detected for status/doctor output, but Hieronymus does not write host configuration because no safe Pi protocol is implemented.
  - `hermes` is detected for status/doctor output, but Hieronymus does not write host configuration because no safe Hermes protocol is implemented.
  ```

  Keep the section that says Read, Learn, and Remember are skill workflows. Add a sentence:

  ```markdown
  The MCP server continues to expose primitives only. Read, Learn, and Remember remain agent skills so host agents make contextual workflow decisions instead of asking Hieronymus MCP tools to judge translation intent.
  ```

- [ ] Update `docs/service-toolkit.md`.

  In the CLI/service target documentation, include MiMo in the reserved target list and mark Pi/Hermes as reserved. Ensure machine-readable output guidance remains: components use `--json`, not parsed human CLI text.

- [ ] Mark the MCP And Agent Integrations roadmap slice complete.

  In `docs/roadmap.md`, replace the remaining-work bullets with a completed status:

  ```markdown
  ### MCP And Agent Integrations

  Status: complete for the alpha integration baseline.

  Completed:

  - Claude Code, Codex, OpenCode, OpenClaw, and Gemini CLI have installable host configuration paths.
  - Xiaomi MiMo, Pi, and Hermes remain explicit reserved targets because no safe noninteractive host configuration protocol is implemented.
  - Install and reinstall coverage exists for every integration that writes host configuration.
  - Read, Learn, and Remember remain agent skill workflows rather than MCP judgment wrappers.
  ```

- [ ] Add a final “Implemented Changes” section at the bottom of this plan after implementation, summarizing the files changed and the verification commands run.

**Verification:**

```bash
uv run pytest tests/test_agent_assets.py tests/test_agent_plugin_installers.py tests/test_agent_plugins.py tests/test_doctor_agent_plugins.py tests/test_cli_agent_install.py
uv run ruff check docs src tests
```

---

## Task 5: Full Verification

**Steps:**

- [ ] Run the project verification suite required by `AGENTS.md`.

  ```bash
  uv run pytest
  uv run ruff check .
  uv run ruff format --check .
  ```

- [ ] If frontend files are untouched, no frontend verification is required. If a frontend file is changed during implementation, also run:

  ```bash
  bun run --cwd frontend format
  bun run --cwd frontend test
  bun run --cwd frontend build
  ```

- [ ] Review the diff:

  ```bash
  git status --short
  git diff --stat
  git diff -- docs/roadmap.md docs/agent-workflows.md docs/service-toolkit.md
  git diff -- src/hieronymus/agent_assets.py src/hieronymus/agent_plugins
  git diff -- tests/test_agent_assets.py tests/test_agent_plugin_installers.py tests/test_agent_plugins.py tests/test_cli_agent_install.py tests/test_doctor_agent_plugins.py
  ```

- [ ] Confirm no implementation adds MCP tools that make Read/Learn/Remember judgment decisions. The generated skill markdown may call MCP primitives, but no new MCP wrapper should decide when a translation session should read, learn, or remember.

---

## Task 6: Commit, Push, And Open PR

**Steps:**

- [ ] Commit with the project author identity:

  ```bash
  git status --short
  git add docs/agent-workflows.md docs/service-toolkit.md docs/roadmap.md docs/superpowers/plans/2026-06-14-mcp-agent-integrations-completion.md src/hieronymus/agent_plugins src/hieronymus/cli.py tests
  git commit -m "Complete MCP agent integration baseline"
  ```

- [ ] Push the branch:

  ```bash
  git push -u origin plan/mcp-agent-integrations-completion-pass
  ```

- [ ] Open a PR against `main`:

  ```bash
  gh pr create \
    --base main \
    --head plan/mcp-agent-integrations-completion-pass \
    --title "Complete MCP agent integration baseline" \
    --body "$(cat <<'EOF'
  ## Summary
  - make reserved agent integrations explicit instead of stub installs
  - add Xiaomi MiMo as a detected reserved target
  - add install/reinstall coverage for every host-config-writing integration
  - update agent workflow docs and roadmap completion status

  ## Verification
  - uv run pytest
  - uv run ruff check .
  - uv run ruff format --check .
  EOF
  )"
  ```

---

## Review Checklist

- [ ] `hiero agent install claude`, `codex`, `opencode`, `openclaw`, and `gemini` all write managed host configuration through plugin classes.
- [ ] `hiero agent install mimo`, `pi`, and `hermes` clearly report `reserved` and do not write config or assets.
- [ ] Every writable target has install and reinstall tests.
- [ ] MiMo detects `~/.mimocode` and `~/.config/mimocode`, and aliases `xiaomi-mimo`, `xiaomi_mimo`, and `mimocode` resolve to canonical `mimo`.
- [ ] Docs describe direct host config writes and reserved targets accurately.
- [ ] Roadmap no longer lists MCP and Agent Integrations as remaining work.
- [ ] Read, Learn, and Remember remain skills, not MCP judgment wrappers.

---

## Implemented Changes

- Updated `docs/agent-workflows.md` to document Claude Code, Codex, OpenCode, OpenClaw, and Gemini CLI as writable install targets with their host configuration paths, and MiMo, Pi, and Hermes as reserved targets.
- Updated `docs/service-toolkit.md` to describe MiMo, Pi, and Hermes as reserved detectable targets and to preserve the `--json` machine-readable output boundary for automation.
- Updated `docs/roadmap.md` to mark the MCP And Agent Integrations alpha baseline complete.
- Updated this plan with the documentation completion record.

Verification run so far:

- `uv run pytest tests/test_agent_plugins.py tests/test_cli_agent_install.py tests/test_install.py tests/test_doctor_agent_plugins.py tests/test_cli_service.py`
- `uv run pytest tests/test_agent_plugins.py tests/test_cli_agent_install.py tests/test_doctor_agent_plugins.py tests/test_install.py`
- `uv run pytest tests/test_agent_assets.py tests/test_agent_plugin_installers.py tests/test_agent_plugins.py tests/test_doctor_agent_plugins.py`
- `uv run ruff check src/hieronymus/agent_plugins src/hieronymus/cli.py tests/test_agent_plugins.py tests/test_cli_agent_install.py`
- `uv run ruff check src/hieronymus/agent_plugins tests/test_agent_plugins.py tests/test_cli_agent_install.py tests/test_doctor_agent_plugins.py`
- `uv run ruff check tests/test_agent_assets.py tests/test_agent_plugin_installers.py tests/test_agent_plugins.py tests/test_doctor_agent_plugins.py`
- `uv run pytest tests/test_agent_assets.py tests/test_agent_plugin_installers.py tests/test_agent_plugins.py tests/test_doctor_agent_plugins.py tests/test_cli_agent_install.py`
- `uv run ruff check docs src tests`
- `uv run pytest`
- `uv run ruff check .`
- `uv run ruff format --check .`

Full verification completed in Task 5: `1002 passed`, `ruff check` passed, and
`ruff format --check` reported `140 files already formatted` after formatting
`tests/test_agent_plugin_installers.py`.
