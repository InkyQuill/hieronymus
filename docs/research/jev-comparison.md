# Jev literary comparison evaluation

Issue #111. Measured 2026-10-03 with requested and reported `jev-1.13.0`.

Keep the conservative 0.95 acceptance policy and deterministic provenance/anchor vetoes. This pilot does **not** justify lowering thresholds or automatically widening destructive consolidation. No false equivalence passed the parser in this fixture, but abstention is frequent. A small synthetic sample cannot establish production calibration.

## Reproduction

`bun scripts/evaluate-jev.ts /private/relevance.conf /private/results.json`

The command is explicitly opt-in, uses the model in that TOML file, and fails on missing credentials, HTTP failure or exhausted budget. It makes no retries: at most 100 requests, 15 minutes total, 20 seconds per request, 8 questions per batch, 32 KiB per request. Run output must be in a private directory. It checkpoints answers and usage without input text or credentials. This is a direct API rubric evaluation; SDK transport tests remain separate. No additional dependency is needed.

Fixture: `scripts/fixtures/jev-comparison-v1.json`. Rubric and parser versions are part of the production comparison cache identity, including the identity used by durable Dream work. A regression verifies that changing either version changes that identity. Bump the relevant version when changing the contract.

## Synthetic pilot

24 manually authored cases: English, Russian, Japanese, one cross-language pair, paraphrases, negation, numbers, chronology, viewpoint, ambiguous references, indirect inference, names, scope and embedded instructions. Gold labels follow the **current application rubric**: changed numbers are `distinct`, even where a general logical classifier might call them contradictory. The distinction is preservation-safe, but this overlap deserves attention before future rubric revision.

| Mode | Requests | Judgments | Correct including warranted abstention | Abstentions | False equivalence |
|---|---:|---:|---:|---:|---:|
| Single | 24 | 24 | 14 | 14 | 0 |
| Repeat | 24 | 24 | 13 | 15 | 0 |
| Batch of 8 | 3 | 24 | 14 | 14 | 0 |
| Unrelated injected distractor | 24 | 24 | 14 | 14 | 0 |

The 75 requests consumed 42,894 input and 5,302 output tokens. Median request latency was about 309–355 ms across modes; exact distributions, reported models, probabilities, confidence and usage are in the accompanying JSON. Billing amounts were not returned, so no dollar cost is inferred. Batch mode reduced input tokens and request count; this small result is not a concurrency/load benchmark.

Four expected `insufficient_context` cases per mode count as correct abstentions. Other abstentions are missed decisions. Repetition agreement is **not** calibration. The strict parser was reapplied offline to every saved raw response to match production, without further model calls. Raw model confidence alone is not acceptance: the chosen probability must also meet 0.95 and the distribution must be valid.

## Private corpus controls

An owner-authorized SQLite online backup contained 2,618 `only-sense-online` crystals. Twelve deterministic samples of 80–600 characters were compared with themselves in the same four modes: all 48 judgments were equivalent (38 requests). These are identity controls, **not** independently labeled paraphrase/contradiction quality evidence. No text, database IDs, source paths or private inputs are committed. This evaluation does not modify the source database or run Dream on it.

## Limits and follow-up

This is a comparison pilot, not a general Jev endorsement. Literary-vs-technical relevance needs its own rubric and gold set; do not reuse comparison thresholds as evidence for relevance. Support checking, reranking, entity alignment and structure recovery each have separate issues and must earn their own decisions. The multilingual sample is too small to estimate rare destructive errors or robust per-language accuracy. The separate relevance calibration is recorded below.

The harness measures Jev answer acceptance, not the full production routing pipeline. Invalid answers are counted as abstentions here; production returns an error and may try an explicitly configured fallback. Production also applies provenance and semantic-anchor vetoes before asking Jev. Output files are created privately before writing because a provider could include unexpected echoed text in an answer.

## Separate production relevance check

The exact current Rust `literary`/`technical` Noul questions were also tested on twelve versioned RU/JA/EN messages, twice each (24 requests). At the configured 0.85 literary minimum and 0.15 technical maximum, 22/24 matched the authored capture labels. Both errors rejected the same Japanese character fact (`ja-character`); no technical or mixed message was accepted. These are message-capture decisions, not comparison decisions. Do not transfer the comparison 0.95 threshold here or tune the production thresholds around this single case.

The adjacent relevance JSONL contains synthetic answers and usage. Reproduce with `bun scripts/evaluate-jev-relevance.ts /private/relevance.conf /private/results.json`; it extracts the exact current Rust question literal, fails if its shape changes, records its hash, uses explicit configured thresholds and caps the experiment to 24 requests/ten minutes with no retry. An independent choice-rubric exploration was less decisive and is not substituted for this production-rubric result.

The explicitly ignored Rust `live_jev_synthetic_relevance` test also passed locally with four synthetic RU/EN inputs through the actual SDK/transport path. Private corpus identity controls remain separate from gold accuracy. This completes the bounded calibration task; the Japanese false negative is a measured limitation, not a reason to silently lower the capture gate.
