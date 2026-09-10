# Deferred native-host qualification — 2026-09-10

The owner deferred the complete Claude/Codex/Pi native matrix to the week of 2026-09-14.
Missing native evidence does not block the current implementation/merge milestone. Domain
security and correctness checks still apply. No native-host or public-release acceptance is
claimed.

| Case | Claude | Codex | Pi | Remaining boundary |
|---|---|---|---|---|
| 1 correction/revision | Partial | Partial | Pending | uninterrupted revision and all Pi evidence |
| 2 invalidation | Partial | Partial | Pending | consolidation/no replacement and all Pi evidence |
| 3 relevance feedback | Partial | Pending | Pending | activation, miss, replay, durable single delta |
| 4 ambiguity | Observed bounded | Observed bounded | Pending | Pi evidence |
| 5 learned sequence | Partial | Observed | Pending | Claude replay/bind and Pi evidence |
| 6 viewpoint manifests | Observed | Observed | Pending | Pi evidence |
| 7 outage/recovery | Failed | Failed | Pending | valid recovery and Pi outage path |
| 8 ordinary workflow | Partial | Partial | Pending | uninterrupted workflows and Pi evidence |
| 9 native/semantic | Partial | Partial | Pending | complete per-host evidence |

Later qualification also covers identical-prompt identity, saved-delivery retry, ordinary
origin rejection, actor/source-role and fabricated/cross-session negatives, and same-turn
steering. Pi must separately qualify optional isolated trusted ingress. Normal Pi package
loading is passive so prompts, packaged skills, and `pi-mcp-adapter` MCP context reads work
without special extension order; it never mints trusted provenance. Streaming slash commands
are outside the trusted path and must be retried idle.

Current practical acceptance remains limited to verifying the MCP build and tools, agent DB
saves, agent-launched RAG, Ollama embeddings, Dream through Ollama or an OpenAI-compatible
provider, and marketplace-installed skills/app callability. Those checks do not complete the
matrix above.

Candidate evidence remains scoped: C0 source `81bd23bdebede7692573e4764db3ffcdbec1e7bb`,
archive `4d8a1fb4909effcb00e8ab8048246a1a1f83b1c8be448cc0ed3cca9f9003b6c3`, native
`db37c69d686d876a9cb705ca81f6e46cf30a04df43c9fa884d2082112433e67e`; C2 source
`fbbe4070b5fb81f7a5ff64940d23f13c9c43d132`, archive
`7c680e09ff81ac94d9874e72ab257bff6880d70a6ecd89cf55bc53ba9d855345`, native
`ea9f706c508cbb98319910a8143f08f7c55fba9bd4edf3668858c54f3a656735`; C4 source
`99dee83`, archive `b5be7c564621fa98cbaceff5569a53e2d0139d506f404be0e94da60641066a10`, native
`bbd6a80a3a811cac1c58c094c78f2548855cb610d389001d9523701930cf85be`.
Pi has no qualified installed candidate.

Detailed provenance remains in the SDD `task-7-current-acceptance-matrix.md`, Claude reports
under `task-7-native-acceptance/claude-extension-1`, Codex reports under
`task-7-codex-acceptance`, and the adjacent Pi design/review/report. Historical zCode failures
stay paused under zCode and cannot qualify Pi. Earlier provider-schema, stale-turn, binding,
and qualification failures remain recorded.

See the [implementation plan](../superpowers/plans/2026-09-06-autonomous-authority-implementation.md),
[design specification](../superpowers/specs/2026-09-06-autonomous-authority-and-corrections.md),
and [host acceptance](../agent-host-acceptance.md).
