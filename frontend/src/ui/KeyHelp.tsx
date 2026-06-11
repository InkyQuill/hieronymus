import React from "react";

/**
 * Renders a horizontal key binding strip.
 *
 * Each entry should be in "key label" format, e.g. "Tab focus" or "+/- reinforce/decay".
 * The first word is treated as the key(s) and rendered as a bracketed badge;
 * the remainder is the plain description.
 *
 * Example input:  ["Tab focus", "q quit", "1-9 view"]
 * Example output: [Tab] focus  [q] quit  [1-9] view
 */
export function KeyHelp({ keys }: { keys: string[] }) {
  const items = keys.map((entry) => {
    const spaceIdx = entry.indexOf(" ");
    const key = spaceIdx >= 0 ? entry.slice(0, spaceIdx) : entry;
    const label = spaceIdx >= 0 ? " " + entry.slice(spaceIdx + 1) : "";
    return `[${key}]${label}`;
  });

  return (
    <box flexDirection="row" marginTop={1}>
      {items.map((item, i) => (
        <text key={String(i)} fg="gray">
          {item}
          {"  "}
        </text>
      ))}
    </box>
  );
}
