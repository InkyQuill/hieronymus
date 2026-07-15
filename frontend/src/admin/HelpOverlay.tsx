import React from "react";
import type { AdminCommand } from "../rpc/schema.js";
import { theme } from "../ui/theme.js";

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
      borderColor={theme.accentPrimary}
      paddingX={1}
      paddingY={1}
      width={width}
    >
      <text fg={theme.accentPrimary}>Help</text>
      <text>Navigation</text>
      <text fg={theme.accentMuted}>
        Tab/Shift+Tab focus panels ↑/↓ or hjkl move 1-9 switch views
      </text>
      <text fg={theme.accentMuted}>/ search Esc/? close Ctrl+P commands</text>
      <text>Commands for {view}</text>
      {visibleCommands.length === 0 ? (
        <text fg={theme.accentMuted}>No commands for this view</text>
      ) : null}
      {visibleCommands.map((command) => (
        <text key={command.id}>
          [{command.key}] {command.label} - {command.hint}
        </text>
      ))}
    </box>
  );
}
