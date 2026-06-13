import React from "react";
import type { AdminCommand } from "../rpc/schema.js";

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
}: {
  commands: Array<AdminCommand & { disabled: boolean }>;
  selectedIndex: number;
}) {
  return (
    <box
      flexDirection="column"
      borderStyle="rounded"
      borderColor="cyan"
      paddingX={1}
      paddingY={1}
      width={54}
      height={10}
    >
      <box height={1}>
        <text fg="cyan">Command Palette</text>
      </box>
      {commands.length === 0 ? (
        <box height={1}>
          <text fg="gray">No commands for this view</text>
        </box>
      ) : null}
      {commands.map((command, index) => (
        <box key={command.id} height={1}>
          <text
            fg={
              command.disabled
                ? "gray"
                : index === selectedIndex
                  ? "cyan"
                  : undefined
            }
          >
            {index === selectedIndex ? "> " : "  "}
            {command.label} [{command.key}]{" "}
            {command.disabled ? "(unavailable)" : ""}
          </text>
        </box>
      ))}
      {commands[selectedIndex] ? (
        <box height={1}>
          <text fg="gray">{commands[selectedIndex].hint}</text>
        </box>
      ) : null}
      <box height={1}>
        <text fg="gray">Enter run  Esc close  ↑/↓ or j/k move</text>
      </box>
    </box>
  );
}
