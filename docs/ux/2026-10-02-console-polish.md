# Author-facing console polish

The primary user writes with an agent. The console helps them connect it, inspect its memory, correct errors and configure processing. It should not suggest that maintaining a database is a prerequisite for writing.

## Changes

- Settings share the same navigation across providers, Dreaming, incoming memory and updates. AI providers are distinguished from the author's writing agent.
- The overview describes actual scheduling state and pending memory counts instead of displaying an absent status as `unknown` or implying disabled processing is ready to run. Processing cannot be submitted again while busy.
- Incoming-memory limits explain warnings, rejection and disabled character limits. Update settings explicitly describe channel selection, not installation; development means published prereleases.
- Memory views explain crystals and recent memory. Empty results explain how memories arrive instead of asking the author to select a nonexistent record. Technical record fields are available under a disclosure.
- Connection instructions distinguish the MCP connection from agent skills. Provider limits are advanced settings. Provider deletion requires a second deliberate action.
- Loading errors offer a retry; settings and main navigation identify the active page for assistive technology.

## Verification and limits

All seven routes were rendered in headless Chromium at 1440px dark and 390px light with synthetic API fixtures. There was no page overflow or JavaScript exception. Screenshots were inspected locally. Frontend regression checks cover disabled processing, pending counts, active navigation and provider deletion confirmation.

Fixtures establish presentation behavior, not live provider availability or acceptance by an agent host. This work does not change the installed runtime or the visual theme. Internal memory categories and diagnostic views remain available; consolidating those categories would be a separate information-architecture change rather than a visual polish.
