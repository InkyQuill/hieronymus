# Hieronymus Management TUI

**Date:** 2026-06-06
**Status:** Draft for review

## Purpose

The Management TUI is the human control surface for Hieronymus. It should come after memory
housekeeping and agent workflows, because the UI should manage real objects and workflows produced
by actual use.

The TUI is not the primary workflow engine. It is for inspection, correction, review, and control.

## Objects Managed

The TUI should manage:

- strict concepts
- scoped renderings
- source forms
- approved variants
- forbidden variants
- evidence
- short-term memories by task/session
- long-term crystals by type
- lessons
- concept links
- dream runs
- dream outputs
- proposals
- audit history

## Core Actions

Required actions:

- add
- edit
- delete
- merge
- split
- approve
- reject
- deprecate
- supersede
- reinforce
- decay
- promote local lesson to global candidate
- activate global lesson
- inspect provenance
- inspect recall reason
- run manual dreaming
- review dream outputs

Reinforce and decay are first-class actions. Human feedback changes memory strength/confidence; it
does not have to mark a fuzzy memory as absolutely true or false.

## Views

Initial views:

- Concepts
- Renderings
- Crystals
- Lessons
- Short-Term Sessions
- Dream Runs
- Proposals
- Audit Log

Each view should support filtering by series, language pair, status, type, confidence, strength,
cycle, and tags where applicable.

## Interaction Style

The first TUI should be keyboard-first and operational. A Textual/Rich-style interface is likely a
good fit:

- dense tables
- detail panes
- filters
- command palette
- modal action dialogs
- side-by-side provenance inspection

Avoid building a graphical knowledge editor until real usage shows that a visual graph is needed.

## Dream Review

Dream runs should show:

- source task sessions
- short-term memories consumed
- crystals created
- crystals updated
- crystals decayed
- strict concept/rendering proposals
- failed LLM outputs
- validation errors

The user should be able to accept, edit, reject, reinforce, decay, or supersede dream outputs.

## Non-Goals

- Do not build this before the memory/dreaming and agent workflow layers exist.
- Do not make the TUI a separate source of truth.
- Do not require a web UI for the first management surface.

