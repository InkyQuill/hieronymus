import React, { useRef, useState } from "react";
import {
  useKeyboard,
  useRenderer,
  useTerminalDimensions,
} from "@opentui/react";
import {
  ProviderDetailSchema,
  type ProviderEditor,
  type ProviderList,
} from "../rpc/schema.js";
import type { RpcClient } from "../rpc/client.js";
import { ProviderList } from "./ProviderList.js";
import { ProviderModal, type ProviderPatch } from "./ProviderModal.js";
import { KeyHelp } from "../ui/KeyHelp.js";
import { StatusLine } from "../ui/StatusLine.js";
import { theme } from "../ui/theme.js";
import { MIN_TERMINAL_HEIGHT, MIN_TERMINAL_WIDTH } from "../ui/responsive.js";
import { isEscapeKey, type KeyboardInput } from "../ui/keyboard.js";

type Props = {
  initial: ProviderList;
  client: RpcClient | undefined;
};

type ScreenState = {
  providers: ProviderEditor[];
  openProvider: ProviderEditor | null;
  busy: boolean;
  status: string;
  error: boolean;
};

export function ConfigScreen({ initial, client }: Props) {
  const renderer = useRenderer();
  const dimensions = useTerminalDimensions();
  const [state, setState] = useState<ScreenState>({
    providers: initial.providers,
    openProvider: null,
    busy: false,
    status: initial.error || "Choose a provider to edit.",
    error: Boolean(initial.error),
  });
  const stateRef = useRef(state);

  const replaceState = (next: ScreenState) => {
    stateRef.current = next;
    setState(next);
  };

  const openProvider = (provider: ProviderEditor) => {
    if (!client || stateRef.current.busy) return;
    replaceState({
      ...stateRef.current,
      busy: true,
      status: `Loading ${provider.name}`,
      error: false,
    });
    void client
      .request("config.provider_detail", { provider_id: provider.id })
      .then((payload) => ProviderDetailSchema.parse(payload))
      .then((detail) => {
        if (detail.error) throw new Error(detail.error);
        replaceState({
          ...stateRef.current,
          openProvider: detail.provider,
          busy: false,
          status: "",
          error: false,
        });
      })
      .catch((error: unknown) => {
        replaceState({
          ...stateRef.current,
          busy: false,
          status: errorMessage(error),
          error: true,
        });
      });
  };

  const closeModal = () => {
    replaceState({
      ...stateRef.current,
      openProvider: null,
      status: "Choose a provider to edit.",
      error: false,
    });
  };

  const saveProvider = (provider: ProviderPatch) => {
    if (!client || stateRef.current.busy) return;
    replaceState({
      ...stateRef.current,
      busy: true,
      status: `Saving ${provider.name}`,
      error: false,
    });
    void client
      .request("config.save_provider", { provider })
      .then((payload) => ProviderDetailSchema.parse(payload))
      .then((detail) => {
        if (detail.error) throw new Error(detail.error);
        const providers = stateRef.current.providers.map((item) =>
          item.id === detail.provider.id ? detail.provider : item,
        );
        replaceState({
          providers,
          openProvider: null,
          busy: false,
          status: "Provider saved to provider.conf",
          error: false,
        });
      })
      .catch((error: unknown) => {
        replaceState({
          ...stateRef.current,
          busy: false,
          status: errorMessage(error),
          error: true,
        });
      });
  };

  useKeyboard((event) => {
    const key = event as KeyboardInput;
    if (
      !stateRef.current.openProvider &&
      (isEscapeKey(key) || key.name === "q")
    ) {
      client?.close();
      renderer.destroy();
    }
  });

  if (
    dimensions.width < MIN_TERMINAL_WIDTH ||
    dimensions.height < MIN_TERMINAL_HEIGHT
  ) {
    return (
      <text>
        Terminal too small — resize to at least {MIN_TERMINAL_WIDTH}×
        {MIN_TERMINAL_HEIGHT}.
      </text>
    );
  }

  return (
    <box flexDirection="column" width="100%" height="100%" padding={1}>
      <text fg={theme.accentPrimary}>Hieronymus Config</text>
      <box flexGrow={1} marginTop={1}>
        <ProviderList
          providers={state.providers}
          active={!state.openProvider}
          onOpen={openProvider}
        />
      </box>
      <StatusLine
        message={state.status || (state.busy ? "Working…" : "Ready")}
        error={state.error}
      />
      <KeyHelp keys={["↑↓ select", "Enter edit", "q quit"]} />
      {state.openProvider ? (
        <ProviderModal
          provider={state.openProvider}
          onCancel={closeModal}
          onSave={saveProvider}
        />
      ) : null}
    </box>
  );
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
