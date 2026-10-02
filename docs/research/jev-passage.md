# Passage classification and reranking: no-go

Issue #113. Live pilot, 2026-10-03, requested/reported `jev-1.13.0`. **Keep the existing retrieval order and do not insert a Jev filtering/reranking step.** The bounded classification prerequisite did not provide enough reliable coverage, and a batch confidently resolved an ambiguous pronoun without evidence.

Twelve synthetic RU/JA/EN passages test answer support, explicit contradiction, related-but-unanswered content, unrelated technical text, temporal mismatch, viewpoint, fictional commands and prompt injection. These are relevance/evidence classes, not generic content moderation. At the fixed 0.95 confidence-and-probability floor:

| Mode | Correct accepted | No accepted decision | Wrong accepted |
|---|---:|---:|---:|
| Single | 3/12 | 9/12 | 0 |
| Repeat | 3/12 | 9/12 | 0 |
| Batch | 3/12 | 8/12 | 1/12 |

The error classified “У неё девятый уровень” as answering Mira's level despite no evidence identifying “her”. The 26 requests used 14,679 input and 2,264 output tokens; median request latency 297–357 ms. Raw synthetic answers, probabilities and usage are in the adjacent JSONL. No billing value was returned. Agreement between repeats is not calibration.

This is an early prerequisite rejection, **not a measured end-to-end ranking win/loss**. We did not reorder production top-K, drop candidates, or claim MRR/nDCG improvement. Building the optional runtime adapter after this result would add disclosure, latency and fallback complexity without evidence of usefulness. Existing eligible candidate IDs, original ranks, authority and scope remain untouched. A future reranking experiment must retain that baseline and separately measure relevance and answerability, while returning the original order on abstention/unavailability.

Reproduction uses `scripts/evaluate-jev-pilot.ts` from the support-research PR (#112) with `scripts/fixtures/jev/passage-v1.json`, an explicit private relevance config and private output path. The harness is shared research tooling; this independent PR changes no runtime code and targets main, not another PR branch. Limits: 64 requests, ten minutes, 20 seconds/request, eight questions/batch and 32 KiB body, with no retries.

The rubric's `answers` and `contradicts` classes overlap for yes/no questions; do not interpret these small-set labels as universal relevance truth or tune thresholds to erase the distinction. The concrete ambiguous-identity error independently rules out automatic evidence filtering. Inspiration: [TypeSafe reranking](https://docs.typesafe.ai/cookbooks/rerank_typesafe).
