# Independent review of the Rust port branch

Reviewer: Claude (Sonnet 5), 2026-09-05
Branch: `feat/rust-rewrite-proposals` @ `fcab0b5`
Baseline: merge-base with `main` = `8540a5d`; port work starts at `88160be`
Inputs: the handoff (`docs/handoff-rust-port.md`), the SDD ledger
(`.superpowers/sdd/handoff-rust-port/progress.md`), ADRs 0005–0015, the
distribution/roadmap docs, and the branch source.

This document records where I agree with the delivered work, what I
independently re-verified, and the places where I would either change
something or ask you to rule before the port is called done. It is additive
to the SDD ledger — it does not restate the per-task minor triage there.

---

## 1. Verdict

The mechanical state the closing report claims is **accurate**. I re-ran the
gates and checked the fixtures myself:

| Claim | Result |
| --- | --- |
| `env cargo test --workspace` | **579 passed, 0 failed, 1 ignored** (the ignored one is the `HIERONYMUS_SEMANTIC_LIVE=1` live test, `semantic_port.rs:1108`) |
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean (0) |
| Frozen fixtures untouched | confirmed — `git log 88160be..HEAD -- compatibility/fixtures compatibility/snapshots` is empty |
| Semantic pins vs ADR 0013 qualification | confirmed — `lancedb =0.37.1`, `ort =2.0.0-rc.13`, `arrow* =58.3.0` in `Cargo.lock` |
| Port is additive; Python untouched | confirmed — 47 deletions on the whole branch, all in docs/CI/workflow files |
| `/api/mcp/{operation}` private bridge removed (ADR 0015) | confirmed — `rest/mod.rs:76` returns `not_found()` and it is tested |
| `POST /mcp`, `GET /health` per ADR 0012/0015 | confirmed — `daemon/server.rs:36-37` |
| Final-fix commit `fcab0b5` content | confirmed — audit prompt SHA-256 + redacted endpoint, non-active-source reconsolidation guard, `install.sh` https enforcement, all with new tests |

The three "decisions that need you" in the closing report are all real and
all correctly stated. My findings below **add** to them; they do not
contradict them.

The one framing I would push back on: the branch is described as "review
clean, ready for finishing." That is true of the *code that was in scope*.
It is not true of the *product*. Over MCP — the primary agent surface — the
release currently does nothing but report adapter status (§2). I would not
describe the port as ready to finish until that is either wired or the
roadmap explicitly reclassifies the first Rust release as "daemon + CLI +
console, MCP domain tools to follow."

---

## 2. ADR conformance findings

### 2.1 — MCP domain tools unwired — blocks ADR 0008's own cutover gate  (Critical, disclosed)

`daemon/registry.rs:107-112`: `tools/call` dispatches exactly one tool.

```rust
match name {
    "hieronymus_status" => Ok(status_result()),
    _ => Err(CallError::NotPorted(name.to_string())),
}
```

`tools/list` advertises all **39** tools from the frozen registry
(`compatibility/snapshots/mcp.json`); 38 of them return `NotPorted`. The
semantic recall lane is in the same position — `arm_recall_service`
(`semantic_arming.rs:60`) and `RecallService::with_semantic_lane`
(`recall.rs:214`) have **no non-test caller** anywhere in the tree.

I agree with the closing report that the handoff queue never contained this
wiring. But note the consequence precisely against **ADR 0008 §Cutover
Gates**, first bullet: *"every compatibility-manifest entry is implemented,
intentionally changed by an accepted ADR, or explicitly removed."* The 38
tools are none of those three — they are advertised-but-absent. By the
ADR's own gate, cutover cannot pass in this state, regardless of how green
the suite is.

`hieronymus_status` itself is faithfully implemented — its constant return
value (`registry.rs:117`) is byte-identical to the frozen fixture
(`compatibility/fixtures/mcp/protocol.json` → `target.tools_call`), which is
genuinely that minimal. So the issue is strictly the other 38 plus recall
arming, not the one that shipped.

**Recommendation.** Treat "wire MCP `tools/call` + arm the recall service in
the daemon" as a blocking task with its own review, not a roadmap footnote.
Until it lands, `docs/roadmap.md` and `docs/distribution.md` should lead
with "the first Rust release serves CLI, daemon lifecycle, REST admin, and
the web console; MCP domain tools are a required follow-up" rather than
carrying it as the fourth deferred-gap bullet. The REST admin surface
(`/api/admin/*`, `/api/providers`, `/api/settings/*`, `/recall/feedback`)
*is* wired, so the console is functional — the gap is specifically the
agent/MCP path.

### 2.2 — Recall response has no deterministic-contract section (ADR 0011)  (Important — the ruling inverted the authority order)

**ADR 0011 §Decision:** *"Every recall response also includes a separate
deterministic-contract section when applicable, so callers cannot mistake
ranking order for enforcement order. Validation evaluates that section
before advisory findings."*

`recall.rs:129-134`:

```rust
pub struct RecallResponse {
    pub recall_id: String,
    pub hits: Vec<RecallHit>,
    pub warnings: Vec<RecallWarning>,
}
```

There is no contract section. The contract *is* computed on every recall
(`recall.rs:368`, `Termbase::open(...).contract(query)`) but it is only used
internally to stamp `conflicts_with_rule_ids` onto individual hits
(`recall.rs:419`, `:451`). The caller never receives the contract itself.

The SDD ruling (ledger line 167, closing-report ruling #12) dismissed this
as a non-gap on the grounds that "Python returns a bare list" and "the
contract reaches callers via the separate `hieronymus_termbase_contract`
tool." That reasoning is upside-down relative to **ADR 0008 §Decision**'s
stated authority order:

> 2. other accepted ADRs … 5. current Python behavior *where no
> higher-level decision changes it*.

ADR 0011 is a level-2 decision that explicitly changes it. "Python returns a
bare list" is a level-5 fact that ADR 0011 supersedes. The ruling used the
lower authority to overrule the higher one. It also leans on the
`hieronymus_termbase_contract` tool as the delivery path — but that tool is
one of the 38 that are not wired (§2.1), so today the contract reaches *no*
caller by *any* path.

This is a small additive change: `RecallResponse` already holds the
computed `Vec<ContractTerm>`; exposing it as a field (and having the
validation path read it first) is a handful of lines plus a fixture.

**Recommendation.** Owner ruling. Either (a) add the contract section to
`RecallResponse` and the recall MCP/REST DTOs, or (b) amend ADR 0011 to say
the contract is delivered only through the termbase tools and recall hits
carry markers only. I lean (a) — the ADR's rationale ("callers cannot
mistake ranking order for enforcement order") is a safety property, and a
separate tool call the caller has to *remember to make* does not provide it.

### 2.3 — ADR 0007's multi-provider model is structurally unimplementable  (Important, disclosed — concur)

The closing report's decision #2 is correct and I want to reinforce how
baked-in it is. `validate_workflow_wiring` (`dreaming.rs:1967-2034`) walks
every enabled `dream.conf` workflow and **rejects** any whose resolved
`(provider, model)` is not identical to the single injected provider:

```rust
if injected_profile != provider_id || injected_model != model {
    return Err(DreamError::InvalidWorkflow(format!(
        "workflow {name} is assigned to provider {provider_id} model {model}; \
         the injected provider serves {injected_profile} model {injected_model}"
    )));
}
```

ADR 0007 §Decision gives, as its worked example, `knowledge_crystals →
deepseek-api` and `reinforcement → local-ollama` in the same `dream.conf`.
That config cannot open a `DreamService`. This is not a bug in the
implementer's work — `DreamService<P>` is generic over exactly one provider
by construction — but it means ADR 0007's core "different workflows use
different endpoints" decision is currently **not honored**, and the failure
mode is a hard refusal at daemon start, not a degraded path.

**Recommendation.** This needs a real decision, not a follow-up ticket:
either ADR 0007 is amended down to single-provider-per-installation (honest,
and matches what shipped), or `DreamService` needs a provider *resolver*
keyed by workflow before the dreaming path can be called cutover-complete.
Given the single-provider gate is load-bearing in the fail-closed design, I
would amend the ADR unless you have a concrete near-term need for
per-workflow endpoints.

### 2.4 — `hiero start` / `hiero stop` are not top-level; `/shutdown` has no client  (Minor — confirm intent)

**ADR 0009 §Decision:** *"`hiero start` installs or starts the per-user
service… `hiero stop` requests authenticated graceful shutdown through the
discovered daemon endpoint."*

Shipped surface (`main.rs:366`, `main.rs:882-945`): `hiero service
{install,uninstall,status,start,stop}`. Two deltas:

1. **Spelling.** The commands are `hiero service start` / `hiero service
   stop`, not `hiero start` / `hiero stop`. Minor, but the ADR names the
   short forms specifically and nothing I can find amends them.
2. **`hiero stop` semantics.** ADR 0009 defines `hiero stop` as an
   *authenticated graceful-shutdown RPC to the discovered endpoint*.
   `service::stop` (`service.rs:339`) runs `systemctl --user stop`. The
   authenticated RPC path exists on the server (`POST /shutdown`,
   `server.rs:38`, `handle_shutdown` at `:135`) but **no CLI drives it** —
   `grep` finds no client caller. For a managed install this is cosmetic
   (systemd sends SIGTERM, the signal handler shuts down cleanly). For an
   unmanaged `hiero daemon` there is no graceful-stop command at all; you
   SIGINT it. `POST /shutdown` is currently exercised only by tests.

**Recommendation.** Either add `hiero stop` as a thin top-level alias that
calls `POST /shutdown` against the discovery record (closes the ADR wording
*and* the unmanaged-daemon gap), or amend ADR 0009's runtime-topology
section to describe the `hiero service …` verb set and note that graceful
shutdown for unmanaged daemons is signal-only.

### 2.5 — CSRF ruling is correct, but the frozen fixtures now disagree with the code by design  (Minor — leave a marker)

The SDD ruling that the CSRF layer is waived (ledger line 57) is **right** —
ADR 0012's 2026-09-03 amendment says verbatim *"the separate CSRF token
layer is waived."* No issue with the decision.

The side effect: `compatibility/fixtures/http/route-cases.json` still encodes
`X-CSRF-Token` headers and `csrf_failed` 403 cases, and it is frozen on
disk and never asserted. So the repo now contains a frozen "contract" that
the implementation deliberately does not meet. Anyone who later runs a
"restore fixture parity" pass — a plausible instinct — will reintroduce a
layer the owner removed.

**Recommendation.** Add one line to whatever indexes the compatibility
manifest (or a `NOTE:` comment beside the fixture) pointing at the ADR 0012
amendment and the ledger ruling, so the divergence is self-documenting.
Cheap insurance.

---

## 3. Durability / quality observations

These are drawn from the ~70-item deferred-minor backlog in the SDD ledger.
Most of that backlog is genuinely safe-to-defer (cosmetic event-ordering
races on `dream_phase_progress`, double-delivery between WS subscribe and
replay that is `event_id`-dedupeable, etc.). A few are closer to
correctness and I would not want them to live only in a ledger that the
handoff says gets deleted after merge:

1. **Audit/phase-completion written on a separate connection after the
   domain commit** (`dreaming.rs`, Task 5 minor). A crash in the window
   leaves a `running` phase row with no audit entry. It mirrors a
   pre-existing provider-phase pattern, so it is consistent — but "consistent
   with an existing latent bug" is still a latent bug in the audit trail
   that ADR 0011 §"Rule lifecycle transitions are transactional and audited"
   leans on.

2. **Reconsolidation activations stamped `cycle_id` when the link-reinforcement
   budget is exhausted mid-batch** (`dreaming.rs run_link_reinforcement`,
   Task 5 minor). Skipped pairs are *permanently* dropped rather than
   deferred to the next cycle. For a memory-consolidation system that is a
   silent data-completeness gap, not just a performance nit.

3. **`global.sql` edited in place with no `ALTER` path** (Task 5 minor). Every
   schema change so far has been a destructive rewrite of the create script.
   That is fine *only* while there is no released Rust schema. The first
   post-cutover schema bump needs a real ordered-migration runner (ADR 0010
   §"ordered SQL and typed converters") and there is currently no scaffold
   for one on the Rust side — `hiero migrate` handles Python→Rust, not
   Rust→Rust.

4. **`semantic_index.rs:256` builds the ANN pre-filter by string
   interpolation** — `format!("series_slug = '{series_slug}'")`. It is
   guarded by `validate_slug(series_slug)?` immediately above, so it is not
   exploitable today, but a parameterized predicate (if `lancedb`'s query
   builder supports one) would be sturdier than relying on the validator
   staying strict forever.

**Recommendation.** Promote items 1–3 to tracked issues before the ledger is
deleted. They are the ones where "documented and deferred" and "forgotten"
look identical in six months.

---

## 4. Process note

The port was built and reviewed almost entirely by subagents with the owner
as the only human gate. On the evidence I can check, that produced
trustworthy *mechanical* output: the gates really pass, the fixtures really
are frozen, the pins really match qualification, the fail-closed paths
really are fail-closed, and the final-fix commit really does what its
message says with real tests behind it.

Where the process is weaker is the **rulings** — the ~13 owner-decision
points the controller resolved mid-run. Most are sound (CSRF waiver,
sequencing splits, http-only-then-TLS). But §2.2 shows one where the
controller reached for the lower authority (Python behavior) to overrule the
higher one (an accepted ADR), and rationalized it with a delivery path
(`hieronymus_termbase_contract`) that does not currently exist. That is the
failure mode to watch for in subagent-driven work: each local decision looks
defensible in isolation, and the authority order only gets violated when you
line them all up. A pass that re-reads every "ruled a non-gap" decision
specifically against ADR 0008 §Decision's numbered authority list would be
worth doing before cutover.

---

## 5. What I would do before calling the port done

Priority order:

1. **Wire MCP `tools/call` and arm the recall service in the daemon** (§2.1).
   This is the difference between "a daemon that speaks the MCP handshake"
   and "the product." Blocking per ADR 0008's own gate.
2. **Rule on the recall contract section** (§2.2) — add the field or amend
   ADR 0011. Do not ship the safety gap silently.
3. **Rule on ADR 0007** (§2.3) — amend to single-provider, or schedule the
   per-workflow resolver. The current state is "ADR says X, code refuses X."
4. **Promote durability items 1–3** (§3) to issues before the SDD ledger is
   deleted.
5. **Decide `hiero stop`** (§2.4) — alias to `/shutdown`, or amend ADR 0009.
6. **Leave the CSRF-fixture marker** (§2.5).

Items 1–3 are owner decisions with real trade-offs — they are exactly the
kind of thing that should be an ADR amendment rather than a ledger line,
because the next person to read ADR 0007 or 0011 will otherwise implement to
a contract the running system does not honor.
