# Rust Dreaming Design

**Status:** Proposed for review on 2026-08-31.

## Goal

Port dreaming as bounded, auditable phases with clear mutation ownership rather
than reproduce the Python god-class in Rust.

## Execution Ownership

The daemon scheduler and authenticated manual requests enqueue dream work. One
worker owns a dream cycle for a data root. An OS-level lock protects exclusive
debug or maintenance invocations from overlapping daemon work. Lock acquisition
is nonblocking on Tokio worker threads; waiting uses bounded asynchronous retry
or a blocking thread.

Every run has a durable run id, trigger type, owner instance, input snapshot,
status, and phase records. Recovery marks abandoned leases and either resumes a
safe phase or fails the run explicitly. It never assumes that process death
means an uncommitted database transaction succeeded.

## Phase Boundaries

The orchestration invokes focused components:

1. selection builds the bounded affected-memory set;
2. provider phases request crystallization or configured judgment;
3. parsing validates and normalizes bounded provider output;
4. deterministic relation discovery proposes graph changes;
5. reconsolidation evaluates advisory working copies;
6. reinforcement consumes unprocessed feedback and memory events;
7. maintenance validates bounded decay, supersession, combination, and links;
8. persistence applies one validated mutation batch transactionally;
9. audit finalizes input, output, warning, decision, and mutation summaries.

Provider access is injected behind a trait, but phase orchestration is not
required to erase heterogeneous associated types into one trait object. Typed
phase calls or an explicit enum pipeline are preferred over an object-unsafe
generic phase abstraction.

## Provider Policy

Provider profiles come from `provider.conf`; workflow assignments come from
`dream.conf` under ADR 0007. Enabled invalid workflows fail closed. A
deterministic provider is available only for workflows explicitly declared
deterministic, tests, and diagnostics; it never silently replaces a failed
configured LLM workflow.

Network requests have configured timeouts, bounded retry for retryable failures,
request ids, and redacted audit data. Provider output is size-limited before
parse. Code fences may be removed, but accepted data must deserialize into the
phase schema and pass domain validation.

Malformed-output penalties apply only to identifiable accepted candidates or
the phase audit. A payload that cannot identify a crystal cannot mutate an
unrelated crystal's penalty.

## Bounded Mutation

Selection and every phase use explicit caps from validated configuration.
Decay queries are indexed and always include an absolute limit. Maintenance may
mutate only ids in the recorded affected set or ids created by the same run.
Active deterministic rule authority is excluded from passive decay and
automatic deactivation.

The scheduler implements interval, urgent backlog, and bounded drain behavior.
Shutdown stops accepting new runs, lets the current transaction finish or roll
back, records the interrupted state, and releases the lock.

## Audit

Audit records include provider profile id, provider type, model, redacted
endpoint, prompt hash, selected input ids, bounded raw-response metadata,
parse warnings, accepted/rejected item reasons, affected ids, proposed and
applied mutations, recovery actions, and phase timing. Secrets and unrestricted
source text are never logged by default.

## Acceptance Criteria

- Two processes cannot execute overlapping cycles for one data root.
- Each phase can be tested with deterministic inputs and fake providers.
- Invalid configured providers fail closed and are audited.
- Failure injection cannot leave unaudited partial domain mutations.
- Caps are enforced before provider calls and persistence.
- Feedback evidence is consumed at most once.
- Active deterministic rules are not passively decayed or archived.
