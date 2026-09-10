# Hieronymus App Review

> Historical record: Python source and commands below belong to the
> [archived Python line](docs/archive/python-v0.7.0.md). Current release acquisition uses
> `bun scripts/stage-release-assets.ts`; these records are not active release gates.

Scope: full-repo pass focused on `src/hieronymus/` (Python backend/daemon), `frontend/src/web`
(Svelte + Tailwind console), packaging (`hatch_build.py`, `pyproject.toml`), and the shell
installer (`install.sh`). Reviewed at commit `11fdedc` on `agent/tailwind-web-console`.

Findings are ranked by severity within each section. Each includes file:line evidence.

---

## 1. Correctness / logic bugs

### 1.1 Crystal auto-archive rule is duplicated and silently diverges between the CLI and the web admin console (HIGH)

There are two independent implementations of "apply a feedback event to a crystal's
strength/confidence and decide whether to archive it":

- **Canonical path** — `src/hieronymus/scoring.py:32-45` (`apply_score_delta`), used by
  `FeedbackStore.record()` (`scoring.py:55-136`), which is invoked from the CLI
  (`src/hieronymus/cli.py:1025`):

  ```python
  # scoring.py:41-44
  if updated_confidence == 0.0 and not (crystal_type == "rule" and status == "active"):
      updated_status = "archived"
  ```
  plus a second rule layered on top at call time (`scoring.py:131`):
  ```python
  if event_type == "deleted_by_user" and strength < _ARCHIVE_STRENGTH_THRESHOLD:
      status = "archived"
  ```

- **Duplicate path** — `AdminBridge._record_immediate_feedback`
  (`src/hieronymus/admin.py:2057-2096`), used by the web console's `reinforce_crystal`,
  `decay_crystal`, and `delete_crystal` actions (`admin.py:1041-1112`, wired through
  `service_http.py`'s `/api/admin/actions/*` routes). It has its own copy of the delta table
  (`admin.py:191-195`, `_ADMIN_IMMEDIATE_EVENT_DELTAS`, currently identical in value to
  `scoring.py`'s `IMMEDIATE_EVENT_DELTAS`) and its own copy of `_ARCHIVE_STRENGTH_THRESHOLD`
  (`admin.py:261`), but only applies the second rule:

  ```python
  # admin.py:2092-2096
  strength = _clamp_score(float(crystal["strength"]) + strength_delta)
  confidence = _clamp_score(float(crystal["confidence"]) + confidence_delta)
  status = crystal["status"]
  if event_type == "deleted_by_user" and strength < _ARCHIVE_STRENGTH_THRESHOLD:
      status = "archived"
  ```

  **The confidence-hits-zero auto-archive rule from `apply_score_delta` is missing entirely
  from the admin path.** There is no other sweep that catches this later — `dreaming.py`'s
  only `'archived'` reference is an unrelated `where status not in ('archived', 'merged')`
  filter (`dreaming.py:2816`).

**Concrete failure scenario:** a translator repeatedly clicks "Decay" on a crystal in the web
console. Its `confidence` reaches exactly `0.0` via `_record_immediate_feedback`. Because the
web path never runs the `updated_confidence == 0.0` check, the crystal's `status` stays
`"active"` indefinitely — it keeps surfacing in recall/search even though it has zero
confidence. The identical sequence of `contradicted_by_user` events issued through the CLI
(`hiero feedback ...` → `FeedbackStore.record`) *would* archive it. Same domain event, same
event types, two different outcomes depending only on which entry point was used.

This is a direct consequence of copy-pasting business logic instead of importing
`apply_score_delta` from `scoring.py`.

---

## 2. Security

### 2.1 Auth token is embedded in the browser-opened URL, logged to history, and never actually used (HIGH)

`_launch_web_console` opens the browser with the session token as a query string parameter:

```python
# src/hieronymus/cli.py:200
url = f"http://{state.host}:{state.port}{route}?token={state.token}"
if not webbrowser.open(url):
```

But `service_http.py`'s request handler never reads a `token` query parameter anywhere —
`_is_authorized` only checks the `X-Hieronymus-Token` header or a `hieronymus_token` cookie
(`src/hieronymus/service_http.py:277-281`), and the frontend never reads
`window.location.search` or sets that cookie (confirmed: no match for `location.search`,
`URLSearchParams`, or `document.cookie` anywhere referencing `token` in
`frontend/src/web/**`; `frontend/src/web/lib/api.ts:14-19` sends every request with only
`credentials: "same-origin"`, no token header). Browser-originated requests are actually
authorized purely via Origin/Referer checks (`_is_browser_authorized`,
`service_http.py:283-296`).

**Net effect:** the bearer token that grants full local API access (including `/shutdown`,
`/api/mcp/*`, `/status` which exposes `data_root`/`database_path`) is written into:
- the OS browser history,
- the argv of the `webbrowser.open()`-launched process (visible to other local users via `ps`
  on multi-user systems),
- potential `Referer` headers if the console page ever loads or links to third-party
  resources,

for **zero functional benefit** — it's dead weight left over from before origin-based browser
auth was introduced (see `eace612 feat: authorize local admin by origin` in git history).
Recommendation: drop `?token=...` from the launch URL entirely.

### 2.2 The now-unused cookie-token code path is dead and misleading (LOW)

`_session_token()` (`service_http.py:414-419`) parses a `hieronymus_token` cookie, and
`_is_authorized` falls back to it (`service_http.py:278-280`). Nothing in the codebase ever
sets this cookie (no `Set-Cookie` header anywhere in `src/hieronymus/**`, verified by
repo-wide grep). This is vestigial from an earlier auth design; combined with 2.1 it suggests
the auth mechanism was migrated (token header → origin check) without cleaning up the old
surface.

### 2.3 The session-token file is written with default (non-restrictive) filesystem permissions, unlike the API-key config (MEDIUM-HIGH)

`write_server_state` persists `server.json`, which contains the plaintext bearer token used
for full local API access:

```python
# src/hieronymus/service_state.py:94-103
def write_server_state(config: HieronymusConfig, state: ServerState) -> None:
    paths = runtime_paths(config)
    paths.config_root.mkdir(parents=True, exist_ok=True)
    tmp = paths.server_json.with_name(f"{paths.server_json.name}.tmp-{os.getpid()}")
    tmp.write_text(
        json.dumps(state.to_json_dict(), ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    tmp.replace(paths.server_json)
```

`Path.write_text()` creates the file via the default `open()` mode (`0o666` masked by umask —
typically `0644`, world-readable on most Linux defaults). There is no permission-hardening
call anywhere in the codebase (verified by repo-wide grep for `chmod`).

Compare this to `provider.conf`, which stores the actual LLM API keys and correctly ends up
mode `0600` because it goes through `atomic_write_text`
(`src/hieronymus/agent_plugins/base.py:345-360`), which uses
`tempfile.NamedTemporaryFile` — Python's tempfile module always creates files at `0600`
regardless of umask, and `Path.replace()`/`os.rename()` preserves that mode on the final file.
`save_provider_catalog` uses this helper (`src/hieronymus/provider_config.py:129-132`).

So the project *has* a secure-write helper and uses it correctly for API keys, but the daemon's
own auth-token file — arguably just as sensitive, since it grants full local API access
including shutdown and MCP operations — bypasses it and falls back to umask defaults. It also
uses a different, ad-hoc implementation instead of the shared atomic-write helper used by every
other config writer in the repo, purely by having been written before/separately from it.

### 2.4 Hand-rolled WebSocket server with no frame parsing (MEDIUM)

`_handle_admin_websocket` (`service_http.py:298-346`) does the RFC 6455 handshake correctly,
but never parses incoming frames — it just discards raw bytes to detect close/liveness:

```python
# service_http.py:330-344
self.connection.settimeout(1)
while self.connection.recv(2):
    pass
except TimeoutError:
    while True:
        try:
            if not self.connection.recv(2):
                break
        except TimeoutError:
            continue
        except OSError:
            break
```

There's no validation of the client-to-server masking bit, no handling of Close/Ping/Pong
control frames, and no server-initiated close handshake on shutdown. It happens to work for a
push-only admin event channel, but it's a fragile, non-compliant reimplementation of a solved
protocol rather than using an existing WS library — and every open browser tab pins a
dedicated OS thread for the life of the connection (`ThreadingHTTPServer` gives one thread per
connection), so it doesn't scale past a handful of concurrent admin tabs. Low risk for a
single-user local tool, but worth flagging as a maintainability/robustness gap.

---

## 3. Build & packaging: duplicated, drifting frontend build recipes

The frontend is built through **four independent recipes** that can disagree on the Bun
version and, during a release, cause the frontend to be built twice:

| Recipe | Location | Bun version pin |
|---|---|---|
| Hatch build hook (used by `uv build` / `uv tool install`) | `hatch_build.py:11-20` | `mise exec bun@1.3.14` (pinned) |
| `python-semantic-release` release build | `pyproject.toml:66` — `build_command = "bun install --cwd frontend --frozen-lockfile && bun run --cwd frontend build && uv build"` | none — whatever `bun` is on `$PATH` |
| Shell installer | `install.sh:150-157` (`build_frontend`) | none — whatever `bun` is on `$PATH` (only checked to be `>=1.3` earlier, in `ensure_bun`) |
| CI | `.github/workflows/pr.yml:28-34` | `oven-sh/setup-bun@<sha>` pinned to `1.3.14` |

Two concrete consequences:

1. **Redundant double build during release/packaging.** `pyproject.toml`'s
   `build_command` runs `bun run --cwd frontend build` *and then* `uv build` as its last step
   — but `uv build` itself triggers `hatch_build.py`'s `CustomBuildHook`, which builds the
   frontend *again* (with a different, mise-pinned Bun). Similarly, `install.sh` calls
   `build_frontend()` at line 232 and then `uv tool install --force --reinstall "$APP_DIR"` at
   line 233, which again invokes the hatch build hook — a third build of the same frontend
   within one install run.
2. **No single source of truth for the Bun version**, despite the README (`README.md:57`)
   telling contributors to "Use Bun >=1.3". If `hatch_build.py`'s pinned `1.3.14` and whatever
   `bun` happens to be on a release runner's `$PATH` ever diverge in behavior, the artifact
   published by semantic-release's `build_command` and the one embedded by the hatch hook could
   differ.

### 3.1 Stale "OpenTUI" reference in the installer (LOW — evidence of incomplete cleanup)

```sh
# install.sh:150-157
build_frontend() {
    echo "Building OpenTUI frontend..."
    (
        cd "$APP_DIR/frontend"
        bun install --frozen-lockfile
        bun run build
    )
}
```

"OpenTUI" was the UI toolkit *before* it was replaced by the local web console
(`272294a feat: replace opentui with local web console`), which was itself later migrated to
Tailwind/Svelte (`28d4dfc refactor: complete Tailwind web console migration`). This log line
is two migrations out of date and will confuse anyone using the documented `curl | sh`
installer. It's a small thing, but it's a reliable marker that `install.sh` wasn't touched
during either migration, which is consistent with finding 3 above (it still runs its own
unpinned, uncoordinated build).

---

### 1.2 Orphan daemon process on startup timeout — two processes compete for same DB (HIGH)

`service_manager.py:118-120`: when the daemon starts, writes a state file, but never becomes
healthy within the timeout, the state file is removed — but the daemon process itself is **not
killed**:

```python
if last_state is not None:
    remove_server_state(self.config, expected_state=last_state)
raise RuntimeError("hieronymus service daemon did not become healthy")
```

The orphaned daemon continues running, may eventually become healthy, and will re-write its
state file. A subsequent `start()` call spawns a *second* daemon process; both compete for the
same database port and files, with the second instance failing silently (port already bound,
stderr suppressed to `/dev/null` at `service_manager.py:101-102`).

Related: `restart()` at `service_manager.py:166-169` calls `stop()` then `start()` without
verifying the stop succeeded — if the daemon was unreachable but the process still lives, you
get the same doubling problem.

### 1.3 Proposal approval crashes mid-transaction, leaving DB inconsistent (HIGH)

`admin.py:2280-2281`: `_approve_advisory_concept_proposal` parses `approved_variants_json` and
`forbidden_variants_json` from the proposal row via `json.loads()`. If either field contains
invalid JSON, a `json.JSONDecodeError` propagates uncaught **after** the concept may already
have been created by `_ensure_concept_for_proposal` at line 2271. The transaction is not rolled
back — the concept exists in the database but the proposal is not marked approved, creating an
orphaned concept with no audit trail.

### 1.4 TOCTOU race in concept matching during proposal approval (HIGH)

`admin.py:2376-2409`: `_matching_concept_id_for_proposal` SELECTs candidate concepts, scores
them, and the caller (`_ensure_concept_for_proposal`, line 2307) does `if concept_id is None:
INSERT`. Between the SELECT and INSERT, a concurrent proposal approval (e.g., from the CLI and
web console simultaneously) could create the same concept, resulting in a duplicate. There is
no `INSERT ... ON CONFLICT` or unique constraint to prevent this. Works safely for a
single-user local tool in practice, but the race is real.

### 1.5 Missing FTS delete triggers for three tables (HIGH)

The migration `global.sql` defines FTS5 external-content tables for `concepts_fts`,
`concept_facet_fts`, and `rag_chunks_fts` WITH corresponding `AFTER DELETE` / `AFTER UPDATE`
triggers to keep the FTS index in sync. Three other FTS tables lack these triggers entirely:

| FTS table | Content table | Has delete triggers? |
|---|---|---|
| `short_term_memories_fts` | `short_term_memories` | **No** — `global.sql:84-88` |
| `crystals_fts` | `crystals` | **No** — `global.sql:123-128` |
| `strict_terms_fts` | `strict_terms` | **No** — `global.sql:228-234` |

When rows are deleted from the content tables (e.g., `crystals` archived, task sessions
cascading to `short_term_memories`), the corresponding FTS entries become orphaned. The
`content=` linkage means SQLite won't return data for deleted rows, but the FTS index grows
unboundedly and eventually degrades search performance. Additionally, `workspace.py:419-422`
manually inserts into `short_term_memories_fts` but never deletes from it.

### 1.6 `_now()` duplicated 12 times across the codebase (MEDIUM)

Every source module defines its own `_now()` function instead of importing from a shared
location:

| File | Line | Implementation |
|---|---|---|
| `workspace.py` | 22 | `datetime.now(UTC).isoformat()` |
| `scoring.py` | 24 | `datetime.now(UTC).isoformat()` |
| `recall.py` | 87 | `datetime.now(UTC).isoformat()` |
| `rag_store.py` | 563 | `datetime.now(UTC).isoformat().replace("+00:00", "Z")` |
| `dream_audit.py` | 129 | `datetime.now(UTC).isoformat()` |
| `admin.py` | 2525 | `datetime.now(UTC).isoformat()` (instance method) |
| `crystals.py` | 27 | `datetime.now(UTC).isoformat()` |
| `concepts.py` | 37 | `datetime.now(UTC).isoformat()` |
| `dreaming.py` | 54 | `datetime.now(UTC).isoformat()` |
| `termbase.py` | 27 | `datetime.now(UTC).isoformat()` |
| `memory_migration.py` | 87 | `conn.execute("select datetime('now')")` (SQLite) |
| `registry.py` | 24 | `datetime.now(UTC).isoformat()` |

More critically, `rag_store.py:563` uses a **different format** (Z-suffix via
`.replace("+00:00", "Z")`) than the other 10 modules (`+00:00` suffix). Both write to the same
database — comparing or sorting timestamps across sources will produce different string
representations for the same instant.

### 1.7 `_clamp_score` duplicated 4 times (MEDIUM)

Identical implementation of `min(max(value, 0.0), 1.0)` exists in:
`scoring.py:28`, `admin.py:2568`, `dreaming.py:135`, and `crystals.py:31`.

### 1.8 N+1 queries in admin views (MEDIUM)

Two admin list views execute one additional query per row:

- `_list_strict_terms()` at `admin.py:1602-1611`: after loading 200 term rows, issues a
  separate `select tag from strict_term_tags where term_id = ?` for each row — up to 200
  additional round-trips.

- `_matching_concept_id_for_proposal()` at `admin.py:2394-2403`: calls
  `_concept_semantic_tags()` per candidate row, which issues its own SELECT — N additional
  queries. Should be a single JOIN.

### 1.9 `admin.py` builds an overly broad `except Exception` (MEDIUM)

`admin.py:2002`: the `except Exception` handler catches `KeyboardInterrupt`, `SystemExit`,
`MemoryError`, etc. when the intent is only to handle malformed audit payloads. Should be
narrowed to `except (TypeError, ValueError)`.

---

## 4. Frontend test suite doesn't test behavior

`frontend/src/web/app.test.ts` is the primary frontend test file. Every test in it asserts on
raw source-file text, not on rendered output or component behavior:

```ts
// app.test.ts:3-9
test("the app shell uses the semantic Tailwind surface utilities", async () => {
  const app = await Bun.file(new URL("./App.svelte", import.meta.url)).text();
  expect(app).toContain("min-h-dvh");
  expect(app).toContain("bg-root");
  expect(app).toContain("border-default");
});
...
// app.test.ts:71-85
test("editor controls preserve native dialog and accessible toggle markup", async () => {
  const providerEditor = await Bun.file(new URL("./components/ProviderEditor.svelte", ...)).text();
  const dreamingEditor = await Bun.file(new URL("./components/DreamingEditor.svelte", ...)).text();
  expect(providerEditor).toContain("<dialog");
  expect(providerEditor).toContain("aria-labelledby");
  expect(dreamingEditor).toContain("peer");
  expect(dreamingEditor).toContain("min-h-11");
  expect(dreamingEditor).toContain("peer-checked:[&>span]:translate-x-[18px]");
  expect(dreamingEditor).toContain("peer-checked:[&>span]:bg-accent");
});
```

These are effectively grep assertions dressed up as tests: they pass as long as the literal
substring exists anywhere in the file, regardless of whether it's reachable, correctly wired,
or does what the test name claims. Renaming a class, moving markup into a different branch of
an `{#if}`, or breaking the actual toggle interaction would not fail these tests as long as the
string is still present somewhere in the file. Conversely a real regression in rendering logic
(e.g., a broken `$state` binding, a component that throws at mount) would not be caught either,
since nothing here ever mounts a component. Aside from `theme.svelte.test.ts` (which does test
real logic), there are **no DOM/interaction tests** for six Svelte components handling
settings, provider CRUD, and admin actions — despite `@sveltejs/vite-plugin-svelte` and Svelte
5 fully supporting component tests.

---

## 5. Service & daemon reliability

### 5.1 Daemon stdout/stderr sent to `/dev/null`, destroying crash diagnostics (MEDIUM)

`service_manager.py:101-102`:

```python
subprocess.Popen(
    [...],
    stdout=subprocess.DEVNULL,
    stderr=subprocess.DEVNULL,
)
```

If the daemon fails to start (import error, corrupted config, DB locked), all diagnostic output
is discarded. The only failure signal is the polling timeout with no access to the actual error.
On a local workstation this means opaque "daemon did not become healthy" messages with no way
to debug.

### 5.2 `service_manager.restart()` doesn't verify `stop()` succeeded (MEDIUM)

`service_manager.py:166-169`:

```python
def restart(self) -> dict[str, Any]:
    stopped = self.stop()
    self.start()
    return {"stopped": stopped, "status": self.status()}
```

If `stop()` returns `{"stopped": False}` because the daemon was unreachable (but the process
still lives), `start()` tries to launch another daemon on the same port. The new daemon fails
to bind, and the old daemon continues running — the user sees "stopped: False" but then gets a
status report from the *old* daemon, hiding the failure.

### 5.3 Daemon `main()` has no signal handler for `SIGTERM` (MEDIUM)

`service_daemon.py:83-107`: The `main()` function relies on `KeyboardInterrupt` from Ctrl+C to
trigger the `finally` cleanup block (stopping the autostart scheduler, shutting down the HTTP
server). A `SIGTERM` (as sent by `kill` or `systemctl stop`) kills the process immediately
without running cleanup. The daemon should register signal handlers for graceful shutdown.

### 5.4 Scheduler thread `join()` has no timeout — blocks shutdown indefinitely (LOW)

`service_daemon.py:49-53`: `DreamAutostartScheduler.stop()` calls `self._thread.join()` with no
timeout. If the scheduler thread is blocked inside `run_due()` (e.g., waiting on a slow DB
query or LLM call), the calling shutdown sequence hangs until that operation completes — which
could be minutes.

### 5.5 Dream progress monitor polls DB at 200 ms without backoff (LOW)

`service_http.py:356-382`: The manual dreaming progress monitor opens a new DB connection every
200 ms for the entire duration of a dream run. For a 5-minute dream, that's ~1500
connection open/close cycles and queries. The interval is hardcoded with no adaptive backoff.

---

## 6. Configuration & naming

### 6.1 `config_root` is a dead alias for `data_root` (LOW)

`config.py:17-18`:

```python
@property
def config_root(self) -> Path:
    return self.data_root
```

`config_root` and `data_root` always return the same value. This suggests a past or planned
separation where config and data would live in different directories, but they are permanently
collapsed. Every consumer using `config_root` could use `data_root` directly.

### 6.2 XDG_CONFIG_HOME not respected (LOW)

`config.py:55`: The default config path is hardcoded to `~/.config/hieronymus`, ignoring the
`$XDG_CONFIG_HOME` environment variable per the XDG Base Directory spec. If a user has set
`XDG_CONFIG_HOME` to a different location, their config is silently put in the wrong place.

### 6.3 `llm_cache_path` uses misleading `.tmp` extension (LOW)

`config.py:37-38`: `self.config_root / "llmcache.tmp"`. The `.tmp` extension conventionally
indicates a temporary file that may be deleted, but this file is a persistent model cache with
a 24-hour TTL.

### 6.4 `tui_bridge/` package name is misleading (LOW)

The `src/hieronymus/tui_bridge/` directory was originally a JSON-RPC bridge for a terminal UI
(React/Ink, then OpenTUI). Both frontends have been replaced by the Svelte web console, but the
package name remains. The actual usage is as the internal API layer between the HTTP server and
the admin/config stores (`service_http.py` imports `AdminBridge` and `ConfigBridge` directly),
making "tui_bridge" a confusing name for anyone reading the codebase.

### 6.5 `tui_bridge/server.py:run_stdio` is dead code (LOW)

`server.py:39-161`: The `run_stdio()` function reads JSON-RPC requests from stdin and writes
responses to stdout. It is never called anywhere in the codebase — not in production, not in
tests. In fact, `tests/test_tui_bridge_protocol.py:162` explicitly asserts the package
exports nothing (`tui_bridge.__all__ == []`), confirming this stdio loop is a leftover from the
terminal-UI era. The `dispatch()` function in the same file IS used by tests, but `run_stdio`
itself is dead.

---

## 7. Database & data integrity

### 7.1 Timestamp format inconsistency between modules (MEDIUM)

Two modules serialize timestamps differently for the same database:
- `rag_store.py:564`: `datetime.now(UTC).isoformat().replace("+00:00", "Z")` → `"2026-01-01T12:00:00Z"`
- `workspace.py:22-23`: `datetime.now(UTC).isoformat()` → `"2026-01-01T12:00:00+00:00"`

Both write to the same database tables. Any cross-module timestamp comparison or sorting will
produce inconsistent results since the strings differ even for the same instant.

### 7.2 Frozen dataclasses mutated via `object.__setattr__` (LOW)

Two `@dataclass(frozen=True)` classes bypass the frozen contract in `__post_init__`:
- `TranslationContext` (`memory_models.py:65-66`): normalizes and overwrites `language_tags`,
  `story_scopes`, `semantic_tags` via `object.__setattr__`
- `AgentAvailability` (`agent_plugins/base.py:42-43`): re-tuples `detect_paths` and
  `config_paths`

This works but is fragile — if a field is renamed, `__setattr__` won't catch the typo the way
normal attribute assignment would.

### 7.3 `_json_loads_object` defined twice with different names (LOW)

`rag_store.py:421-425` defines `_json_loads_object` and `workspace.py:31-36` defines
`_json_object` — identical logic (parse JSON, return `{}` on non-dict, but neither handles
`json.JSONDecodeError`). Should be centralized.

### 7.4 `clean_text_tuple` / `normalize_string_tuple` near-duplicate (LOW)

`rag_store.py:567-576` and `memory_models.py:10-25` both strip, deduplicate, and preserve order
of string tuples. `normalize_string_tuple` adds a `lowercase` flag. One should delegate to the
other.

---

## 8. Frontend / web console

### 8.1 `bun-types` in tsconfig for a browser-targeted build (MEDIUM)

`frontend/tsconfig.json:11`: includes `"bun-types"` alongside `"vite/client"`. The compiled
frontend runs in a browser, not in Bun. Including `bun-types` could shadow browser-native types
(`Request`, `Response`, `fetch`) and is unnecessary. The test file `app.test.ts` uses `Bun.file`
which is why it's present, but the production build doesn't need it.

### 8.2 `remove_short_term_memory` action defined but unreachable (MEDIUM)

`MemoryViews.svelte:37`: the action string `"remove_short_term_memory"` is in the
`destructiveActions` set and `actionLabels` record, but appears in **no** `actionByView` entry.
It can never be triggered from the UI. Either missing from a view definition or leftover dead
code.

### 8.3 Dead CSS classes in `app.css` (LOW)

`frontend/src/web/app.css:180-215`: `.table-shell`, `.toggle-track`, and `.toggle-thumb` are
defined in the `@layer components` block but never referenced in any Svelte template. Tables
use inline Tailwind utilities, and toggle switches are built directly with Tailwind classes.

### 8.4 No fetch timeout in API client (LOW)

`api.ts:14-19`: The `request()` function wraps `fetch()` with no `AbortController` or timeout
mechanism. A hung API call blocks the UI indefinitely (mitigated only by the `busy` flag
preventing duplicate requests).

### 8.5 Error display styling uses two different patterns (LOW)

Five components use two inconsistent CSS patterns for error alert boxes:
- `App.svelte:243`, `MemoryViews.svelte:135`, `AdminDashboard.svelte:73` use `bg-raised`
- `DreamingEditor.svelte:84`, `IngestEditor.svelte:23`, `ReleaseEditor.svelte:14`,
  `ProviderEditor.svelte:43` use `bg-[var(--hiero-danger-bg)]`

The two classes render the same color in practice but the inconsistency makes theme changes
error-prone.

---

## 9. Design / maintainability observations

### 9.1 `DreamService` is a large god-class

`src/hieronymus/dreaming.py` is 3,718 lines — the largest file in the codebase by a wide
margin (next is `admin.py` at 2,613) — and `DreamService` alone spans roughly 150 methods
(`dreaming.py:347` onward, methods listed from `_run_cycle_unlocked` through
`_passive_event_cap_skips`). It mixes cycle orchestration, phase-run bookkeeping, audit
logging, passive-event application, and score maintenance in one class. Not a bug, but a
standing maintainability risk: any change to one concern (e.g., audit logging) requires
reasoning about the whole class, and the size makes it easy for logic like §1.1 to be
reimplemented elsewhere rather than found and reused.

### 9.2 Ambiguous static-asset resolution order

```python
# service_http.py:405-411
def _web_asset_roots() -> list[Path]:
    package_root = Path(__file__).resolve().parent / "frontend" / "dist"
    roots = [package_root]
    for ancestor in Path(__file__).resolve().parents[:5]:
        roots.append(ancestor / "frontend" / "dist")
        roots.append(ancestor / "frontend")
    return roots
```

This tries 11 candidate directories (packaged install path, then 5 ancestor directories × 2
variants each) before giving up, to support both installed and source-checkout layouts. It
works, but it means the server can silently serve assets from whichever of 11 locations
happens to exist first, which makes "why is the console showing stale UI" hard to debug in a
dev environment with multiple checkouts/venvs nearby (e.g. this repo has a
`.worktrees/rag-pipelines/frontend` directory that would be within the parent-walk of
anything inside it). Worth collapsing to one clearly-documented lookup rule.

---

## Summary table

| # | Finding | File(s) | Severity |
|---|---|---|---|
| 1.1 | Web-console crystal decay skips the confidence==0 auto-archive rule that the CLI path applies | `admin.py:2057-2096`, `scoring.py:32-45` | High (logic bug) |
| 1.2 | Orphan daemon process on startup timeout — two daemons can compete for same DB/port | `service_manager.py:118-120,166-169` | High (reliability) |
| 1.3 | Proposal approval crashes mid-transaction on bad JSON, leaving DB inconsistent | `admin.py:2280-2281` | High (data integrity) |
| 1.4 | TOCTOU race in concept matching during proposal approval | `admin.py:2376-2409` | High (race condition) |
| 1.5 | Missing FTS delete triggers for `short_term_memories_fts`, `crystals_fts`, `strict_terms_fts` | `global.sql:84-88,123-128,228-234`, `workspace.py:419-422` | High (data integrity) |
| 2.1 | Auth token leaked into browser URL/history/process args, never consumed by anything | `cli.py:200` | High (security) |
| 2.3 | Session-token file (`server.json`) written world-readable; inconsistent with `provider.conf`'s secure write path | `service_state.py:94-103` vs `agent_plugins/base.py:345-360` | Medium-High (security) |
| 1.6 | `_now()` duplicated 12× with two different timestamp formats | 12 files (see §1.6) | Medium (maintenance) |
| 1.7 | `_clamp_score` duplicated 4× | `scoring.py:28`, `admin.py:2568`, `dreaming.py:135`, `crystals.py:31` | Medium (maintenance) |
| 1.8 | N+1 queries in admin list views (strict terms, concept matching) | `admin.py:1602-1611,2394-2403` | Medium (performance) |
| 1.9 | Overly broad `except Exception` in audit payload parsing | `admin.py:2002` | Medium (robustness) |
| 2.4 | Hand-rolled WebSocket server, no frame parsing/close handshake, one thread per connection | `service_http.py:298-346` | Medium (robustness) |
| 3 | Frontend build defined 4 ways with inconsistent Bun pinning; redundant double-build on release/install | `hatch_build.py`, `pyproject.toml:66`, `install.sh:150-233` | Medium (build) |
| 4 | Frontend tests assert on source text, not rendered/behavioral output — 6 components with no real component tests | `frontend/src/web/app.test.ts` | Medium (test quality) |
| 5.1 | Daemon stdout/stderr to `/dev/null` destroys crash diagnostics | `service_manager.py:101-102` | Medium (operations) |
| 5.2 | `restart()` doesn't verify `stop()` succeeded before starting | `service_manager.py:166-169` | Medium (reliability) |
| 5.3 | No `SIGTERM` handler — `kill` skips graceful shutdown | `service_daemon.py:83-107` | Medium (reliability) |
| 7.1 | Timestamp format inconsistency between `rag_store` (Z-suffix) and `workspace` (+00:00) | `rag_store.py:564`, `workspace.py:22-23` | Medium (data) |
| 8.1 | `bun-types` in tsconfig for browser-targeted build | `frontend/tsconfig.json:11` | Medium (build) |
| 8.2 | `remove_short_term_memory` action defined but unreachable in UI | `MemoryViews.svelte:37` | Medium (logic) |
| 2.2 | Dead cookie-token auth code path, nothing ever sets the cookie | `service_http.py:278-280,414-419` | Low |
| 3.1 | Stale "OpenTUI" log message survives two UI migrations | `install.sh:151` | Low |
| 9.1 | `DreamService` god-class, ~150 methods / 3,718 lines | `dreaming.py` | Low (architecture) |
| 9.2 | 11-directory ambiguous static-asset search order | `service_http.py:405-411` | Low |
| 5.4 | Scheduler `thread.join()` with no timeout blocks shutdown | `service_daemon.py:49-53` | Low |
| 5.5 | Dream progress DB poll at 200 ms, no backoff | `service_http.py:356-382` | Low |
| 6.1 | `config_root` is a dead alias for `data_root` | `config.py:17-18` | Low |
| 6.2 | `$XDG_CONFIG_HOME` not respected | `config.py:55` | Low |
| 6.3 | `llm_cache_path` uses misleading `.tmp` extension | `config.py:37-38` | Low |
| 6.4 | `tui_bridge/` package name misleading (was for terminal UI, now serves web API) | `tui_bridge/` directory | Low |
| 6.5 | `tui_bridge/server.py:run_stdio` is dead code — never called | `tui_bridge/server.py:39-161` | Low |
| 7.2 | `@dataclass(frozen=True)` mutated via `object.__setattr__` in `__post_init__` | `memory_models.py:65-66`, `agent_plugins/base.py:42-43` | Low |
| 7.3 | `_json_loads_object` defined twice with different names | `rag_store.py:421-425`, `workspace.py:31-36` | Low |
| 7.4 | `clean_text_tuple` / `normalize_string_tuple` near-duplicate | `rag_store.py:567-576`, `memory_models.py:10-25` | Low |
| 8.3 | Dead CSS classes (`.table-shell`, `.toggle-track`, `.toggle-thumb`) | `frontend/src/web/app.css:180-215` | Low |
| 8.4 | No fetch timeout in frontend API client | `frontend/src/web/lib/api.ts:14-19` | Low |
| 8.5 | Two inconsistent error display CSS patterns across components | 7 Svelte components (see §8.5) | Low |
| — | Pytest suite is clean: no bare excepts, no sleep(), no assert True, no empty tests | `tests/` (89 files) | Clean (positive) |

Areas checked and found clean: no bare `except:` / silent `except Exception: pass`, no
`shell=True` / `eval` / `exec` / unsafe deserialization usage, no string-built SQL fed by user
input (the f-string SQL in `admin.py`, `db.py`, `memory_migration.py` interpolates only
hardcoded table names, never request data), no `TODO`/`FIXME` markers left in source,
cross-process dream-cycle locking (`dream_locks.py`) is correctly implemented with `fcntl` +
a per-path `threading.Lock`, and CI pins third-party GitHub Actions to full commit SHAs.
