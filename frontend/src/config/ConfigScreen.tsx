import React, { useEffect, useRef, useState } from "react";
import { Box, Text, useStdin } from "ink";
import { ConfigBootstrapSchema, type ConfigBootstrap } from "../rpc/schema.js";
import type { JsonRpcClient } from "../rpc/client.js";
import { ConfigForm } from "./ConfigForm.js";
import { ProviderSelector } from "./ProviderSelector.js";
import { KeyHelp } from "../ui/KeyHelp.js";
import { StatusLine } from "../ui/StatusLine.js";

type Props = {
  initial: ConfigBootstrap;
  client: JsonRpcClient | undefined;
};

type Status = {
  message: string;
  error: boolean;
};

const providerKeys = ["1", "2", "3"] as const;

export function ConfigScreen({ initial, client }: Props) {
  const [payload, setPayload] = useState(initial);
  const [status, setStatus] = useState<Status>({
    message: "Ready",
    error: false,
  });
  const [busy, setBusy] = useState(false);
  const operationInFlight = useRef(false);
  const suggestions = modelSuggestions(payload);
  const detailErrors = getDetailErrors(payload);

  return (
    <Box flexDirection="column">
      {client ? (
        <ConfigInputHandler
          client={client}
          payload={payload}
          busy={busy}
          setPayload={setPayload}
          setStatus={setStatus}
          setBusy={setBusy}
          operationInFlight={operationInFlight}
        />
      ) : null}
      <Text bold>Hieronymus Config</Text>
      <Text dimColor>{payload.config_paths.settings_path}</Text>
      <Box marginTop={1}>
        <ProviderSelector
          choices={payload.provider_choices}
          selected={payload.selected_provider}
        />
        <ConfigForm payload={payload} />
      </Box>
      <Text>
        Models: {suggestions.length > 0 ? suggestions.join(", ") : "-"}
      </Text>
      {payload.validation.errors.map((error) => (
        <Text key={error} color="red">
          {error}
        </Text>
      ))}
      {detailErrors.map((error) => (
        <Text key={error} color="red">
          {error}
        </Text>
      ))}
      <StatusLine
        message={busy ? `Working: ${status.message}` : status.message}
        error={status.error}
      />
      <KeyHelp
        keys={["1/2/3 provider", "s save", "r reload", "c check", "q quit"]}
      />
    </Box>
  );
}

function ConfigInputHandler({
  client,
  payload,
  busy,
  setPayload,
  setStatus,
  setBusy,
  operationInFlight,
}: {
  client: JsonRpcClient;
  payload: ConfigBootstrap;
  busy: boolean;
  setPayload: (payload: ConfigBootstrap) => void;
  setStatus: (status: Status) => void;
  setBusy: (busy: boolean) => void;
  operationInFlight: React.MutableRefObject<boolean>;
}) {
  const { stdin, setRawMode, isRawModeSupported } = useStdin();

  useEffect(() => {
    const canUseInkRawMode =
      isRawModeSupported &&
      typeof stdin.ref === "function" &&
      typeof stdin.unref === "function";
    if (!canUseInkRawMode) {
      return undefined;
    }

    setRawMode(true);
    return () => {
      setRawMode(false);
    };
  }, [isRawModeSupported, setRawMode, stdin]);

  useEffect(() => {
    const onData = (chunk: Buffer | string) => {
      const input = String(chunk)[0] ?? "";
      if (busy || operationInFlight.current) {
        return;
      }

      const providerIndex = providerKeys.indexOf(
        input as (typeof providerKeys)[number],
      );
      if (providerIndex >= 0) {
        const provider = payload.provider_choices[providerIndex]?.name;
        if (provider) {
          void runConfigOperation({
            client,
            method: "config.select_provider",
            params: {
              provider,
              draft: payload.draft,
            },
            pendingMessage: `Selecting ${provider}`,
            successMessage: `Selected ${provider}`,
            setBusy,
            setPayload,
            setStatus,
            operationInFlight,
          });
        }
        return;
      }

      if (input === "s") {
        void runConfigOperation({
          client,
          method: "config.save",
          params: { draft: payload.draft },
          pendingMessage: "Saving configuration",
          successMessage: "Saved configuration",
          setBusy,
          setPayload,
          setStatus,
          operationInFlight,
        });
        return;
      }

      if (input === "r") {
        void runConfigOperation({
          client,
          method: "config.reload",
          params: {
            selected_provider: payload.selected_provider,
          },
          pendingMessage: "Reloading configuration",
          successMessage: "Reloaded configuration",
          setBusy,
          setPayload,
          setStatus,
          operationInFlight,
        });
        return;
      }

      if (input === "c") {
        void runConfigOperation({
          client,
          method: "config.check_provider",
          params: {
            selected_provider: payload.selected_provider,
            draft: payload.draft,
          },
          pendingMessage: "Checking provider",
          successMessage: "Provider check complete",
          setBusy,
          setPayload,
          setStatus,
          operationInFlight,
        });
      }
    };

    stdin.on("data", onData);
    return () => {
      stdin.off("data", onData);
    };
  }, [
    busy,
    client,
    operationInFlight,
    payload,
    setBusy,
    setPayload,
    setStatus,
    stdin,
  ]);

  return null;
}

async function runConfigOperation({
  client,
  method,
  params,
  pendingMessage,
  successMessage,
  setBusy,
  setPayload,
  setStatus,
  operationInFlight,
}: {
  client: JsonRpcClient;
  method: string;
  params: Record<string, unknown>;
  pendingMessage: string;
  successMessage: string;
  setBusy: (busy: boolean) => void;
  setPayload: (payload: ConfigBootstrap) => void;
  setStatus: (status: Status) => void;
  operationInFlight: React.MutableRefObject<boolean>;
}) {
  operationInFlight.current = true;
  setBusy(true);
  setStatus({ message: pendingMessage, error: false });
  try {
    const next = await requestBootstrap(client, method, params);
    setPayload(next);
    setStatus({ message: successMessage, error: false });
  } catch (error) {
    setErrorStatus(setStatus)(error);
  } finally {
    operationInFlight.current = false;
    setBusy(false);
  }
}

function requestBootstrap(
  client: JsonRpcClient,
  method: string,
  params: Record<string, unknown>,
): Promise<ConfigBootstrap> {
  return client
    .request(method, params)
    .then((response) => ConfigBootstrapSchema.parse(response));
}

function setErrorStatus(setStatus: (status: Status) => void) {
  return (error: unknown) => {
    const message = error instanceof Error ? error.message : String(error);
    setStatus({ message, error: true });
  };
}

function modelSuggestions(payload: ConfigBootstrap): string[] {
  const { suggestions } = payload;
  if (typeof suggestions === "object" && "models" in suggestions) {
    return suggestions.models;
  }
  return [];
}

function getDetailErrors(payload: ConfigBootstrap): string[] {
  const { detail } = payload;
  if (typeof detail === "object" && "errors" in detail) {
    return detail.errors;
  }
  if (typeof detail === "string" && detail) {
    return [detail];
  }
  return [];
}
