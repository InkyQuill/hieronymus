# Multilingual entity alignment: keep advisory, no runtime addition

Issue #114. Live `jev-1.13.0` pilot on 2026-10-03. **Do not introduce automatic entity alignment or a new Jev request on every concept lookup.** The existing concept/alias storage remains useful; this result concerns the proposed classifier.

Twelve synthetic cases cover RU/JA/EN aliases, an explicitly evidenced Japanese/Russian rendering, siblings, same titles across time, homonyms, unsupported spelling similarity, scope mismatch and injected instructions. The rubric distinguishes same entity, related distinct entities, different entities and insufficient context. At fixed confidence and chosen probability >=0.95, single/repeat/batch accepted respectively 6/12, 7/12 and 3/12 correct decisions, with 6/12, 5/12 and 9/12 abstentions. No accepted wrong identity appeared in this small set. Batch behavior is materially less decisive here; do not infer equivalence from matching labels alone.

The 26 requests consumed 13,824 input and 2,001 output tokens; median latency was 296–309 ms/request. Raw synthetic responses record model, probabilities, confidence, latency and usage. Dollar cost is unavailable. These are answer-acceptance measurements, not calibrated identity probabilities or a large-corpus alias benchmark.

This no-go stops before building persistence or cache integration. No candidate is merged, no relation becomes authoritative, and no strict term mapping is added. Consequently no stale cached identity can affect runtime. If revisited, code must first produce a same-scope shortlist, bind proposals to exact source/evidence IDs and revisions, and invalidate them after changes. A model label must never replace those checks; uncertainty retains separate records.

Reproduce with `scripts/evaluate-jev-pilot.ts` from the support study (#112), `scripts/fixtures/jev/alignment-v1.json`, an explicit private config and private output. Bounded to 64 requests/ten minutes, 20 seconds/request, eight questions/batch, 32 KiB, no retry. The fixture and report are independently reviewable; this PR targets main. The lack of accepted errors is insufficient grounds to add another runtime dependency on network judgment.

Inspiration: [TypeSafe entity alignment](https://docs.typesafe.ai/cookbooks/entity_alignment). Its taxonomy is a starting point, not proof of literary identity correctness.
