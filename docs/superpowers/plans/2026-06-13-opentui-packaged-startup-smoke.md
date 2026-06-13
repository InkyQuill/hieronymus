# OpenTUI Packaged Startup Smoke Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add real-process smoke checks proving the built OpenTUI bundle can start `hiero config` and `hiero admin` through the packaged `frontend/dist/main.js` path.

**Architecture:** Keep existing mocked CLI launcher tests as fast unit coverage. Add narrowly scoped Python smoke tests that spawn Bun with the built frontend bundle under a real PTY, bridge back into the current Python environment with `python -m hieronymus tui-bridge`, wait for each screen title, send `q`, and assert clean exit. Tests skip cleanly when Bun, PTY support, or the built bundle is unavailable; local verification rebuilds `frontend/dist/main.js` before running the smoke tests so it exercises the current source.

**Tech Stack:** Python 3.12, pytest, standard-library `pty`/`select`/`subprocess`, Bun, built OpenTUI bundle at `frontend/dist/main.js`.

---

## Current Code Map

- `src/hieronymus/cli.py`: `_frontend_entrypoint()` locates `frontend/dist/main.js`; `_launch_opentui()` runs `bun <bundle> <admin|config> --bridge-command <python> --bridge-arg -m --bridge-arg hieronymus`.
- `frontend/src/main.tsx`: starts the OpenTUI React renderer, creates a JSON-RPC bridge client, and renders `App`.
- `frontend/src/app/App.tsx`: requests either `config.bootstrap` or `admin.bootstrap`, then renders `ConfigScreen` or `AdminScreen`.
- `frontend/src/config/ConfigScreen.tsx`: renders `Hieronymus Config` and exits on `q`.
- `frontend/src/admin/AdminScreen.tsx`: renders `Hieronymus Admin` through its header and exits on `q`.
- `tests/test_cli_opentui.py`: already covers JSON-RPC dispatch, bundle lookup, mocked `_launch_opentui()`, command construction, and clean launcher errors. It has no real-process smoke coverage for the built bundle.
- `frontend/dist/main.js`: built artifact used by the CLI and wheel packaging. It may be absent in fresh source checkouts, so the smoke tests must skip when it is missing and verification must build it before expecting local smoke execution.
- `docs/roadmap.md`: OpenTUI remaining work includes real-process smoke checks for packaged `config` and `admin` startup.

---

### Task 1: Add Real-Process OpenTUI Smoke Harness

**Files:**
- Modify: `tests/test_cli_opentui.py`

- [ ] **Step 1: Write failing smoke tests**

In `tests/test_cli_opentui.py`, add these imports near the existing imports:

```python
import fcntl
import os
import pty
import re
import select
import shutil
import struct
import termios
import time
```

Add these helpers below the existing `_frontend_entrypoint` tests and above `test_cli_config_launches_opentui_when_tui_env_unset`:

```python
ANSI_RE = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")


def _repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def _built_frontend_bundle() -> Path:
    return _repo_root() / "frontend" / "dist" / "main.js"


def _require_opentui_smoke_runtime() -> Path:
    if os.name != "posix":
        pytest.skip("OpenTUI smoke tests require a POSIX PTY")
    if shutil.which("bun") is None:
        pytest.skip("OpenTUI smoke tests require Bun")
    bundle = _built_frontend_bundle()
    if not bundle.exists():
        pytest.skip("OpenTUI smoke tests require frontend/dist/main.js")
    return bundle


def _strip_ansi(text: str) -> str:
    return ANSI_RE.sub("", text)


def _read_pty_until(fd: int, expected: str, *, timeout: float = 8.0) -> str:
    deadline = time.monotonic() + timeout
    chunks: list[bytes] = []
    while time.monotonic() < deadline:
        readable, _, _ = select.select([fd], [], [], 0.1)
        if not readable:
            continue
        try:
            chunk = os.read(fd, 4096)
        except OSError:
            break
        if not chunk:
            break
        chunks.append(chunk)
        text = _strip_ansi(b"".join(chunks).decode(errors="replace"))
        if expected in text:
            return text
    text = _strip_ansi(b"".join(chunks).decode(errors="replace"))
    raise AssertionError(f"did not see {expected!r} in OpenTUI output:\n{text}")


def _smoke_opentui_bundle(mode: str, expected_title: str, tmp_path: Path) -> str:
    bundle = _require_opentui_smoke_runtime()
    master_fd, slave_fd = pty.openpty()
    try:
        _set_pty_size(slave_fd, rows=36, columns=120)
    except OSError as error:
        os.close(master_fd)
        os.close(slave_fd)
        pytest.skip(f"OpenTUI smoke tests could not size PTY: {error}")
    data_root = tmp_path / "hieronymus"
    env = {
        **os.environ,
        "HIERONYMUS_DATA_ROOT": str(data_root),
        "TERM": os.environ.get("TERM", "xterm-256color"),
    }
    command = [
        "bun",
        str(bundle),
        mode,
        "--bridge-command",
        sys.executable,
        "--bridge-arg",
        "-m",
        "--bridge-arg",
        "hieronymus",
    ]
    process = subprocess.Popen(
        command,
        stdin=slave_fd,
        stdout=slave_fd,
        stderr=slave_fd,
        cwd=_repo_root(),
        env=env,
        close_fds=True,
    )
    os.close(slave_fd)
    try:
        output = _read_pty_until(master_fd, expected_title)
        os.write(master_fd, b"q")
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.wait(timeout=5)
            raise AssertionError(f"OpenTUI {mode} did not exit after q")
        assert process.returncode == 0
        return output
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        os.close(master_fd)


def _set_pty_size(fd: int, *, rows: int, columns: int) -> None:
    size = struct.pack("HHHH", rows, columns, 0, 0)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, size)
```

Add these tests:

```python
def test_packaged_opentui_config_starts_from_built_bundle(tmp_path: Path) -> None:
    output = _smoke_opentui_bundle("config", "Hieronymus Config", tmp_path)

    assert "Providers" in output
    assert "dream.conf" in output


def test_packaged_opentui_admin_starts_from_built_bundle(tmp_path: Path) -> None:
    output = _smoke_opentui_bundle("admin", "Hieronymus Admin", tmp_path)

    assert "Views" in output
    assert "Status" in output
```

- [ ] **Step 2: Run focused tests to verify behavior**

Build the current frontend bundle first when Bun is available:

```bash
bun run --cwd frontend build
```

If Bun is unavailable, skip this build command and confirm the smoke tests skip.

Run:

```bash
uv run pytest tests/test_cli_opentui.py::test_packaged_opentui_config_starts_from_built_bundle tests/test_cli_opentui.py::test_packaged_opentui_admin_starts_from_built_bundle -q
```

Expected in a local checkout with Bun and `frontend/dist/main.js`: tests start the real bundle. They may initially fail if PTY output timing, expected text, terminal sizing, or exit handling needs adjustment.

Expected in an environment without Bun, without POSIX PTY support, or without `frontend/dist/main.js`: tests SKIP with a clear skip reason.

- [ ] **Step 3: Adjust harness minimally if the red run exposes real startup details**

If the tests fail because visible text differs, keep the smoke signal broad and update only the expected text. The accepted titles are:

```python
CONFIG_TITLE = "Hieronymus Config"
ADMIN_TITLE = "Hieronymus Admin"
```

If the tests fail because `q` does not exit after the title has rendered, keep that as a product bug and fix only the screen exit path that already handles `q`:

```typescript
if (key.name === "q") {
  client?.close();
  renderer.destroy();
}
```

Do not add sleeps except through `_read_pty_until()` polling. Do not add a fake bridge server; this smoke exists specifically to exercise `python -m hieronymus tui-bridge`.

If a platform raises from `fcntl.ioctl(..., termios.TIOCSWINSZ, ...)`, treat that as an environment limitation and skip the smoke test with a clear reason rather than leaving an unhandled startup failure.

```python
try:
    _set_pty_size(slave_fd, rows=36, columns=120)
except OSError as error:
    pytest.skip(f"OpenTUI smoke tests could not size PTY: {error}")
```

- [ ] **Step 4: Run full OpenTUI CLI tests**

Run:

```bash
bun run --cwd frontend build
uv run pytest tests/test_cli_opentui.py -q
```

Expected: PASS locally when Bun and `frontend/dist/main.js` are available, with the two new smoke tests passing. In backend-only CI without Bun, expected result is PASS with the two smoke tests skipped.

- [ ] **Step 5: Commit**

```bash
git add tests/test_cli_opentui.py
git commit -m "test: smoke packaged opentui startup"
```

---

### Task 2: Document Smoke Coverage Status

**Files:**
- Modify: `docs/roadmap.md`

- [ ] **Step 1: Update roadmap**

In `docs/roadmap.md`, move this OpenTUI remaining-work item out of Remaining work:

```markdown
- Add real-process smoke checks for packaged `config` and `admin` startup using
  `frontend/dist/main.js`, skipped cleanly when Bun or a PTY is unavailable.
```

Add this bullet to the OpenTUI `Completed baseline:` list:

```markdown
- Packaged `config` and `admin` startup have real-process smoke checks through
  `frontend/dist/main.js`, skipped cleanly when Bun, a POSIX PTY, or the built
  bundle is unavailable.
```

- [ ] **Step 2: Run docs diff check**

Run:

```bash
git diff -- docs/roadmap.md
```

Expected: only the packaged OpenTUI smoke-check item moves from remaining work to completed baseline.

- [ ] **Step 3: Commit**

```bash
git add docs/roadmap.md
git commit -m "docs: mark opentui startup smoke coverage"
```

---

### Task 3: Final Verification

**Files:**
- No source file changes expected unless verification exposes a defect.

- [ ] **Step 1: Run backend verification**

```bash
bun run --cwd frontend build
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

Expected: PASS. In environments without Bun, skip the build command and run `uv run pytest`; the new packaged OpenTUI smoke tests must skip cleanly.

- [ ] **Step 2: Run frontend verification**

```bash
bun install --cwd frontend --frozen-lockfile
bun run --cwd frontend format
bun run --cwd frontend typecheck
bun --cwd frontend test
bun run --cwd frontend build
```

Expected: PASS. Existing React `act(...)` and OpenTUI `TerminalConsoleCache` warnings may still appear in `bun --cwd frontend test`; this plan does not address that separate roadmap item.

- [ ] **Step 3: Run final diff check**

```bash
git diff --check
git status --short --branch
```

Expected: `git diff --check` prints nothing. `git status --short --branch` shows a clean branch.

---

## Self-Review

Spec coverage:

- Real-process smoke checks for packaged `config` startup: Task 1.
- Real-process smoke checks for packaged `admin` startup: Task 1.
- Skip cleanly when Bun, a PTY, or bundle is unavailable: Task 1.
- Rebuild current frontend bundle before local smoke execution: Tasks 1 and 3.
- Roadmap records completed work after implementation: Task 2.
- Full backend and frontend verification: Task 3.

Red-flag scan:

- No planning markers, shortcut references, or undefined helper names remain.
- All code helpers, tests, commands, expected skip/pass behavior, and commit commands are explicit.

Type consistency:

- Smoke helper names are `_built_frontend_bundle()`, `_require_opentui_smoke_runtime()`, `_read_pty_until()`, and `_smoke_opentui_bundle()`.
- Test names are `test_packaged_opentui_config_starts_from_built_bundle` and `test_packaged_opentui_admin_starts_from_built_bundle`.
- The launched frontend command matches `src/hieronymus/cli.py` launcher semantics: `bun frontend/dist/main.js <mode> --bridge-command <python> --bridge-arg -m --bridge-arg hieronymus`.
