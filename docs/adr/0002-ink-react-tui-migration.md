# Replace Textual TUI with TypeScript Ink/React TUI

## Context
Hieronymus historically exposed two interactive terminal user interfaces (`hiero config` and `hiero admin`) built using Python's Textual framework. To allow TypeScript to own interactive terminal rendering, raw ANSI composition, local UI state management (using Nanostores), keyboard layout/focus handling, and dialog layout while keeping the domain/storage logic strictly in Python, we require a framework migration.

## Decision
We migrated the Textual TUI to a TypeScript Ink/React frontend, preserving the Python backend as the single source of truth for storage, settings, dreaming cycles, rule crystals, and migrations:
1. **JSON-RPC Bridge**: A stdio-based JSON-RPC bridge protocol is implemented at `src/hieronymus/tui_bridge/` to serialize requests and responses between Python and the TypeScript UI.
2. **Ink Frontend**: An independent `frontend/` project is created, utilizing React 19, Ink 7, Zod for payload validation, and Nanostores for state.
3. **Packaging & Delivery**: The frontend is built into `frontend/dist/main.js` and bundled directly inside the Python wheel via Hatch.
4. **Node Runtime Requirement**: The Python CLI invokes the built JavaScript bundle using Node.js. `hiero doctor` reports Node.js runtime availability.
5. **Eradication of Textual**: The transition phase is complete; the `HIERONYMUS_TUI` environment variable has been removed, the legacy Textual codebase has been deleted, and the React/Ink TUI is now the default and sole terminal UI.

## Consequences
- Clean separation between terminal visualization and domain validation.
- Node.js is a runtime dependency for terminal TUI users (non-TUI JSON/MCP commands are unaffected).
- Secret API key values are redacted at the JSON-RPC boundary before entering the TypeScript process space.
- Legacy Textual dependency and all related modules are completely eradicated.
