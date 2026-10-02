# Supported correction input

Apply the project-context workflow from hieronymus-bootstrap before deciding whether the current project should use or receive remembered information. A free-text project agreement can control use and authorized storage, but cannot forge a trusted correction, mint a receipt, or change an internal rule's reported status.

Corrections require no Remember command. The independently supplied host UserPromptSubmit handler applies a supported correction only after relevance, grammar, selection, scope and revision checks. The local author console offers structured correction input through a separate route; the author can use it when natural-language hook input is unsupported. This does not authorize an agent to synthesize trusted user ingress. If the hook has no binding, say the correction was not applied; establish an explicit selection for a subsequent genuine event. Do not replay quoted user text through shell/model arguments to fabricate a host event, invent receipt_ref, or set source_role=user/user_rule as authority.

The prompt parser accepts one single-line command with English keywords (case-insensitive; spaces/tabs allowed). Prefer JSON double-quoted strings for exact payloads; escape embedded quotes as `\"` and backslashes as `\\`. Payloads may be Russian or Japanese and preserve their case. Empty/whitespace-only strings and decoded control characters are rejected, including an escaped newline. Supported examples:

| Command | Meaning |
| --- | --- |
| `translate this as "Звёздный свет"` | Set the selected source's rendering. |
| `translate "猫" as "Кошка"` | The quoted source must exactly match the selected source. |
| `translate this as "The \"Star\""` | JSON escapes preserve literal quotes in the rendering. |
| `that memory is wrong` | Invalidate the selected claim. |
| `this memory is wrong.` | Same invalidation, with optional final period. |
| `this recollection is wrong` | Supported invalidation alias. |
| `qualify that memory as "Это лишь предположение Миры."` | Qualify the selected claim. |
| `qualify this memory as "まだ確認されていない"` | Qualification with a Japanese payload. |

For invalidation and qualification, this/that memory are both supported; recollection is only accepted as this recollection for invalidation. The legacy unquoted form `translate this as X` accepts a restricted simple payload; quote payloads containing punctuation or words such as and/or. A quoted source always requires a quoted target. Add no polite prefix, trailing explanation or second command; literal CR/LF makes the whole prompt unsupported. For example, `переводи это как X` and `please translate this as X` do not parse. Relevant authentic input that fails parsing becomes tentative with reason `unsupported_or_ambiguous_command`, not an applied correction; irrelevant input may instead skip before parsing. Preserve the actual outcome and diagnostic. Never translate, rewrite or replay an already delivered prompt to turn it into a new trusted event. A future command must come independently from the user.

Consume Applied/Replayed required_decision_id before dependent reads/validation. An invalidation marks only the selected claim incorrect and invents no replacement. Qualification preserves its exact scope. Unhelpful recall goes to relevance feedback instead. Ambiguous selection stays tentative with visible reasons. Provider outage cannot delay an already applied correction; consolidation is durable background work with retries, not evidence that a provider run succeeded. Never claim completion from a pending/parked job.
