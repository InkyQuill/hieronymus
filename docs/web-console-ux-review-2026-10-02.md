# Web console UX review — 2026-10-02

The console supports authors inspecting agent memory, correcting it and connecting their writing agent. Its ordinary language now explains the purpose and next action; technical inspection remains available through explicit disclosures. The existing typography, palette and navigation identity are preserved.

## Coverage

| Surface | Author-facing behavior | Technical access |
| --- | --- | --- |
| `/`, `/admin/connect` | Three-step agent setup explains MCP, skills and verification by the agent | Setup request remains readable and manually copyable |
| `/admin` | Memory counts use familiar names; processing state and service problems link to relevant settings/history | Provider checks, processing stages and validated service data |
| `/admin/memory` — Crystals | Long-term memories; read before managing records; correction remains prominent | Original view ID, row, fields and detail payload |
| Concepts | Ideas and subjects, with an explanation of recurring characters and terminology | Original concept data and available server commands |
| Renderings | Translation history explicitly distinguished from current approved choices | Source fields, original record payload and correction selection |
| Lessons | Writing lessons, with a reminder to check project relevance | Original record data and advanced memory actions |
| Short-Term Memory | Recent memories; processing completion is distinguished from correctness | Source text, metadata and exact record payload |
| Short-Term Sessions | Working sessions and their reading context | Session fields and original identifiers |
| Dream Runs | Processing runs across all books, including failures | Outcome text, run metadata and review actions |
| Dream Audits | Processing events, with raw event bodies under technical inspection | Original event body and metadata remain available |
| Audit Log | Change history across all books | Original decision, scope and record data |
| `/config` and provider editor | Separates memory-processing AI from the writing agent; explains stored keys and optional Ollama keys | Profile ID, endpoint, model limits, discovery, connection check and saved profile metadata |
| `/config/dreaming` | Schedule and shared instructions first; seven tasks explained and ordered by processing phase | Per-task model/prompt settings, advanced limits and full current configuration |
| `/config/ingest` | Explains warning/rejection consequences and separate relevance saving | Memory limits and relevance configuration; advanced thresholds disclosed |
| `/config/release` | Explains channels and that saving does not install an update | Current update configuration |
| Correction and action forms | Keep exact selection and destructive confirmation; technical correction selection available | Evidence, revisions and original selection fields |

## Shared behavior

- Memory view labels are separate from stable daemon/API IDs; existing deep links continue to work.
- Settings report whether current values match saved values. Native form validation runs before saving; invalid numeric inputs reveal their containing disclosure.
- Technical panels preserve field names, support copying and offer manual selection when clipboard access fails. Readiness uses its validated projection so unexpected credential/diagnostic fields are not rendered.
- Small screens use a memory-section select. Selecting a record scrolls to its detail panel. Tables scroll within their container; the page itself does not overflow horizontally.
- The main header stays in normal document flow on small screens, leaving space for reading. Keyboard focus and a skip link remain available.
- Storage status and model confidence are not presented as proof that a memory is true.

## Verification and limits

- Initial review used the installed service at `http://127.0.0.1:9768/` (reported server version 0.10.1).
- Built frontend verified through a disposable, read-only preview at `http://127.0.0.1:9770/`, using the running service's GET data. POST/DELETE operations were blocked by the preview; no author data or provider settings were changed during browser review.
- Desktop and 390px mobile layouts inspected; all nine memory views visited, including empty translation/subject views and real processing failures. A second bounded inspection confirmed the compact task settings and reading-first detail panel.
- `bun run typecheck`, `bun run test` (109 tests), and `bun run build` passed. `git diff --check` passed. Impeccable's detector returned no findings.
- Existing tests still verify canonical action IDs, explicit destructive confirmation, stale-response handling, book scoping and pagination. New tests exercise technical copying/fallback and saved-setting status.
- The installed daemon was not replaced or restarted. Shipping the new embedded UI requires a build/install of the intended source revision. Rust changes from other ongoing work were outside this review.
- Screenshots for this review are local artifacts under `/tmp/hieronymus-ux-2026-10-02/`; they contain actual project data and were not added to the source repository.
