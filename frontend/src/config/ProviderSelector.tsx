import React from "react";
import type { ConfigBootstrap, ProviderName } from "../rpc/schema.js";

type Props = {
  choices: ConfigBootstrap["provider_choices"];
  selected: ProviderName;
  focused?: boolean;
  onSelect?: (index: number) => void;
};

export function ProviderSelector({
  choices,
  selected,
  focused = true,
  onSelect,
}: Props) {
  const selectedIndex = Math.max(
    choices.findIndex((choice) => choice.name === selected),
    0,
  );

  return (
    <select
      width={24}
      height={Math.max(choices.length, 1)}
      focused={focused}
      selectedIndex={selectedIndex}
      showDescription={false}
      selectedTextColor={focused ? "cyan" : "white"}
      selectedBackgroundColor="transparent"
      focusedBackgroundColor="transparent"
      options={choices.map((choice) => ({
        name: choice.display_name,
        description: choice.name,
        value: choice.name,
      }))}
      onSelect={(index) => onSelect?.(index)}
    />
  );
}
