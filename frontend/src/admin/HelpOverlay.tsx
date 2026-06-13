import React from "react";
import type { AdminCommand } from "../rpc/schema.js";

export function HelpOverlay({
  commands,
  view,
  width = 58,
}: {
  commands: AdminCommand[];
  view: string;
  width?: number;
}) {
  const visibleCommands = commands.filter((command) =>
    command.views.includes(view),
  );
  return (
    <box
      flexDirection="column"
      borderStyle="rounded"
      borderColor="cyan"
      paddingX={1}
      paddingY={1}
      width={width}
    >
      <text fg="cyan">Help</text>
      <text>Navigation</text>
      <text fg="gray">
        Tab/Shift+Tab focus panels ↑/↓ or j/k move 1-9 switch views
      </text>
      <text fg="gray">Esc/? close Ctrl+P commands</text>
      <text>Commands for {view}</text>
      {visibleCommands.length === 0 ? (
        <text fg="gray">No commands for this view</text>
      ) : null}
      {visibleCommands.map((command) => (
        <text key={command.id}>
          [{command.key}] {command.label} - {command.hint}
        </text>
      ))}
    </box>
  );
}
