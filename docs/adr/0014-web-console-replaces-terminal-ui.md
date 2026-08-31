# Replace The Terminal UI With The Svelte Web Console

## Status

Accepted on 2026-08-31. This ADR supersedes ADR 0002, ADR 0004, and the
React/OpenTUI product-surface, terminal-application, install/runtime, non-goal,
and test references in ADR 0005. Those texts remain historical descriptions of
retired implementations.

## Context

The Ink and OpenTUI applications have been removed. The current interactive
frontend is a Svelte 5 web console served by the local daemon. Rust migration
specifications consistently embed and serve that console, while accepted ADRs
still describe a Bun-hosted terminal UI and say that Hieronymus must not require
a browser.

## Decision

The Svelte 5 web console is the only first-class interactive configuration and
administration UI. `hiero config` and `hiero admin` start or discover the local
daemon, create a single-use launch grant, and open the corresponding loopback
web route. The frontend uses authenticated HTTP and WebSocket contracts and
never writes SQLite or configuration files directly.

A browser is required only for interactive UI use. Headless CLI commands, MCP
stdio, native MCP HTTP, daemon operation, migration, backup, doctor, import,
export, and validation remain usable without a browser. Electron, an embedded
browser runtime, React/Ink, and React/OpenTUI are not distribution requirements.

Release artifacts embed the built Svelte assets. Bun is a pinned build-time CI
dependency, not an end-user runtime dependency. Frontend compatibility means
the Svelte client's route, DTO, authentication, navigation, and WebSocket
contracts recorded in the compatibility manifest; it does not include terminal
rendering or keyboard behavior.

## Consequences

ADR 0005's product surface 4 is read as “Svelte web console,” its terminal UI
section is historical, and its “do not require a browser” non-goal is replaced
by the narrower headless-operation guarantee above. Tests protect web-console
launch and contracts rather than OpenTUI launch behavior.
