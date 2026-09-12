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
supports project schema versions 1 and 2 independently. Schema 2 is structurally
readable, but actionable direction selection currently returns
`unsupported_direction_selection`. Do not infer a translation direction,
edition, language, or series from a map key, directory name, or legacy outer
binding fields.

Then apply the agreement:

Read the project's AGENTS.md and current user instructions before choosing memory.
Interpret trust conditions as free text, including local exceptions. Do not create
a mode flag, numeric ranking, or stored policy summary. With no special instruction,
project files are primary and Hieronymus is additional memory.

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
