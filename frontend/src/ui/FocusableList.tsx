import React from "react";

type Props<T> = {
  items: T[];
  selectedIndex: number;
  label: (item: T) => string;
  focused?: boolean;
};

export function FocusableList<T>({
  items,
  selectedIndex,
  label,
  focused = true,
}: Props<T>) {
  return (
    <box flexDirection="column">
      {items.map((item, index) => (
        <text
          key={`${index}-${label(item)}`}
          fg={
            index === selectedIndex ? (focused ? "cyan" : "white") : undefined
          }
        >
          {index === selectedIndex ? ">" : " "} {label(item)}
        </text>
      ))}
    </box>
  );
}
