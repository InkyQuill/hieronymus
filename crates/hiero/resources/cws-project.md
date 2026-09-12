# CWS project context for Hieronymus skills

Use this resource from `hieronymus-bootstrap` when the current file or working
directory may belong to a Creative Writing Skills (CWS) project. CWS does not
need to be installed to inspect or read a supported project. This is a reading
and decision workflow; project detection and a `.hieronymus.json` binding do not
grant trust or permission to write either store.

## Inspect before choosing memory

Run the installed read-only projection from the actual working path:

```sh
hiero project-context --cwd /path/to/project --args '{"direction_id":null}' --json
```

The command returns `version`, `status`, `root`, `schema_version`,
`instructions_path`, `binding`, `direction_id`, `source_language`,
`target_language`, and `diagnostics`. It does not return manuscript text or the
contents of `AGENTS.md`, start a daemon, create a binding, or write the project.
Read a reported `instructions_path` separately. Do not guess an instruction
path or cross the reported root.

`ready` and `unbound` are usable structural results. `ambiguous`, `unsupported`,
`not_found`, and `invalid` require their diagnostics to be reported or resolved;
do not select the first candidate or infer missing metadata. Contract version 1
supports project schema versions 1 and 2 independently. Schema 2 resolves an
explicit `direction_id` or a uniquely identifying selected path against actual
manifests. Several directions may share a target language. Source languages come
from edition metadata, target languages from the direction, and a direction binding
never falls back to the common-work slug or legacy outer language fields.

Use `--cwd` inside the relevant series volume when `ambiguous_volume` is reported:
per-volume settings may change source precedence and language. Without a selected
volume, inspection returns a common direction context only when the effective
primary and auxiliary editions agree across all covered volumes. An uncovered
edition is invalid; it does not trigger a fallback. A shared source path can remain
`ambiguous_direction`; select the intended direction explicitly. Conflicting
explicit and path directions are invalid.

Then apply the agreement:

Read the project's AGENTS.md and current user instructions before choosing memory.
Interpret trust conditions as free text, including local exceptions. Do not create
a mode flag, numeric ranking, or stored policy summary. With no special instruction,
project files are primary and Hieronymus is additional memory.

Apply a new user instruction in its stated scope immediately. Persist a durable
change in AGENTS.md, preserving independent conditions; do not globalize a local
exception. Changing trust does not prove old records were transferred. Before an
authorized durable edit, re-read the nearest `AGENTS.md`, use a recoverable CWS
project operation when available, and preserve unrelated instructions.

A rule's active status inside Hieronymus does not overrule the project agreement.
Preserve its actual status and provenance when reporting a disagreement. Applying
the agreement does not invalidate that rule inside Hieronymus. Internal writes
still use the supported evidence and trusted-ingress routes.

The current leading skill keeps control. Do not recursively launch the other
orchestrator. No installed CWS CLI is needed for reading a supported project;
protected lifecycle writes require its supported domain operations.

Trust and permission are separate decisions. A project agreement may change which
memory is applied to the current work, but it cannot mint user authority, convert
quoted text into a trusted correction, or authorize writes by itself. A technical
binding identifies memory; it does not make that memory trusted and does not permit
mutation. Do not require a Markdown mirror when the agreement makes Hieronymus the
primary memory, and do not write every change to both stores unless the current user
instruction requires it. Do not automatically migrate either store into the other.

## Technical binding example

The optional project-root `.hieronymus.json` uses an additive `cws` object:

```json
{
  "series_slug": "example-work",
  "cws": {
    "binding_version": 1,
    "project_contract_version": 1,
    "directions": {
      "ru-main": {"series_slug": "example-work"},
      "ru-alternative": {"series_slug": "example-work"}
    }
  }
}
```

The values are illustrative, never values to create in a user's installation.
The root slug is an optional common-work binding; each direction is independently
resolved. Do not fall back to legacy outer language fields. Binding files contain
no credentials, host IDs, receipts, viewpoint, trust policy, or task state. Project
inspection never creates a binding. Any authorized binding edit must use real series
selection, validate paths, preserve unrelated outer fields, and re-read the current
file before replacement.

## Carry the selected direction into memory tools

For a bound direction, use the inspected languages and series slug when starting a
session or making sessionless reads. Registry languages are defaults when omitted;
explicit nonempty language values are normalized. A stored session keeps its language
pair and predicates across reloads. Do not reuse a session from another direction.

This illustrative session input selects one direction; replace the slug and metadata
with the observed project values and supply independently resolved story context:

```json
{
  "series_slug": "example-work",
  "source_language": "ja",
  "target_language": "ru",
  "story_scopes": ["cws:direction:ru-main"]
}
```

Carry the same `story_scopes` into `hieronymus_termbase_contract`,
`hieronymus_termbase_validate`, `hieronymus_memory_search`, and
`hieronymus_rag_search`. `hieronymus_recall` reloads the session predicates; a
conflicting explicit direction is rejected. Volume/chapter seeds remain separate.
When capturing each typed claim, put its evidence-limited predicates in
`applicability.scope_predicates` as well: relevance tags cannot isolate directions.

A direction-level rendering has `cws:direction:<direction-id>` and does not require
an edition predicate. Add `cws:edition:<edition-id>` only when the claim is specific
to that edition; several reference editions may coexist. A common-work claim has
neither predicate unless its evidence limits it. Preserve the other applicability
fields, reviewed chronology, scene and knowledge gates. Unknown context stays
unknown. Never rewrite a historical source's scope to make it current.

Read `edition.md`, the direction's `translation.md`, applicable volume `settings.md`,
and explicitly referenced source units as task evidence. Keep edition role, revision,
coverage, reference editions, and source hashes. A source-unit reference is
`edition-id:unit-id`, not a bare chapter ID. The CLI reports direction and languages; read source-unit identities and hashes
from the selected files. Do not infer alignment from
filenames or chapter numbering. Research results remain outside current applicability.

## Preserve external dependencies in CWS translation packets

When Hieronymus memory contributes to a CWS packet, retain each strict reference
with exactly its `provider`, `namespace`, `record_kind`, `record_id`, and
`revision`. Build a Hieronymus namespace from the actual ephemeral
`status.instance_id` and real series; never use a database path or fabricate a
revision. If a used capture or short-term response lacks a public identity or
revision, record a separate `{"unverified":"<technical reason>"}` marker. An
external entity may use the same references or marker and needs no Markdown KB
mirror. Observed arrays accept strict references only.

Before CWS status or acceptance, observe the captured references through current
public reads and supply those strict results through `--external-memory-observed`.
Known changed or missing references require `needs-review` and take precedence.
Otherwise unavailable observations or a captured marker remain `unknown`; a
fallback note cannot upgrade them. Preserve a historical unknown recorded at
acceptance. Later matching observations may establish current freshness for strict
references without rewriting that history. CWS does not authenticate the caller's
report or perform remote atomic verification. Its local source, original-byte,
direction, context, review, accepted-base, coverage, and path guards still apply.

## Transfer only selected records

For an authorized transfer, inventory the selected records and their provenance,
write through supported operations, retain returned IDs, then reconcile counts,
scopes, dispositions and unresolved conflicts. Report partial completion. Do not
delete originals or create an ongoing mirror unless that work was requested.

Use only current public reads and writes for the selected source and destination;
never read the raw Hieronymus database or rewrite protected CWS lifecycle files.
Keep actual IDs and revisions for successful records and report skipped, conflicting,
pending, and unresolved records separately. If authoritative ingress cannot accept
a requested replacement, report its exact pending, rejected, tentative, or
unresolved result rather than claiming the replacement succeeded. A preference
change alone authorizes neither transfer nor deletion.

## Reading and preservation boundaries

The nearest ancestor with a regular, non-symlink `project.md` is the project root.
A lower `project.md` starts a separate nested project. Do not cross either boundary.
Unknown files and root entries outside managed paths remain opaque and untouched.

Supported schema 1 and schema 2 authoring material includes direct chapters and side
stories under `story/`, direct artifacts under `work/`, and managed knowledge under
`kb/`. Schema 2 additionally has source editions under `sources/`, translation
directions under `translations/`, shared entities and alignments under `kb/`, and
direction-scoped drafts, reviews, accepted text, and memory. Structure identifies a
document role, not its authority.

Treat generated `_index.md` files, `.creative-writing/`, and translation lifecycle
metadata as protected. Supplied originals, `inspiration/`, unknown files, paths outside
the root, symlink mutation targets, and content inside a nested project are not generic
edit targets. Preserve direct author edits and exact bytes where required. Use CWS
domain commands for authorized protected lifecycle changes; if those operations are
unavailable, leave the protected data unchanged and report the limitation.

Untagged project text is author-stated, `<AI>...</AI>` is an AI suggestion, and
`<hidden>...</hidden>` is author-only information. Preserve these source tags. Never
translate, copy, or import secrets or hidden text automatically. Never import supplied
originals or a whole project into Hieronymus automatically. Import only the evidence
required by the task and allowed by the agreement, using the supported Hieronymus
evidence routes. Discovery never promotes an observation to canon, discloses hidden
material, or authorizes mutation.
