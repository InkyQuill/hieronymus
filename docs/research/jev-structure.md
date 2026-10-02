# Reversible structure recovery: no-go for Jev

Issue #116. Live `jev-1.13.0` evaluation on 2026-10-03. **Keep original source layout and current chunking.** Do not add a network classifier to repair paragraphs: at the fixed conservative acceptance rule it almost always abstains.

The versioned synthetic fixture contains twelve RU/JA/EN boundaries: clear prose wraps, speaker changes, poetry, lists, headings, code, ambiguous prose and page boundaries. The model chooses an annotation only; it never returns replacement text. Single and repeated modes accepted 0/12 annotations; batches accepted 1/12 correctly and abstained on 11/12. No wrong annotation passed acceptance, but 35/36 abstentions mean the extra pass provides essentially no useful coverage. Reported confidence is not a calibrated layout probability.

The 26 requests consumed 13,902 input and 2,010 output tokens, with median latency 310–341 ms/request. The adjacent JSONL retains model identity, raw synthetic answers, probabilities and usage. No billing amount was returned.

This prerequisite study stopped before modifying chunking or running a retrieval benchmark. It does **not** claim measured retrieval-quality equivalence/improvement. Existing source bytes, hashes, coordinates, RAG chunks and applicability are unchanged by this decision, so their baseline behavior is preserved without classification cost. A future layout tool must retain a reversible mapping to original bytes, test normalized Unicode/repeated passages and validate any selected source span against the original snapshot before evidence capture. It must not rewrite prose, dialect, names, dialogue or intentional poetry layout.

Reproduction: shared `scripts/evaluate-jev-pilot.ts` from #112 with `scripts/fixtures/jev/structure-v1.json`, explicit private credentials and private output. Limits are 64 requests/ten minutes, 20 seconds/request, eight questions/batch and 32 KiB body, no retries. A rejected runtime experiment is the intended outcome when its cost/coverage does not support implementation; no extra fallback framework or dependency is added.
