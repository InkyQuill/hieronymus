import React from "react";
import type { AdminCommand } from "../rpc/schema.js";
import { theme } from "../ui/theme.js";

export function commandsForView(
  commands: AdminCommand[],
  view: string,
  hasSelection: boolean,
): Array<AdminCommand & { disabled: boolean }> {
  return commands
    .filter((command) => command.views.includes(view))
    .map((command) => ({
      ...command,
      disabled: command.requires_selection && !hasSelection,
    }));
}

export function CommandPalette({
  commands,
  selectedIndex,
  width = 54,
}: {
  commands: Array<AdminCommand & { disabled: boolean }>;
  selectedIndex: number;
  width?: number;
}) {
  const commandRows = Math.max(commands.length, 1);
  const hintRows = commands[selectedIndex] ? 1 : 0;
  const borderAndPaddingRows = 4;
  const height = 1 + commandRows + hintRows + 1 + borderAndPaddingRows;

  return (
    <box
      flexDirection="column"
      borderStyle="rounded"
      borderColor={theme.accentPrimary}
      paddingX={1}
      paddingY={1}
      width={width}
      height={height}
    >
      <box height={1}>
        <text fg={theme.accentPrimary}>Command Palette</text>
      </box>
      {commands.length === 0 ? (
        <box height={1}>
          <text fg={theme.accentMuted}>No commands for this view</text>
        </box>
      ) : null}
      {commands.map((command, index) => (
        <box key={command.id} height={1}>
          <text
            fg={
              index === selectedIndex
                ? command.disabled
                  ? theme.statusWarning
                  : theme.accentPrimary
                : undefined
            }
          >
            {index === selectedIndex ? "> " : "  "}
            {command.disabled ? "✕ " : ""}
            {command.label} [{command.key}]
          </text>
        </box>
      ))}
      {commands[selectedIndex] ? (
        <box height={1}>
          <text
            fg={
              commands[selectedIndex].disabled
                ? theme.statusWarning
                : theme.accentMuted
            }
          >
            {commands[selectedIndex].disabled
              ? `${commands[selectedIndex].label} needs a selected row`
              : commands[selectedIndex].hint}
          </text>
        </box>
      ) : null}
      <box height={1}>
        <text fg={theme.accentMuted}>Enter run Esc close ↑/↓ or j/k move</text>
      </box>
    </box>
  );
}
