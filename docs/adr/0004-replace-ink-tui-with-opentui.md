# Replace Ink/React TUI with OpenTUI React TUI

## Context
In ADR 0002, we migrated the terminal user interfaces (`hiero config` and `hiero admin`) from Python's Textual framework to a TypeScript Ink/React frontend running under Node.js. While this successfully decoupled visualization from domain logic via a stdio JSON-RPC bridge, the Ink ecosystem has several limitations for building advanced, premium terminal applications:
1. **Focus and Keyboard Management**: Ink has limited layout-driven keyboard and focus lifecycle control. Inputs, panels, and modal overrides require custom hooks catching raw keystrokes manually.
2. **Text Formatting Constraints**: Ink does not support standard inline modifier elements natively inside block text, forcing verbose nested markup tags.
3. **Lack of Advanced Components**: Advanced presentation primitives (such as native multi-line textareas, unified/split diffs, syntax highlighting, scrollboxes, and selection lists) are either missing or require custom implementation.
4. **Performance & Runtime Overhead**: The Node.js runtime environment has larger memory footprint and start times than modern local-first engines.
5. **Static/Limited Visual Feedback**: Basic textual loaders lack dynamic, eye-catching visual feedback. Premium interactive terminal interfaces require modern unicode spinners and animations to denote active execution and background operations.

## Decision
We will replace the TypeScript Ink/React frontend with an OpenTUI React frontend (`@opentui/react` and `@opentui/core`) running on the native Bun runtime:
1. **Migration to OpenTUI React**: All React components (`App`, `ConfigScreen`, `AdminScreen`, and the shared UI widgets) will be ported from Ink to `@opentui/react`.
2. **First-Class Layouts and Form Fields**: Use native `<box>`, `<scrollbox>`, and `<input>` elements. Custom input event capturing will be replaced by native React inputs.
3. **Upgrade Visual Design and Interactivity**:
   - Utilize OpenTUI's flexible border styles (`single`, `double`, `round`) and direct layout styling props.
   - Leverage native scrollboxes for the `Detail Inspector` and `AdminTable` data views, enabling clean navigation and rendering.
   - Improve typography and colors using HSL-tailored/RGB color structures.
   - Use the native `<code>` and `<diff>` components in the Detail Inspector to display code chunks, rule crystals, and dreaming memory diffs with proper syntax highlighting.
   - Integrate the `unicode-animations` library to render high-fidelity Unicode spinners (e.g. `helix`, `pulse`, `braille`) for bootstrap loading states, in-progress saving operations, and background dream/drain execution panels.
   - Integrate subtle micro-animations (e.g., selection transitions, status bar fade-ins) using `useTimeline` for a modern, responsive TUI.
4. **Bun Runtime Execution**:
   - The CLI backend will invoke the built frontend bundle via `bun`, which is configured in `mise.toml`.
   - Update `Doctor` checks (`doctor.py`) to verify `bun` availability instead of `node` and `pnpm`.
   - The frontend project package manager will be configured to use `bun` natively.
5. **JSON-RPC Stdio Bridge**: The JSON-RPC bridge protocol remains the same, but the frontend client runtime shifts from Node-specific primitives to Bun/native ones.

## Consequences
- The Node.js runtime dependency is replaced by Bun. Bun's faster startup and lower overhead improve the CLI responsiveness.
- Improved terminal UI robustness, thanks to OpenTUI's Zig-based terminal rendering and cleaner event loop integration.
- Higher fidelity, scrollable, and syntax-highlighted inspector screens that support mouse scroll and click.
- Standardized form handling simplifies the config panel and eliminates manual key listeners.
- Enhanced interactivity and user experience from rich, multi-threaded feel of animated loaders and background task status indicators.
