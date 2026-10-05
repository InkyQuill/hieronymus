# Documentation

Start with the guide for your task. Hieronymus is a local memory service for
writing agents; you do not need to maintain a knowledge base to use it.

## For authors and translators

| I want to… | Read |
| --- | --- |
| Install and start | [Project introduction](../README.md#install) |
| Connect my agent, configure processing, update or remove the app | [Usage](usage.md) |
| Understand what is remembered and how corrections work | [Business logic](business-logic.md) |
| Understand pending memories, Dreaming and comparison settings | [Memory and Dreaming](memory-dreaming.md) |
| Use project files and memory together in translation | [Translation integration](translation-workspace-integration.md) |
| Diagnose startup or service problems | [Service operations](service-toolkit.md) |

## For contributors and integration authors

[PRODUCT.md](../PRODUCT.md) states the purpose and product principles.
[DESIGN.md](../DESIGN.md) owns interface conventions. [AGENTS.md](../AGENTS.md)
owns development and documentation rules. [Roadmap](roadmap.md) lists unresolved
work; completed work belongs in Git history and release notes.

| Area | Maintained reference |
| --- | --- |
| Agent setup and skills | [Agent workflows](agent-workflows.md), [Read/Learn/Remember](skills/read-learn-remember.md) |
| Corrections, evidence and revisions | [Authority ingress](authority-ingress.md) |
| Prompt binding, capture and session lifecycle | [Hook context](agent-hook-context.md) |
| Decision protocol and comparison batching | [Decision adapter](jev-sdk-batching.md), [upstream provenance](../crates/hiero-decision/UPSTREAM.md) |
| Storage and semantic verification | [Semantic validation](semantic-validation.md), [runtime checks](rust-cutover-rehearsal.md) |
| Build, packaging and publication | [Distribution](distribution.md), [automatic releases](automatic-releases.md), [CI checks](ci-checks.md) |
| Desktop behavior | [Tray and registration](desktop-tray.md), [Linux](desktop-linux.md), [macOS](desktop-macos.md), [Windows](desktop-windows.md) |
| Native packaging and evidence | [Platform artifacts](desktop-platforms.md), [desktop checks](desktop-qualification.md), [filesystem guarantees](desktop-filesystem-qualification.md), [agent-host checks](agent-host-acceptance.md) |
| Compatibility inputs | [Protocol definitions and fixtures](../compatibility/README.md), [qualification assets](../qualification/README.md) |

## Why these rules exist

ADRs record decisions and tradeoffs, not execution reports. Their main text states
the current accepted policy. An accepted decision is not proof of implementation
or native qualification.

| Decision | ADR |
| --- | --- |
| Local plaintext configuration | [0001](adr/0001-dream-conf-plaintext-secrets.md) |
| Multilingual concept identity | [0003](adr/0003-multilingual-concept-centered-memory-graph.md) |
| Alpha versioning and release authority | [0006](adr/0006-alpha-versioning-and-release-authority.md) |
| Providers, workflows and bounded model decisions | [0007](adr/0007-provider-catalog-and-workflow-assignments.md) |
| Rust runtime and one-way cutover | [0008](adr/0008-rust-reimplementation-authority-and-cutover.md) |
| Daemon ownership and process boundaries | [0009](adr/0009-runtime-topology-and-daemon-lifecycle.md) |
| Data locations, upgrades and recovery | [0010](adr/0010-data-locations-schema-ownership-and-upgrade.md) |
| Deterministic terminology versus ranked memory | [0011](adr/0011-deterministic-terminology-and-graded-memory.md) |
| Authentication and discovery | [0012](adr/0012-mcp-transport-authentication-and-discovery.md) |
| Derived SQLite vectors and semantic readiness | [0013](adr/0013-semantic-index-and-platform-support.md) |
| Author-facing web console | [0014](adr/0014-web-console-replaces-terminal-ui.md) |
| MCP protocol and stdio negotiation | [0015](adr/0015-mcp-protocol-and-transport.md) |
| Autonomous story memory and user corrections | [0016](adr/0016-autonomous-story-memory-product-vision.md) |

## History and special-purpose Markdown

Completed plans, research and receipts are outside the maintained reading path.
Earlier committed versions are available with `git log --all -- <path>` and
`git show <revision>:<path>`. The previous Python application is on
[stale/python-v0.7.0](https://github.com/InkyQuill/hieronymus/tree/stale/python-v0.7.0).
[Archive details](archive/python-v0.7.0.md) distinguish that application from later
migration tooling.

Local originals, drafts and audit reports live under ignored `docs/.local/`.
They are not shared product requirements. CHANGELOG.md is release history;
Markdown in `crates/hiero/resources/` is embedded agent input; component
provenance and icon notices stay beside their assets. Personal `.remember/`
notes and ignored `.agents/` skills remain outside the public documentation.
