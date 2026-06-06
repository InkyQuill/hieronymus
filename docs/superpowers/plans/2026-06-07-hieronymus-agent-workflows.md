# Hieronymus Agent Workflows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build real agent workflow support: learn/read MCP tools, packaged Hieronymus skills, plugin-style agent installer definitions, host availability/installed checks, hooks, and installer wiring for Claude, Codex, OpenClaw, opencode, and Gemini CLI.

**Architecture:** Keep the Hieronymus daemon and MCP tools as the only memory API that integrations call. Add a small plugin registry where each agent integration is a provider module; adding a provider module makes it appear in installer candidates, status output, and doctor checks. Installers write versioned Hieronymus plugin assets under `~/.config/hieronymus/agent-plugins/<agent>/` and patch each host's config with backups, markers, and idempotent updates.

**Tech Stack:** Python 3.12+, Click, stdlib `json`/`tomllib`/`importlib.resources`, `tomli-w` for TOML writes, SQLite-backed existing stores, MCP stdio server, pytest, ruff.

---

## Scope Rules

- Work from `main` in a new feature worktree.
- Focus only on `docs/superpowers/specs/2026-06-06-hieronymus-agent-workflows.md`.
- Keep global Hieronymus config and plugin asset cache under `~/.config/hieronymus` by default.
- Do not bypass MCP or service APIs from hooks, skills, or generated plugins.
- Do not make agents auto-approve strict concept/rendering proposals.
- Keep installer operations idempotent and backed up. A second install should report `already-installed` or rewrite identical managed blocks without duplicating config.
- Treat Pi and Hermes as detectable reserved targets unless a concrete local host format exists in this pass.

## Target File Structure

```text
src/hieronymus/
├── agent_context.py              # Project/session context file discovery for hooks and skills.
├── agent_hooks.py                # `python -m hieronymus.agent_hooks <event>` hook entry point.
├── agent_ingestion.py            # Learn/read block splitting and structured extraction helpers.
├── agent_plugins/
│   ├── __init__.py               # Plugin registry discovery.
│   ├── base.py                   # AgentPlugin protocol, status/result models, safe config helpers.
│   ├── claude.py                 # Claude Code / Claude Desktop provider.
│   ├── codex.py                  # Codex provider.
│   ├── gemini.py                 # Gemini CLI provider.
│   ├── openclaw.py               # OpenClaw provider.
│   ├── opencode.py               # opencode provider.
│   └── reserved.py               # Pi and Hermes reserved providers.
├── agent_assets.py               # Generated skill, hook, MCP, and plugin asset templates.
├── cli.py                        # `hiero install`, `hiero install list`, status/doctor wiring.
├── config.py                     # Agent plugin cache path helper.
├── doctor.py                     # Agent integration findings.
├── install.py                    # Compatibility wrapper around plugin registry.
└── mcp_server.py                 # Add `hieronymus_learn` and `hieronymus_read` tools.
tests/
├── test_agent_context.py
├── test_agent_hooks.py
├── test_agent_ingestion.py
├── test_agent_plugins.py
├── test_agent_plugin_installers.py
├── test_cli_agent_install.py
├── test_doctor_agent_plugins.py
└── test_mcp_agent_ingestion.py
docs/
├── agent-workflows.md
└── service-toolkit.md
```

## Agent Host Detection Contract

Each provider returns these values:

```python
AgentAvailability(
    target="codex",
    display_name="Codex",
    available=True,
    installed=False,
    detect_paths=["~/.codex"],
    config_paths=["~/.codex/config.toml"],
    install_path="~/.config/hieronymus/agent-plugins/codex",
    reason="host config directory exists",
)
```

Initial host availability checks:

- Claude Code / Claude Desktop: `~/.claude` or `~/.claude.json`
- Codex: `~/.codex`
- OpenClaw: `~/.openclaw`
- opencode: `~/.config/opencode`
- Gemini CLI: `~/.gemini`
- Pi: `~/.pi`
- Hermes: `~/.hermes`

Installed means both:

- the Hieronymus plugin asset directory for that target exists under `config.agent_plugins_root`
- the host config contains the managed Hieronymus marker or expected plugin reference

## Managed Config Markers

Use explicit markers in host config where a structured host format does not provide a plugin id field:

```text
# BEGIN HIERONYMUS MANAGED BLOCK
# END HIERONYMUS MANAGED BLOCK
```

Structured JSON/TOML configs should use a stable key:

```json
{
  "hieronymus": {
    "managed": true,
    "version": "0.1.0"
  }
}
```

## Task 1: Add Learn/Read Ingestion Service

**Files:**
- Create: `src/hieronymus/agent_ingestion.py`
- Modify: `src/hieronymus/mcp_server.py`
- Create: `tests/test_agent_ingestion.py`
- Create: `tests/test_mcp_agent_ingestion.py`

- [ ] **Step 1: Write failing unit tests for block splitting and learn persistence**

Create `tests/test_agent_ingestion.py`:

```python
from __future__ import annotations

from pathlib import Path

from hieronymus.agent_ingestion import IngestionService, split_learning_blocks
from hieronymus.config import HieronymusConfig
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def _session(config: HieronymusConfig) -> int:
    Registry(config).create_series(
        slug="oso",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    context = TranslationContext(
        series_slug="oso",
        source_language="ja",
        target_language="en",
        task_type="learning",
        volume="1",
        chapter="1",
    )
    return WorkspaceStore(config).start_session(context).id


def test_split_learning_blocks_keeps_source_order() -> None:
    blocks = split_learning_blocks(
        "First paragraph.\n\nSecond paragraph has a term: Gantz.\n\nThird.",
        max_chars=80,
    )

    assert [block.index for block in blocks] == [1, 2, 3]
    assert blocks[1].text == "Second paragraph has a term: Gantz."


def test_learn_writes_short_term_memories(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    session_id = _session(config)

    result = IngestionService(config).learn(
        session_id=session_id,
        text="Gantz avoids heavy armor.\n\nGantz relies on speed.",
        source_role="mentor",
        source_ref="audit:chapter-1",
    )

    assert result.block_count == 2
    assert result.memory_ids == [1, 2]
    memories = WorkspaceStore(config).list_short_term_memories(session_id)
    assert [memory.source_role for memory in memories] == ["mentor", "mentor"]
    assert memories[0].metadata["ingestion_mode"] == "learn"
    assert memories[0].metadata["block_index"] == 1
```

- [ ] **Step 2: Write failing unit tests for read extraction without default persistence**

Append to `tests/test_agent_ingestion.py`:

```python
def test_read_extracts_candidates_without_writing_by_default(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    session_id = _session(config)

    result = IngestionService(config).read(
        session_id=session_id,
        text="Gantz: a martial arts user. Soumen may need a footnote.",
        source_ref="notes:gantz",
    )

    assert result.stored_memory_ids == []
    assert "Gantz" in result.candidate_terms
    assert "Soumen" in result.candidate_terms
    assert WorkspaceStore(config).list_short_term_memories(session_id) == []


def test_read_can_store_deliberate_observation_when_requested(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    session_id = _session(config)

    result = IngestionService(config).read(
        session_id=session_id,
        text="Gantz: a martial arts user.",
        source_ref="notes:gantz",
        store_observation=True,
    )

    assert result.stored_memory_ids == [1]
    memory = WorkspaceStore(config).list_short_term_memories(session_id)[0]
    assert memory.kind == "read_observation"
    assert memory.metadata["ingestion_mode"] == "read"
```

- [ ] **Step 3: Run tests and verify they fail**

Run:

```bash
uv run pytest tests/test_agent_ingestion.py -v
```

Expected:

```text
FAILED tests/test_agent_ingestion.py::test_split_learning_blocks_keeps_source_order
FAILED tests/test_agent_ingestion.py::test_learn_writes_short_term_memories
FAILED tests/test_agent_ingestion.py::test_read_extracts_candidates_without_writing_by_default
FAILED tests/test_agent_ingestion.py::test_read_can_store_deliberate_observation_when_requested
```

- [ ] **Step 4: Implement ingestion service**

Create `src/hieronymus/agent_ingestion.py`:

```python
from __future__ import annotations

import re
from dataclasses import dataclass

from hieronymus.config import HieronymusConfig
from hieronymus.workspace import WorkspaceStore


@dataclass(frozen=True)
class LearningBlock:
    index: int
    text: str


@dataclass(frozen=True)
class LearnResult:
    session_id: int
    block_count: int
    memory_ids: list[int]


@dataclass(frozen=True)
class ReadResult:
    session_id: int
    candidate_terms: list[str]
    findings: list[str]
    stored_memory_ids: list[int]


TERM_PATTERN = re.compile(r"\b[A-Z][A-Za-z0-9'_-]{2,}\b")


def split_learning_blocks(text: str, *, max_chars: int = 1200) -> list[LearningBlock]:
    paragraphs = [part.strip() for part in re.split(r"\n\s*\n", text) if part.strip()]
    blocks: list[str] = []
    for paragraph in paragraphs:
        if len(paragraph) <= max_chars:
            blocks.append(paragraph)
            continue
        current = ""
        for sentence in re.split(r"(?<=[.!?。！？])\s+", paragraph):
            if not sentence:
                continue
            candidate = sentence if not current else f"{current} {sentence}"
            if len(candidate) > max_chars and current:
                blocks.append(current)
                current = sentence
            else:
                current = candidate
        if current:
            blocks.append(current)
    return [LearningBlock(index=index, text=block) for index, block in enumerate(blocks, start=1)]


def extract_candidate_terms(text: str) -> list[str]:
    seen: set[str] = set()
    terms: list[str] = []
    for match in TERM_PATTERN.finditer(text):
        term = match.group(0)
        if term not in seen:
            seen.add(term)
            terms.append(term)
    return terms


class IngestionService:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config

    def learn(
        self,
        *,
        session_id: int,
        text: str,
        source_role: str,
        source_ref: str = "",
        kind: str = "learned_block",
    ) -> LearnResult:
        workspace = WorkspaceStore(self.config)
        memory_ids: list[int] = []
        blocks = split_learning_blocks(text)
        for block in blocks:
            memory_ids.append(
                workspace.add_short_term_memory(
                    session_id=session_id,
                    source_role=source_role,
                    kind=kind,
                    text=block.text,
                    source_ref=source_ref,
                    metadata={
                        "ingestion_mode": "learn",
                        "block_index": block.index,
                        "block_count": len(blocks),
                    },
                )
            )
        return LearnResult(session_id=session_id, block_count=len(blocks), memory_ids=memory_ids)

    def read(
        self,
        *,
        session_id: int,
        text: str,
        source_ref: str = "",
        store_observation: bool = False,
    ) -> ReadResult:
        terms = extract_candidate_terms(text)
        findings = [f"candidate_term:{term}" for term in terms]
        stored: list[int] = []
        if store_observation and findings:
            stored.append(
                WorkspaceStore(self.config).add_short_term_memory(
                    session_id=session_id,
                    source_role="mundane",
                    kind="read_observation",
                    text="\n".join(findings),
                    source_ref=source_ref,
                    metadata={"ingestion_mode": "read", "candidate_terms": terms},
                )
            )
        return ReadResult(
            session_id=session_id,
            candidate_terms=terms,
            findings=findings,
            stored_memory_ids=stored,
        )
```

- [ ] **Step 5: Add MCP tool tests**

Create `tests/test_mcp_agent_ingestion.py`:

```python
from __future__ import annotations

from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.mcp_server import hieronymus_learn, hieronymus_read
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def test_mcp_learn_writes_blocks(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    Registry(config).create_series(slug="oso", title="Only Sense Online")
    session_id = WorkspaceStore(config).start_session(
        TranslationContext(
            series_slug="oso",
            source_language="ja",
            target_language="en",
            task_type="learning",
        )
    ).id

    result = hieronymus_learn(
        session_id=session_id,
        text="One.\n\nTwo.",
        source_role="user",
        source_ref="user:note",
    )

    assert result == {"session_id": session_id, "block_count": 2, "memory_ids": [1, 2]}


def test_mcp_read_returns_findings_without_memory_write(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    Registry(config).create_series(slug="oso", title="Only Sense Online")
    session_id = WorkspaceStore(config).start_session(
        TranslationContext(
            series_slug="oso",
            source_language="ja",
            target_language="en",
            task_type="reading",
        )
    ).id

    result = hieronymus_read(
        session_id=session_id,
        text="Gantz uses speed.",
        source_ref="notes:gantz",
    )

    assert result["candidate_terms"] == ["Gantz"]
    assert result["stored_memory_ids"] == []
```

- [ ] **Step 6: Add MCP tools**

Modify `src/hieronymus/mcp_server.py`:

```python
from hieronymus.agent_ingestion import IngestionService
```

Add after `hieronymus_short_term_add`:

```python
@server.tool()
def hieronymus_learn(
    session_id: int,
    text: str,
    source_role: str,
    source_ref: str = "",
    kind: str = "learned_block",
) -> dict[str, object]:
    """Split material into short-term memories eligible for dreaming."""
    config = _load_validated_config()
    result = IngestionService(config).learn(
        session_id=session_id,
        text=text,
        source_role=source_role,
        source_ref=source_ref,
        kind=kind,
    )
    return {
        "session_id": result.session_id,
        "block_count": result.block_count,
        "memory_ids": result.memory_ids,
    }


@server.tool()
def hieronymus_read(
    session_id: int,
    text: str,
    source_ref: str = "",
    store_observation: bool = False,
) -> dict[str, object]:
    """Extract temporary concepts/terms without committing the full source by default."""
    config = _load_validated_config()
    result = IngestionService(config).read(
        session_id=session_id,
        text=text,
        source_ref=source_ref,
        store_observation=store_observation,
    )
    return {
        "session_id": result.session_id,
        "candidate_terms": result.candidate_terms,
        "findings": result.findings,
        "stored_memory_ids": result.stored_memory_ids,
    }
```

- [ ] **Step 7: Run targeted tests and commit**

Run:

```bash
uv run pytest tests/test_agent_ingestion.py tests/test_mcp_agent_ingestion.py -v
```

Expected:

```text
6 passed
```

Commit:

```bash
git add src/hieronymus/agent_ingestion.py src/hieronymus/mcp_server.py tests/test_agent_ingestion.py tests/test_mcp_agent_ingestion.py
git commit -m "feat: add agent learn and read ingestion tools"
```

## Task 2: Add Workflow Skill and Plugin Asset Bundle

**Files:**
- Create: `src/hieronymus/agent_assets.py`
- Create: `tests/test_agent_assets.py`
- Modify: `docs/agent-workflows.md`

- [ ] **Step 1: Write failing tests for required assets**

Create `tests/test_agent_assets.py`:

```python
from __future__ import annotations

from hieronymus.agent_assets import asset_map, render_agent_plugin_assets


def test_asset_map_contains_required_skills() -> None:
    assets = asset_map()

    assert "skills/hieronymus-recall/SKILL.md" in assets
    assert "skills/hieronymus-learn/SKILL.md" in assets
    assert "skills/hieronymus-read/SKILL.md" in assets
    assert "skills/hieronymus-translate/SKILL.md" in assets
    assert "skills/hieronymus-review/SKILL.md" in assets
    assert "skills/hieronymus-orchestrate/SKILL.md" in assets
    assert "mcp/hieronymus.mcp.json" in assets
    assert "hooks/hooks.codex.json" in assets


def test_skill_text_enforces_strict_vs_advisory_contract() -> None:
    assets = asset_map()
    translate = assets["skills/hieronymus-translate/SKILL.md"]

    assert "Strict concept contracts are mandatory" in translate
    assert "Crystals and lessons are advisory" in translate
    assert "Do not approve terminology proposals yourself" in translate


def test_render_agent_plugin_assets_includes_agent_name() -> None:
    assets = render_agent_plugin_assets("codex")

    assert assets[".codex-plugin/plugin.json"].startswith("{")
    assert '"name": "hieronymus"' in assets[".codex-plugin/plugin.json"]
    assert "hieronymus-mcp" in assets["mcp/hieronymus.mcp.json"]
```

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
uv run pytest tests/test_agent_assets.py -v
```

Expected:

```text
FAILED tests/test_agent_assets.py::test_asset_map_contains_required_skills
FAILED tests/test_agent_assets.py::test_skill_text_enforces_strict_vs_advisory_contract
FAILED tests/test_agent_assets.py::test_render_agent_plugin_assets_includes_agent_name
```

- [ ] **Step 3: Implement asset bundle**

Create `src/hieronymus/agent_assets.py`:

```python
from __future__ import annotations

import json


MCP_CONFIG = {
    "mcpServers": {
        "hieronymus": {
            "command": "hieronymus-mcp",
            "args": [],
            "env": {},
        }
    }
}


CODEX_HOOKS = {
    "hooks": [
        {
            "event": "SessionStart",
            "command": "python",
            "args": ["-m", "hieronymus.agent_hooks", "session-start"],
        },
        {
            "event": "Stop",
            "command": "python",
            "args": ["-m", "hieronymus.agent_hooks", "session-end"],
        },
    ]
}


RECALL_SKILL = """---
name: hieronymus-recall
description: Recall Hieronymus memory before translation, review, terminology, or documentation work.
---

# Hieronymus Recall

Use this skill before memory-sensitive work. Start or identify a task session, call
`hieronymus_recall`, and keep strict concept contracts separate from advisory crystals.

Strict concept contracts are mandatory. Crystals and lessons are advisory. Cite influential crystals
when they shape a translation or review decision.
"""


LEARN_SKILL = """---
name: hieronymus-learn
description: Commit material into short-term memory for later dreaming and crystallization.
---

# Hieronymus Learn

Use when the user says to absorb, remember, study, ingest, import, or learn from a source.
Call `hieronymus_learn` with provenance. Do not promote strict terminology directly; dreaming will
produce crystals, lessons, erudition, and proposals later.
"""


READ_SKILL = """---
name: hieronymus-read
description: Inspect material for the current task without committing the whole source to memory.
---

# Hieronymus Read

Use for casual lookup, extraction, summaries, or temporary understanding. Call `hieronymus_read`.
By default, do not store every block as short-term memory.
"""


TRANSLATE_SKILL = """---
name: hieronymus-translate
description: Translate with Hieronymus strict terminology and advisory crystals.
---

# Hieronymus Translate

Strict concept contracts are mandatory. Crystals and lessons are advisory. Do not approve
terminology proposals yourself. Record uncertainty and discoveries as short-term memories.
"""


REVIEW_SKILL = """---
name: hieronymus-review
description: Review translation output using strict terminology and mentor-grade observations.
---

# Hieronymus Review

Check strict validation findings first. Identify whether crystals helped or misled. Record recurring
issues, contradictions, and correction patterns as mentor short-term memories.
"""


ORCHESTRATE_SKILL = """---
name: hieronymus-orchestrate
description: Coordinate Hieronymus task sessions, recall, validation, feedback, and dreaming.
---

# Hieronymus Orchestrate

Create a task session, recall before work, collect short-term memories, record feedback events, and
trigger or defer dreaming based on configuration or user instruction.
"""


def _json(payload: object) -> str:
    return json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n"


def asset_map() -> dict[str, str]:
    return {
        "skills/hieronymus-recall/SKILL.md": RECALL_SKILL,
        "skills/hieronymus-learn/SKILL.md": LEARN_SKILL,
        "skills/hieronymus-read/SKILL.md": READ_SKILL,
        "skills/hieronymus-translate/SKILL.md": TRANSLATE_SKILL,
        "skills/hieronymus-review/SKILL.md": REVIEW_SKILL,
        "skills/hieronymus-orchestrate/SKILL.md": ORCHESTRATE_SKILL,
        "mcp/hieronymus.mcp.json": _json(MCP_CONFIG),
        "hooks/hooks.codex.json": _json(CODEX_HOOKS),
    }


def render_agent_plugin_assets(target: str) -> dict[str, str]:
    assets = asset_map()
    plugin_json = {
        "name": "hieronymus",
        "version": "0.1.0",
        "description": "Translation memory, termbase, recall, and dreaming workflows for agents.",
        "skills": "./skills/",
        "mcpServers": "./mcp/hieronymus.mcp.json",
    }
    if target == "codex":
        plugin_json["hooks"] = "./hooks/hooks.codex.json"
        assets[".codex-plugin/plugin.json"] = _json(plugin_json)
    if target == "claude":
        assets[".claude-plugin/plugin.json"] = _json(plugin_json)
    if target == "gemini":
        assets["gemini-extension.json"] = _json(
            {
                "name": "hieronymus",
                "version": "0.1.0",
                "contextFileName": "AGENTS.md",
                "mcpServers": MCP_CONFIG["mcpServers"],
            }
        )
    if target == "opencode":
        assets["opencode/plugin.json"] = _json(
            {"name": "hieronymus", "version": "0.1.0", "mcp": "./mcp/hieronymus.mcp.json"}
        )
    return assets
```

- [ ] **Step 4: Add user docs**

Create `docs/agent-workflows.md`:

```markdown
# Hieronymus Agent Workflows

Hieronymus agent integrations install skills and MCP configuration for local coding agents. The
skills teach agents to recall memory, distinguish strict terms from advisory crystals, write
short-term observations, learn material deliberately, and read material casually.

`learn` stores blocks as short-term memory for dreaming. `read` extracts useful concepts and terms
without committing the whole source by default.
```

- [ ] **Step 5: Run tests and commit**

Run:

```bash
uv run pytest tests/test_agent_assets.py -v
```

Expected:

```text
3 passed
```

Commit:

```bash
git add src/hieronymus/agent_assets.py tests/test_agent_assets.py docs/agent-workflows.md
git commit -m "feat: add packaged agent workflow assets"
```

## Task 3: Replace Static Installer Targets With Plugin Registry

**Files:**
- Create: `src/hieronymus/agent_plugins/base.py`
- Create: `src/hieronymus/agent_plugins/__init__.py`
- Modify: `src/hieronymus/config.py`
- Modify: `src/hieronymus/install.py`
- Create: `tests/test_agent_plugins.py`
- Modify: `tests/test_install.py`

- [ ] **Step 1: Write failing plugin registry tests**

Create `tests/test_agent_plugins.py`:

```python
from __future__ import annotations

from pathlib import Path

from hieronymus.agent_plugins import available_plugins, resolve_plugin
from hieronymus.config import HieronymusConfig


def test_available_plugins_include_supported_targets() -> None:
    assert [plugin.name for plugin in available_plugins()] == [
        "claude",
        "codex",
        "openclaw",
        "opencode",
        "gemini",
        "pi",
        "hermes",
    ]


def test_resolve_plugin_returns_provider() -> None:
    assert resolve_plugin("codex").display_name == "Codex"


def test_config_has_agent_plugins_root(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    assert config.agent_plugins_root == tmp_path / "hieronymus" / "agent-plugins"
```

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
uv run pytest tests/test_agent_plugins.py -v
```

Expected:

```text
FAILED tests/test_agent_plugins.py::test_available_plugins_include_supported_targets
FAILED tests/test_agent_plugins.py::test_resolve_plugin_returns_provider
FAILED tests/test_agent_plugins.py::test_config_has_agent_plugins_root
```

- [ ] **Step 3: Add config helper**

Modify `src/hieronymus/config.py`:

```python
    @property
    def agent_plugins_root(self) -> Path:
        return self.config_root / "agent-plugins"
```

- [ ] **Step 4: Add base plugin models**

Create `src/hieronymus/agent_plugins/base.py`:

```python
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from hieronymus.config import HieronymusConfig


AGENT_WORKFLOW_SPEC = "docs/superpowers/specs/2026-06-06-hieronymus-agent-workflows.md"


@dataclass(frozen=True)
class AgentAvailability:
    target: str
    display_name: str
    available: bool
    installed: bool
    detect_paths: list[str]
    config_paths: list[str]
    install_path: str
    reason: str

    def to_json_dict(self) -> dict[str, object]:
        return {
            "target": self.target,
            "display_name": self.display_name,
            "available": self.available,
            "installed": self.installed,
            "detect_paths": self.detect_paths,
            "config_paths": self.config_paths,
            "install_path": self.install_path,
            "reason": self.reason,
        }


@dataclass(frozen=True)
class InstallStep:
    action: str
    path: str
    description: str

    def to_json_dict(self) -> dict[str, str]:
        return {"action": self.action, "path": self.path, "description": self.description}


@dataclass(frozen=True)
class InstallPlan:
    target: str
    display_name: str
    protocol_note: str
    docs: str
    result_kind: str
    steps: list[InstallStep]
    availability: AgentAvailability

    def to_json_dict(self) -> dict[str, object]:
        return {
            "target": self.target,
            "display_name": self.display_name,
            "protocol_note": self.protocol_note,
            "docs": self.docs,
            "result_kind": self.result_kind,
            "steps": [step.to_json_dict() for step in self.steps],
            "availability": self.availability.to_json_dict(),
        }


class AgentPlugin(Protocol):
    name: str
    display_name: str
    detect_paths: tuple[str, ...]
    config_paths: tuple[str, ...]
    docs: str

    def availability(self, config: HieronymusConfig) -> AgentAvailability:
        raise NotImplementedError

    def plan(self, config: HieronymusConfig) -> InstallPlan:
        raise NotImplementedError

    def install(self, config: HieronymusConfig, *, force: bool = False) -> InstallPlan:
        raise NotImplementedError


def expand_user(path: str) -> Path:
    return Path(path).expanduser()


def any_path_exists(paths: tuple[str, ...]) -> bool:
    return any(expand_user(path).exists() for path in paths)
```

- [ ] **Step 5: Add provider discovery**

Create `src/hieronymus/agent_plugins/__init__.py`:

```python
from __future__ import annotations

from hieronymus.agent_plugins.base import AgentPlugin


def available_plugins() -> list[AgentPlugin]:
    from hieronymus.agent_plugins.claude import ClaudePlugin
    from hieronymus.agent_plugins.codex import CodexPlugin
    from hieronymus.agent_plugins.gemini import GeminiPlugin
    from hieronymus.agent_plugins.openclaw import OpenClawPlugin
    from hieronymus.agent_plugins.opencode import OpenCodePlugin
    from hieronymus.agent_plugins.reserved import HermesPlugin, PiPlugin

    return [
        ClaudePlugin(),
        CodexPlugin(),
        OpenClawPlugin(),
        OpenCodePlugin(),
        GeminiPlugin(),
        PiPlugin(),
        HermesPlugin(),
    ]


def resolve_plugin(name: str) -> AgentPlugin:
    normalized = name.lower()
    for plugin in available_plugins():
        if plugin.name == normalized:
            return plugin
    supported = ", ".join(plugin.name for plugin in available_plugins())
    raise ValueError(f"unknown install target: {name}; supported targets: {supported}")
```

- [ ] **Step 6: Temporarily add provider shell modules**

Create `src/hieronymus/agent_plugins/codex.py`, `claude.py`, `openclaw.py`, `opencode.py`, `gemini.py`, and `reserved.py` using this shape; later tasks fill in installation behavior:

```python
from __future__ import annotations

from hieronymus.agent_plugins.base import (
    AGENT_WORKFLOW_SPEC,
    AgentAvailability,
    InstallPlan,
    InstallStep,
    any_path_exists,
)
from hieronymus.config import HieronymusConfig


class CodexPlugin:
    name = "codex"
    display_name = "Codex"
    detect_paths = ("~/.codex",)
    config_paths = ("~/.codex/config.toml",)
    docs = AGENT_WORKFLOW_SPEC

    def availability(self, config: HieronymusConfig) -> AgentAvailability:
        install_path = config.agent_plugins_root / self.name
        available = any_path_exists(self.detect_paths)
        installed = install_path.exists()
        return AgentAvailability(
            target=self.name,
            display_name=self.display_name,
            available=available,
            installed=installed,
            detect_paths=list(self.detect_paths),
            config_paths=list(self.config_paths),
            install_path=str(install_path),
            reason="host detected" if available else "host config path not found",
        )

    def plan(self, config: HieronymusConfig) -> InstallPlan:
        return InstallPlan(
            target=self.name,
            display_name=self.display_name,
            protocol_note="Codex integration installs Hieronymus skills, MCP config, and hooks.",
            docs=self.docs,
            result_kind="installable",
            availability=self.availability(config),
            steps=[
                InstallStep("write-assets", str(config.agent_plugins_root / self.name), "Write plugin assets."),
                InstallStep("patch-config", self.config_paths[0], "Register the Hieronymus plugin."),
            ],
        )

    def install(self, config: HieronymusConfig, *, force: bool = False) -> InstallPlan:
        return self.plan(config)
```

For the other providers, use these values:

```python
("claude", "Claude Code / Claude Desktop", ("~/.claude", "~/.claude.json"), ("~/.claude.json",))
("openclaw", "OpenClaw", ("~/.openclaw",), ("~/.openclaw/openclaw.json",))
("opencode", "opencode", ("~/.config/opencode",), ("~/.config/opencode/plugin.json",))
("gemini", "Gemini CLI", ("~/.gemini",), ("~/.gemini/settings.json",))
("pi", "Pi", ("~/.pi",), ("~/.pi/config.json",))
("hermes", "Hermes", ("~/.hermes",), ("~/.hermes/config.json",))
```

- [ ] **Step 7: Update install compatibility wrapper**

Modify `src/hieronymus/install.py` so existing imports still work:

```python
from hieronymus.agent_plugins import available_plugins, resolve_plugin
from hieronymus.agent_plugins.base import (
    InstallPlan,
    InstallStep,
    atomic_write_text,
    backup_file,
)

def known_targets() -> list[str]:
    return [plugin.name for plugin in available_plugins()]


def resolve_target(name: str):
    return resolve_plugin(name)


def plan_install(config: HieronymusConfig, target_name: str) -> InstallPlan:
    return resolve_plugin(target_name).plan(config)
```

This keeps `atomic_write_text()` and `backup_file()` re-exported from `install.py` for compatibility
until all existing imports have moved to `agent_plugins/base.py`.

- [ ] **Step 8: Run tests and commit**

Run:

```bash
uv run pytest tests/test_agent_plugins.py tests/test_install.py -v
```

Expected:

```text
13 passed
```

Commit:

```bash
git add src/hieronymus/config.py src/hieronymus/install.py src/hieronymus/agent_plugins tests/test_agent_plugins.py tests/test_install.py
git commit -m "feat: add plugin-style agent installer registry"
```

## Task 4: Add Installer Status, Candidate Listing, and CLI Wiring

**Files:**
- Modify: `src/hieronymus/cli.py`
- Modify: `src/hieronymus/install.py`
- Create: `tests/test_cli_agent_install.py`

- [ ] **Step 1: Write failing CLI tests for listing candidates and status**

Create `tests/test_cli_agent_install.py`:

```python
from __future__ import annotations

import json
from pathlib import Path

from click.testing import CliRunner

from hieronymus.cli import main


def test_install_without_app_lists_candidates_json(tmp_path: Path, monkeypatch) -> None:
    codex_home = tmp_path / "home" / ".codex"
    codex_home.mkdir(parents=True)
    monkeypatch.setenv("HOME", str(tmp_path / "home"))

    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "install", "--json"],
    )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    codex = next(item for item in payload["candidates"] if item["target"] == "codex")
    assert codex["available"] is True
    assert codex["installed"] is False


def test_install_list_human_output_marks_available_targets(tmp_path: Path, monkeypatch) -> None:
    (tmp_path / "home" / ".claude").mkdir(parents=True)
    monkeypatch.setenv("HOME", str(tmp_path / "home"))

    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "install", "list"],
    )

    assert result.exit_code == 0
    assert "Claude Code / Claude Desktop: available, not installed" in result.output
```

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
uv run pytest tests/test_cli_agent_install.py -v
```

Expected:

```text
FAILED tests/test_cli_agent_install.py::test_install_without_app_lists_candidates_json
FAILED tests/test_cli_agent_install.py::test_install_list_human_output_marks_available_targets
```

- [ ] **Step 3: Add list helper**

Modify `src/hieronymus/install.py`:

```python
def agent_install_candidates(config: HieronymusConfig) -> list[dict[str, object]]:
    return [plugin.availability(config).to_json_dict() for plugin in available_plugins()]
```

- [ ] **Step 4: Update `hiero install` argument behavior**

Modify the install command in `src/hieronymus/cli.py`:

```python
from hieronymus.install import agent_install_candidates, plan_install
```

Change the decorator and first lines:

```python
@main.command("install")
@click.argument("app", required=False)
@click.option("--dry-run", is_flag=True)
@click.option("--force", is_flag=True)
@click.option("--json", "as_json", is_flag=True)
@click.pass_context
def install_command(
    ctx: click.Context, app: str | None, dry_run: bool, force: bool, as_json: bool
) -> None:
    config = ctx.obj["config"]
    if app is None or app == "list":
        payload = {"candidates": agent_install_candidates(config)}
        if as_json:
            click.echo(render_json(payload))
            return
        click.echo(render_greeting())
        click.echo()
        click.echo("Agent integration candidates:")
        for item in payload["candidates"]:
            available = "available" if item["available"] else "not found"
            installed = "installed" if item["installed"] else "not installed"
            click.echo(f"- {item['display_name']}: {available}, {installed}")
        return
```

Keep the existing target-specific plan path after this block.

- [ ] **Step 5: Run tests and commit**

Run:

```bash
uv run pytest tests/test_cli_agent_install.py tests/test_cli_service.py::test_install_json_returns_stub_plan -v
```

Expected:

```text
15 passed
```

Commit:

```bash
git add src/hieronymus/cli.py src/hieronymus/install.py tests/test_cli_agent_install.py
git commit -m "feat: list agent installer candidates"
```

## Task 5: Add Safe Asset Writer and Idempotent Install Result

**Files:**
- Modify: `src/hieronymus/agent_plugins/base.py`
- Modify: `src/hieronymus/install.py`
- Create: `tests/test_agent_plugin_installers.py`

- [ ] **Step 1: Write failing tests for writing plugin assets**

Create `tests/test_agent_plugin_installers.py`:

```python
from __future__ import annotations

from pathlib import Path

from hieronymus.agent_plugins.base import write_plugin_assets
from hieronymus.config import HieronymusConfig


def test_write_plugin_assets_creates_expected_files(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    written = write_plugin_assets(
        config,
        "codex",
        {
            "skills/hieronymus-recall/SKILL.md": "recall",
            "mcp/hieronymus.mcp.json": "{}\n",
        },
    )

    assert written == [
        config.agent_plugins_root / "codex" / "skills" / "hieronymus-recall" / "SKILL.md",
        config.agent_plugins_root / "codex" / "mcp" / "hieronymus.mcp.json",
    ]
    assert written[0].read_text(encoding="utf-8") == "recall"


def test_write_plugin_assets_rejects_path_escape(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    try:
        write_plugin_assets(config, "codex", {"../escape": "bad"})
    except ValueError as error:
        assert "asset path escapes plugin root" in str(error)
    else:
        raise AssertionError("expected path escape rejection")
```

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
uv run pytest tests/test_agent_plugin_installers.py -v
```

Expected:

```text
FAILED tests/test_agent_plugin_installers.py::test_write_plugin_assets_creates_expected_files
FAILED tests/test_agent_plugin_installers.py::test_write_plugin_assets_rejects_path_escape
```

- [ ] **Step 3: Implement asset writer**

Modify `src/hieronymus/agent_plugins/base.py`:

```python
import os
import shutil
import tempfile
import time


def write_plugin_assets(
    config: HieronymusConfig,
    target: str,
    assets: dict[str, str],
) -> list[Path]:
    root = (config.agent_plugins_root / target).resolve()
    written: list[Path] = []
    for relative, text in assets.items():
        destination = (root / relative).resolve()
        if destination != root and root not in destination.parents:
            raise ValueError(f"asset path escapes plugin root: {relative}")
        atomic_write_text(destination, text)
        written.append(destination)
    return written


def atomic_write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary_path: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            "w",
            delete=False,
            dir=path.parent,
            encoding="utf-8",
            prefix=f".{path.name}.",
            suffix=".tmp",
        ) as temporary:
            temporary_path = Path(temporary.name)
            temporary.write(text)
            temporary.flush()
            os.fsync(temporary.fileno())
        temporary_path.replace(path)
    except Exception:
        if temporary_path is not None:
            temporary_path.unlink(missing_ok=True)
        raise


def backup_file(config: HieronymusConfig, source: Path, *, agent: str, extension: str) -> Path:
    config.backups_root.mkdir(parents=True, exist_ok=True)
    suffix = extension.removeprefix(".")
    for attempt in range(100):
        unique = f"{time.time_ns()}-{os.getpid()}-{attempt}"
        backup = config.backups_root / f"{agent}-{source.stem}-{unique}.{suffix}"
        try:
            descriptor = os.open(backup, os.O_WRONLY | os.O_CREAT | os.O_EXCL)
        except FileExistsError:
            continue
        else:
            os.close(descriptor)
            try:
                shutil.copy2(source, backup)
            except Exception:
                backup.unlink(missing_ok=True)
                raise
            return backup
    raise FileExistsError(f"could not create unique backup path for {source}")
```

- [ ] **Step 4: Add install result tests to Codex provider**

Append to `tests/test_agent_plugin_installers.py`:

```python
from hieronymus.agent_plugins import resolve_plugin


def test_codex_install_writes_assets_and_reports_installed(tmp_path: Path, monkeypatch) -> None:
    home = tmp_path / "home"
    (home / ".codex").mkdir(parents=True)
    (home / ".codex" / "config.toml").write_text("", encoding="utf-8")
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    plan = resolve_plugin("codex").install(config, force=False)

    assert plan.result_kind == "installed"
    assert (config.agent_plugins_root / "codex" / "skills" / "hieronymus-recall" / "SKILL.md").exists()
    assert resolve_plugin("codex").availability(config).installed is True
```

- [ ] **Step 5: Implement Codex asset install without config patch**

Modify `src/hieronymus/agent_plugins/codex.py`:

```python
from hieronymus.agent_assets import render_agent_plugin_assets
from hieronymus.agent_plugins.base import write_plugin_assets


    def install(self, config: HieronymusConfig, *, force: bool = False) -> InstallPlan:
        write_plugin_assets(config, self.name, render_agent_plugin_assets(self.name))
        plan = self.plan(config)
        return InstallPlan(
            target=plan.target,
            display_name=plan.display_name,
            protocol_note=plan.protocol_note,
            docs=plan.docs,
            result_kind="installed",
            steps=plan.steps,
            availability=self.availability(config),
        )
```

- [ ] **Step 6: Run tests and commit**

Run:

```bash
uv run pytest tests/test_agent_plugin_installers.py -v
```

Expected:

```text
3 passed
```

Commit:

```bash
git add src/hieronymus/agent_plugins/base.py src/hieronymus/agent_plugins/codex.py tests/test_agent_plugin_installers.py
git commit -m "feat: write agent plugin assets safely"
```

## Task 6: Install Codex and Claude Plugin Configurations

**Files:**
- Modify: `pyproject.toml`
- Modify: `src/hieronymus/agent_plugins/base.py`
- Modify: `src/hieronymus/agent_plugins/codex.py`
- Modify: `src/hieronymus/agent_plugins/claude.py`
- Modify: `src/hieronymus/cli.py`
- Modify: `tests/test_agent_plugin_installers.py`
- Modify: `tests/test_cli_agent_install.py`

- [ ] **Step 1: Add TOML writer dependency**

Run:

```bash
uv add tomli-w
```

Expected:

```text
Resolved dependencies and updated pyproject.toml and uv.lock
```

- [ ] **Step 2: Write failing tests for Codex TOML patch**

Append to `tests/test_agent_plugin_installers.py`:

```python
import tomllib


def test_codex_install_patches_toml_mcp_and_plugin(tmp_path: Path, monkeypatch) -> None:
    home = tmp_path / "home"
    codex = home / ".codex"
    codex.mkdir(parents=True)
    config_path = codex / "config.toml"
    config_path.write_text("[profile]\nname = \"default\"\n", encoding="utf-8")
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    resolve_plugin("codex").install(config)

    payload = tomllib.loads(config_path.read_text(encoding="utf-8"))
    assert payload["mcp_servers"]["hieronymus"]["command"] == "hieronymus-mcp"
    assert payload["plugins"]["hieronymus"]["path"] == str(config.agent_plugins_root / "codex")
    assert payload["hieronymus"]["managed"] is True
```

- [ ] **Step 3: Write failing tests for Claude JSON patch**

Append; if the file does not already import `json`, add `import json` at the top:

```python
import json


def test_claude_install_patches_json_config(tmp_path: Path, monkeypatch) -> None:
    home = tmp_path / "home"
    (home / ".claude").mkdir(parents=True)
    claude_json = home / ".claude.json"
    claude_json.write_text("{}", encoding="utf-8")
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    plan = resolve_plugin("claude").install(config)

    payload = json.loads(claude_json.read_text(encoding="utf-8"))
    assert plan.result_kind == "installed"
    assert payload["mcpServers"]["hieronymus"]["command"] == "hieronymus-mcp"
    assert payload["hieronymus"]["pluginPath"] == str(config.agent_plugins_root / "claude")
```

- [ ] **Step 4: Run tests and verify they fail**

Run:

```bash
uv run pytest tests/test_agent_plugin_installers.py::test_codex_install_patches_toml_mcp_and_plugin tests/test_agent_plugin_installers.py::test_claude_install_patches_json_config -v
```

Expected both tests fail because configs are not patched.

- [ ] **Step 5: Add safe JSON/TOML patch helpers**

Modify `src/hieronymus/agent_plugins/base.py`:

```python
import json
import tomllib
import tomli_w
from typing import Any

def load_json_object(path: Path) -> dict[str, Any]:
    if not path.exists() or path.read_text(encoding="utf-8").strip() == "":
        return {}
    payload = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(payload, dict):
        raise ValueError(f"expected JSON object in {path}")
    return payload


def patch_json_config(
    config: HieronymusConfig,
    path: Path,
    *,
    agent: str,
    payload: dict[str, Any],
) -> None:
    if path.exists():
        backup_file(config, path, agent=agent, extension=".json")
    atomic_write_text(path, json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n")


def load_toml_object(path: Path) -> dict[str, Any]:
    if not path.exists() or path.read_text(encoding="utf-8").strip() == "":
        return {}
    return tomllib.loads(path.read_text(encoding="utf-8"))


def patch_toml_config(
    config: HieronymusConfig,
    path: Path,
    *,
    agent: str,
    payload: dict[str, Any],
) -> None:
    if path.exists():
        backup_file(config, path, agent=agent, extension=".toml")
    atomic_write_text(path, tomli_w.dumps(payload))
```

- [ ] **Step 6: Implement Codex config patch**

Modify `src/hieronymus/agent_plugins/codex.py`:

```python
from hieronymus.agent_plugins.base import expand_user, load_toml_object, patch_toml_config


    def install(self, config: HieronymusConfig, *, force: bool = False) -> InstallPlan:
        write_plugin_assets(config, self.name, render_agent_plugin_assets(self.name))
        config_path = expand_user(self.config_paths[0])
        payload = load_toml_object(config_path)
        payload.setdefault("mcp_servers", {})["hieronymus"] = {"command": "hieronymus-mcp", "args": []}
        payload.setdefault("plugins", {})["hieronymus"] = {
            "path": str(config.agent_plugins_root / self.name)
        }
        payload["hieronymus"] = {"managed": True, "version": "0.1.0"}
        patch_toml_config(config, config_path, agent=self.name, payload=payload)
        plan = self.plan(config)
        return InstallPlan(
            target=plan.target,
            display_name=plan.display_name,
            protocol_note=plan.protocol_note,
            docs=plan.docs,
            result_kind="installed",
            steps=plan.steps,
            availability=self.availability(config),
        )
```

- [ ] **Step 7: Implement Claude config patch**

Modify `src/hieronymus/agent_plugins/claude.py`:

```python
from hieronymus.agent_assets import render_agent_plugin_assets
from hieronymus.agent_plugins.base import (
    InstallPlan,
    expand_user,
    load_json_object,
    patch_json_config,
    write_plugin_assets,
)


    def install(self, config: HieronymusConfig, *, force: bool = False) -> InstallPlan:
        write_plugin_assets(config, self.name, render_agent_plugin_assets(self.name))
        config_path = expand_user("~/.claude.json")
        payload = load_json_object(config_path)
        payload.setdefault("mcpServers", {})["hieronymus"] = {"command": "hieronymus-mcp", "args": []}
        payload["hieronymus"] = {
            "managed": True,
            "version": "0.1.0",
            "pluginPath": str(config.agent_plugins_root / self.name),
        }
        patch_json_config(config, config_path, agent=self.name, payload=payload)
        plan = self.plan(config)
        return InstallPlan(
            target=plan.target,
            display_name=plan.display_name,
            protocol_note=plan.protocol_note,
            docs=plan.docs,
            result_kind="installed",
            steps=plan.steps,
            availability=self.availability(config),
        )
```

- [ ] **Step 8: Make CLI perform install unless `--dry-run`**

Modify target-specific branch in `src/hieronymus/cli.py`:

```python
from hieronymus.agent_plugins import resolve_plugin

# replace the existing `plan = plan_install(...)` line in the target-specific branch
    plugin = resolve_plugin(app)
    plan = plugin.plan(ctx.obj["config"]) if dry_run else plugin.install(ctx.obj["config"], force=force)
```

Keep JSON/human rendering unchanged.

- [ ] **Step 9: Run tests and commit**

Run:

```bash
uv run pytest tests/test_agent_plugin_installers.py tests/test_cli_agent_install.py tests/test_cli_service.py -v
```

Expected all selected tests pass.

Commit:

```bash
git add pyproject.toml uv.lock src/hieronymus/agent_plugins src/hieronymus/cli.py tests/test_agent_plugin_installers.py tests/test_cli_agent_install.py
git commit -m "feat: install codex and claude agent plugins"
```

## Task 7: Install OpenClaw, opencode, and Gemini Plugin Configurations

**Files:**
- Modify: `src/hieronymus/agent_plugins/openclaw.py`
- Modify: `src/hieronymus/agent_plugins/opencode.py`
- Modify: `src/hieronymus/agent_plugins/gemini.py`
- Modify: `tests/test_agent_plugin_installers.py`

- [ ] **Step 1: Add failing tests for OpenClaw JSON patch**

Append to `tests/test_agent_plugin_installers.py`:

```python
def test_openclaw_install_patches_json_config(tmp_path: Path, monkeypatch) -> None:
    home = tmp_path / "home"
    openclaw = home / ".openclaw"
    openclaw.mkdir(parents=True)
    config_path = openclaw / "openclaw.json"
    config_path.write_text("{}", encoding="utf-8")
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    resolve_plugin("openclaw").install(config)

    payload = json.loads(config_path.read_text(encoding="utf-8"))
    assert payload["plugins"]["hieronymus"]["path"] == str(config.agent_plugins_root / "openclaw")
    assert payload["mcpServers"]["hieronymus"]["command"] == "hieronymus-mcp"
```

- [ ] **Step 2: Add failing tests for opencode plugin config**

Append:

```python
def test_opencode_install_writes_plugin_json(tmp_path: Path, monkeypatch) -> None:
    home = tmp_path / "home"
    opencode = home / ".config" / "opencode"
    opencode.mkdir(parents=True)
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    resolve_plugin("opencode").install(config)

    payload = json.loads((opencode / "plugin.json").read_text(encoding="utf-8"))
    assert payload["plugins"]["hieronymus"]["path"] == str(config.agent_plugins_root / "opencode")
```

- [ ] **Step 3: Add failing tests for Gemini settings patch**

Append:

```python
def test_gemini_install_patches_settings_json(tmp_path: Path, monkeypatch) -> None:
    home = tmp_path / "home"
    gemini = home / ".gemini"
    gemini.mkdir(parents=True)
    settings = gemini / "settings.json"
    settings.write_text("{}", encoding="utf-8")
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    resolve_plugin("gemini").install(config)

    payload = json.loads(settings.read_text(encoding="utf-8"))
    assert payload["mcpServers"]["hieronymus"]["command"] == "hieronymus-mcp"
    assert payload["extensions"]["hieronymus"]["path"] == str(config.agent_plugins_root / "gemini")
```

- [ ] **Step 4: Run tests and verify they fail**

Run:

```bash
uv run pytest tests/test_agent_plugin_installers.py::test_openclaw_install_patches_json_config tests/test_agent_plugin_installers.py::test_opencode_install_writes_plugin_json tests/test_agent_plugin_installers.py::test_gemini_install_patches_settings_json -v
```

Expected all three tests fail because these providers still only plan.

- [ ] **Step 5: Implement the three JSON installers**

Use this pattern in each provider:

```python
from hieronymus.agent_assets import render_agent_plugin_assets
from hieronymus.agent_plugins.base import (
    InstallPlan,
    expand_user,
    load_json_object,
    patch_json_config,
    write_plugin_assets,
)


    def install(self, config: HieronymusConfig, *, force: bool = False) -> InstallPlan:
        write_plugin_assets(config, self.name, render_agent_plugin_assets(self.name))
        config_path = expand_user(self.config_paths[0])
        payload = load_json_object(config_path)
        payload.setdefault("mcpServers", {})["hieronymus"] = {"command": "hieronymus-mcp", "args": []}
        payload.setdefault("plugins", {})["hieronymus"] = {
            "path": str(config.agent_plugins_root / self.name)
        }
        payload["hieronymus"] = {"managed": True, "version": "0.1.0"}
        patch_json_config(config, config_path, agent=self.name, payload=payload)
        plan = self.plan(config)
        return InstallPlan(
            target=plan.target,
            display_name=plan.display_name,
            protocol_note=plan.protocol_note,
            docs=plan.docs,
            result_kind="installed",
            steps=plan.steps,
            availability=self.availability(config),
        )
```

For Gemini, write
`payload.setdefault("extensions", {})["hieronymus"] = {"path": str(config.agent_plugins_root / self.name)}`
instead of `plugins`.

- [ ] **Step 6: Run tests and commit**

Run:

```bash
uv run pytest tests/test_agent_plugin_installers.py -v
```

Expected all installer tests pass.

Commit:

```bash
git add src/hieronymus/agent_plugins/openclaw.py src/hieronymus/agent_plugins/opencode.py src/hieronymus/agent_plugins/gemini.py tests/test_agent_plugin_installers.py
git commit -m "feat: install openclaw opencode and gemini plugins"
```

## Task 8: Add Agent Hook Context and Event Recorder

**Files:**
- Create: `src/hieronymus/agent_context.py`
- Create: `src/hieronymus/agent_hooks.py`
- Modify: `pyproject.toml`
- Create: `tests/test_agent_context.py`
- Create: `tests/test_agent_hooks.py`

- [ ] **Step 1: Write failing context discovery tests**

Create `tests/test_agent_context.py`:

```python
from __future__ import annotations

import json
from pathlib import Path

from hieronymus.agent_context import discover_project_context


def test_discover_project_context_reads_hieronymus_json(tmp_path: Path) -> None:
    (tmp_path / ".hieronymus.json").write_text(
        json.dumps(
            {
                "series_slug": "oso",
                "source_language": "ja",
                "target_language": "en",
                "volume": "1",
                "chapter": "2",
            }
        ),
        encoding="utf-8",
    )

    context = discover_project_context(tmp_path)

    assert context.series_slug == "oso"
    assert context.source_language == "ja"
    assert context.target_language == "en"
```

- [ ] **Step 2: Write failing hook tests**

Create `tests/test_agent_hooks.py`:

```python
from __future__ import annotations

import json
from pathlib import Path

from click.testing import CliRunner

from hieronymus.agent_hooks import main


def test_hook_session_start_outputs_missing_context_when_no_project_file(tmp_path: Path) -> None:
    result = CliRunner().invoke(main, ["session-start", "--cwd", str(tmp_path), "--json"])

    assert result.exit_code == 0
    assert json.loads(result.output) == {
        "event": "session-start",
        "handled": False,
        "reason": "no .hieronymus.json context found",
    }
```

- [ ] **Step 3: Run tests and verify they fail**

Run:

```bash
uv run pytest tests/test_agent_context.py tests/test_agent_hooks.py -v
```

Expected both files fail because modules do not exist.

- [ ] **Step 4: Implement context discovery**

Create `src/hieronymus/agent_context.py`:

```python
from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class ProjectAgentContext:
    series_slug: str
    source_language: str = "ja"
    target_language: str = "en"
    task_type: str = "translation"
    volume: str = ""
    chapter: str = ""


def discover_project_context(cwd: Path) -> ProjectAgentContext | None:
    current = cwd.resolve()
    for path in (current, *current.parents):
        candidate = path / ".hieronymus.json"
        if not candidate.exists():
            continue
        payload = json.loads(candidate.read_text(encoding="utf-8"))
        return ProjectAgentContext(
            series_slug=str(payload["series_slug"]),
            source_language=str(payload.get("source_language", "ja")),
            target_language=str(payload.get("target_language", "en")),
            task_type=str(payload.get("task_type", "translation")),
            volume=str(payload.get("volume", "")),
            chapter=str(payload.get("chapter", "")),
        )
    return None
```

- [ ] **Step 5: Implement hook CLI**

Create `src/hieronymus/agent_hooks.py`:

```python
from __future__ import annotations

from pathlib import Path

import click

from hieronymus.agent_context import discover_project_context
from hieronymus.presentation import render_json


@click.group()
def main() -> None:
    pass


@main.command("session-start")
@click.option("--cwd", type=click.Path(file_okay=False, dir_okay=True), default=".")
@click.option("--json", "as_json", is_flag=True)
def session_start(cwd: str, as_json: bool) -> None:
    context = discover_project_context(Path(cwd))
    if context is None:
        payload = {
            "event": "session-start",
            "handled": False,
            "reason": "no .hieronymus.json context found",
        }
    else:
        payload = {
            "event": "session-start",
            "handled": True,
            "series_slug": context.series_slug,
            "source_language": context.source_language,
            "target_language": context.target_language,
        }
    click.echo(render_json(payload) if as_json else payload["reason"] if not payload["handled"] else "Hieronymus context loaded")


@main.command("session-end")
@click.option("--json", "as_json", is_flag=True)
def session_end(as_json: bool) -> None:
    payload = {"event": "session-end", "handled": True}
    click.echo(render_json(payload) if as_json else "Hieronymus session hook complete")


if __name__ == "__main__":
    main()
```

- [ ] **Step 6: Add console script alias**

Modify `pyproject.toml`:

```toml
hieronymus-agent-hook = "hieronymus.agent_hooks:main"
```

- [ ] **Step 7: Run tests and commit**

Run:

```bash
uv run pytest tests/test_agent_context.py tests/test_agent_hooks.py -v
```

Expected:

```text
2 passed
```

Commit:

```bash
git add pyproject.toml src/hieronymus/agent_context.py src/hieronymus/agent_hooks.py tests/test_agent_context.py tests/test_agent_hooks.py
git commit -m "feat: add agent hook context entry point"
```

## Task 9: Add Doctor Checks and Docs for Agent Integrations

**Files:**
- Modify: `src/hieronymus/doctor.py`
- Modify: `docs/agent-workflows.md`
- Modify: `docs/service-toolkit.md`
- Create: `tests/test_doctor_agent_plugins.py`

- [ ] **Step 1: Write failing doctor tests**

Create `tests/test_doctor_agent_plugins.py`:

```python
from __future__ import annotations

from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.doctor import Doctor, report_to_json


def test_doctor_reports_available_uninstalled_agent(tmp_path: Path, monkeypatch) -> None:
    home = tmp_path / "home"
    (home / ".codex").mkdir(parents=True)
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    report = Doctor(config).run()
    payload = report_to_json(report)

    assert any(
        finding["code"] == "agent-plugin-available"
        and "Codex is available but Hieronymus is not installed" in finding["message"]
        for finding in payload["warnings"]
    )
```

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
uv run pytest tests/test_doctor_agent_plugins.py -v
```

Expected the test fails because doctor has no agent plugin checks.

- [ ] **Step 3: Add doctor integration checks**

Modify `src/hieronymus/doctor.py`:

```python
from hieronymus.agent_plugins import available_plugins
```

Add a call in `Doctor.run()`:

```python
self._check_agent_plugins(report)
```

Add method:

```python
    def _check_agent_plugins(self, report: DoctorReport) -> None:
        for plugin in available_plugins():
            availability = plugin.availability(self.config)
            if availability.available and not availability.installed:
                report.warnings.append(
                    DoctorFinding(
                        code="agent-plugin-available",
                        severity="warning",
                        message=(
                            f"{availability.display_name} is available but Hieronymus is not installed"
                        ),
                        fix="Run hiero install "
                        + availability.target
                        + " --dry-run, then hiero install "
                        + availability.target,
                    )
                )
```

- [ ] **Step 4: Update docs**

Append to `docs/agent-workflows.md`:

```markdown
## Installing Agent Integrations

Use `hiero install --json` or `hiero install list` to see available agent hosts and whether the
Hieronymus plugin is installed.

Use `hiero install codex --dry-run` to inspect planned changes. Use `hiero install codex` to write
plugin assets and patch the host config with backups under `~/.config/hieronymus/backups`.

Supported targets in this pass:

- `claude`
- `codex`
- `openclaw`
- `opencode`
- `gemini`

Reserved detectable targets:

- `pi`
- `hermes`
```

Append to `docs/service-toolkit.md`:

```markdown
`hiero doctor` also reports detected agent hosts where Hieronymus has not been installed yet.
```

- [ ] **Step 5: Run tests and commit**

Run:

```bash
uv run pytest tests/test_doctor_agent_plugins.py tests/test_doctor.py -v
```

Expected all selected doctor tests pass.

Commit:

```bash
git add src/hieronymus/doctor.py tests/test_doctor_agent_plugins.py docs/agent-workflows.md docs/service-toolkit.md
git commit -m "feat: report agent integration status in doctor"
```

## Task 10: End-to-End Verification and Boundary Review

**Files:**
- Modify only if verification exposes a real issue.

- [ ] **Step 1: Run full test suite**

Run:

```bash
uv run pytest
```

Expected:

```text
all tests passed
```

- [ ] **Step 2: Run lint and format gates**

Run:

```bash
uv run ruff check .
uv run ruff format --check .
```

Expected:

```text
All checks passed!
```

and:

```text
N files already formatted
```

- [ ] **Step 3: Run boundary searches**

Run:

```bash
rg -n "auto-approve|approved automatically|bypass MCP|raw source text into long-term" src tests docs
rg -n "TODO|TBD|implement later|stub; real integration is deferred" src/hieronymus docs/agent-workflows.md
```

Expected:

- no claims that agents auto-approve strict terminology
- no integration path that writes memory without MCP/service APIs
- no stale installer-stub wording for Claude/Codex/OpenClaw/opencode/Gemini

- [ ] **Step 4: Manual smoke test with temporary HOME**

Run:

```bash
tmp_home="$(mktemp -d)"
mkdir -p "$tmp_home/.codex"
printf '[profile]\nname = "default"\n' > "$tmp_home/.codex/config.toml"
HOME="$tmp_home" uv run hiero --data-root "$tmp_home/.config/hieronymus" install codex --json
HOME="$tmp_home" uv run hiero --data-root "$tmp_home/.config/hieronymus" install --json
```

Expected:

- first command reports `"result_kind": "installed"`
- second command shows Codex with `"available": true` and `"installed": true`

- [ ] **Step 5: Request final code review**

Use `superpowers:requesting-code-review` with this review scope:

```text
Review the agent workflows implementation against docs/superpowers/specs/2026-06-06-hieronymus-agent-workflows.md.
Focus on:
- plugin-style installer extensibility
- host availability and installed detection
- safe config writes/backups
- MCP/service boundary preservation
- learn vs read behavior
- strict terminology not being auto-approved
```

- [ ] **Step 6: Commit any review fixes**

If review finds issues, fix them with focused tests and commits. If no issues, do not create an empty commit.
