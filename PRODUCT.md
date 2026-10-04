# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Authors and literary translators who write with an agent. The web console must remain approachable for people who do not administer software professionally.

## Product Purpose

Hieronymus gives writing agents persistent local memory. Authors inspect what their agent remembers, supply context and pointers, and flag incorrect or stale memories without maintaining a comprehensive knowledge base by hand.

## Operating Context

A local Rust server serves the Svelte web console and authenticated MCP connections. Authors install Hieronymus, connect an agent through both MCP and Hieronymus skills, verify the connection, then continue writing in Codex, Cowork or pi. The server owns the desktop tray directly or through its supervised helper. Book projects and runtime databases live outside the source repository.

## Capabilities and Constraints

- Strict terminology stays deterministic; semantic recall cannot override approved termbase entries.
- SQLite stores domain data; semantic retrieval uses exact SQLite vector search, ONNX and the pinned multilingual model.
- Provider profiles and dreaming settings support hosted and local models. Dreaming derives evidence-linked memory through seven passes.
- The official GitHub repository and explicit local artifacts are the only release sources.
- Configuration uses platform-standard user directories. Updates and removal must preserve data ownership and explicit keep-data choices.
- Browser authentication is optional and off by default; MCP access remains authenticated.

## Brand Commitments

Keep the name Hieronymus and writer-oriented, concrete language. Preserve the established interface during scoped refinements.

## Evidence on Hand

AGENTS.md records the product contract; README.md documents installation. The implemented console, semantic theme tokens and embedded fonts are the current interface evidence. User-provided screenshots show actual settings usage. Do not invent adoption, performance or qualification claims.

## Product Principles

- Let authors continue writing with their agent instead of administering a knowledge base.
- Make memory provenance, configuration and failures understandable.
- Preserve explicit author authority and approved terminology.
- Agents populate and reactivate memory automatically; authors correct mistakes rather than maintain a termbase. Ordinary recall combines crystals, recent memory and source retrieval, returning contextual or uncertain evidence with labels instead of hiding it when story context is missing. Default recall allows a generous result set; strict current-scene validation remains separate.
- Processing budgets split work into continuing batches. Valid model output is preserved in full; arbitrary output record counts never turn it into a failed Dream run.
- Keep valuable content local and protect existing data during maintenance.

## Open Decisions

No separate product-specific accessibility standard has been specified; preserve semantic controls, keyboard access and readable contrast in the console.
