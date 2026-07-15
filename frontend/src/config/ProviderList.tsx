import React, { useRef, useState } from "react";
import { useKeyboard } from "@opentui/react";
import type { ProviderEditor } from "../rpc/schema.js";
import {
  isConfirmKey,
  isDownKey,
  isUpKey,
  type KeyboardInput,
} from "../ui/keyboard.js";
import { theme } from "../ui/theme.js";

type Props = {
  providers: ProviderEditor[];
  active: boolean;
  onOpen: (provider: ProviderEditor) => void;
};

export function ProviderList({ providers, active, onOpen }: Props) {
  const [selected, setSelected] = useState(0);
  const selectedRef = useRef(selected);

  useKeyboard((event) => {
    if (!active) return;
    const key = event as KeyboardInput;
    if (isUpKey(key)) {
      const next = Math.max(0, selectedRef.current - 1);
      selectedRef.current = next;
      setSelected(next);
      return;
    }
    if (isDownKey(key)) {
      const next = Math.min(
        Math.max(providers.length - 1, 0),
        selectedRef.current + 1,
      );
      selectedRef.current = next;
      setSelected(next);
      return;
    }
    if (isConfirmKey(key)) {
      const provider = providers[selectedRef.current];
      if (provider) onOpen(provider);
    }
  });

  return (
    <box flexDirection="column" flexGrow={1}>
      <text fg={theme.accentPrimary}>Provider profiles</text>
      <text fg={theme.accentMuted}>
        Each profile is stored in provider.conf.
      </text>
      <box flexDirection="column" marginTop={1}>
        {providers.map((provider, index) => (
          <box key={provider.id} flexDirection="column" marginBottom={1}>
            <text fg={index === selected ? theme.accentPrimary : undefined}>
              {index === selected ? "> " : "  "}
              {provider.name || provider.id} · {provider.type}
              {provider.key_configured ? " · key set" : " · key missing"}
            </text>
            <text fg={theme.accentMuted}>
              {provider.model || "No default model"}
              {provider.url ? `  ${provider.url}` : ""}
            </text>
          </box>
        ))}
      </box>
      <text fg={theme.accentMuted}>[↑↓] select [Enter] edit [q] quit</text>
    </box>
  );
}
