import React, { useRef, useState } from "react";
import { useKeyboard } from "@opentui/react";
import type { ProviderEditor } from "../rpc/schema.js";
import {
  isConfirmKey,
  isDownKey,
  isEscapeKey,
  isUpKey,
  printableSearchChar,
  type KeyboardInput,
} from "../ui/keyboard.js";
import { theme } from "../ui/theme.js";

export type ProviderPatch = {
  id: string;
  name: string;
  type: string;
  url: string;
  key: string;
  model: string;
  timeout_seconds: string;
};

type Props = {
  provider: ProviderEditor;
  onCancel: () => void;
  onSave: (provider: ProviderPatch) => void;
};

const FIELD_LABELS = ["Name", "Type", "API key", "URL", "Model", "Timeout"];
const PROVIDER_TYPES = ["openai", "google", "anthropic", "ollama"];

type ModalState = {
  draft: ProviderPatch;
  selected: number;
  editing: boolean;
  status: string;
};

export function ProviderModal({ provider, onCancel, onSave }: Props) {
  const [state, setState] = useState<ModalState>(() => ({
    draft: {
      id: provider.id,
      name: provider.name,
      type: provider.type,
      url: provider.url,
      key: "",
      model: provider.model,
      timeout_seconds: String(provider.timeout_seconds),
    },
    selected: 0,
    editing: false,
    status: "",
  }));
  const stateRef = useRef(state);

  const transition = (action: ModalAction) => {
    const next = reduceModal(stateRef.current, action);
    stateRef.current = next;
    setState(next);
    if (action.type === "save") {
      onSave(next.draft);
    }
    if (action.type === "cancel") {
      onCancel();
    }
  };

  useKeyboard((event) => {
    const key = event as KeyboardInput;
    if (isEscapeKey(key)) {
      transition({ type: "cancel" });
      return;
    }
    if (!stateRef.current.editing && key.name === "s") {
      transition({ type: "save" });
      return;
    }
    if (!stateRef.current.editing && isUpKey(key)) {
      transition({ type: "move", delta: -1 });
      return;
    }
    if (!stateRef.current.editing && isDownKey(key)) {
      transition({ type: "move", delta: 1 });
      return;
    }
    if (isConfirmKey(key)) {
      transition({ type: "confirm" });
      return;
    }
    if (stateRef.current.editing && key.name === "backspace") {
      transition({ type: "backspace" });
      return;
    }
    if (stateRef.current.editing) {
      const char = printableSearchChar(key);
      if (char !== null) transition({ type: "append", char });
    }
  });

  return (
    <box
      position="absolute"
      top={1}
      left="10%"
      width="80%"
      flexDirection="column"
      borderStyle="double"
      borderColor={theme.accentPrimary}
      backgroundColor="black"
      padding={1}
    >
      <text fg={theme.accentPrimary}>Edit provider: {provider.id}</text>
      <text fg={theme.accentMuted}>Changes affect provider.conf only.</text>
      <box flexDirection="column" marginTop={1}>
        {FIELD_LABELS.map((label, index) => (
          <text
            key={label}
            fg={index === state.selected ? theme.accentPrimary : undefined}
          >
            {index === state.selected ? "> " : "  "}
            {label}: {fieldValue(state.draft, index)}
            {index === state.selected && state.editing ? "▏" : ""}
          </text>
        ))}
      </box>
      {state.status ? (
        <text fg={theme.statusSuccess}>{state.status}</text>
      ) : null}
      <text fg={theme.accentMuted}>
        {state.editing
          ? "[Enter] done  [Esc] cancel"
          : "[↑↓] field  [Enter] edit  [s] save  [Esc] close"}
      </text>
    </box>
  );
}

type ModalAction =
  | { type: "move"; delta: number }
  | { type: "confirm" }
  | { type: "append"; char: string }
  | { type: "backspace" }
  | { type: "save" }
  | { type: "cancel" };

function reduceModal(state: ModalState, action: ModalAction): ModalState {
  if (action.type === "move" && !state.editing) {
    return {
      ...state,
      selected: Math.max(
        0,
        Math.min(FIELD_LABELS.length - 1, state.selected + action.delta),
      ),
    };
  }
  if (action.type === "confirm") {
    return { ...state, editing: !state.editing };
  }
  if (action.type === "save") {
    return { ...state, status: `Saved ${state.draft.name || state.draft.id}` };
  }
  if (action.type === "append" && state.editing) {
    return {
      ...state,
      draft: updateField(
        state.draft,
        state.selected,
        fieldValue(state.draft, state.selected) + action.char,
      ),
    };
  }
  if (action.type === "backspace" && state.editing) {
    const value = fieldValue(state.draft, state.selected);
    return {
      ...state,
      draft: updateField(state.draft, state.selected, value.slice(0, -1)),
    };
  }
  return state;
}

function fieldValue(draft: ProviderPatch, index: number): string {
  return (
    [
      draft.name,
      draft.type,
      draft.key,
      draft.url,
      draft.model,
      draft.timeout_seconds,
    ][index] ?? ""
  );
}

function updateField(
  draft: ProviderPatch,
  index: number,
  value: string,
): ProviderPatch {
  const keys: Array<keyof ProviderPatch> = [
    "name",
    "type",
    "key",
    "url",
    "model",
    "timeout_seconds",
  ];
  const key = keys[index];
  return key ? { ...draft, [key]: value } : draft;
}
