import React, { useState } from "react";
import { Box, Text, useInput } from "ink";
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
  const suggestions = modelSuggestions(payload);
  const detailErrors = getDetailErrors(payload);

  return (
    <Box flexDirection="column">
      {client ? (
        <ConfigInputHandler
          client={client}
          payload={payload}
          setPayload={setPayload}
          setStatus={setStatus}
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
      {suggestions.length > 0 ? (
        <Text>Models: {suggestions.join(", ")}</Text>
      ) : null}
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
      <StatusLine message={status.message} error={status.error} />
      <KeyHelp keys={["1/2/3 provider", "s save", "r reload", "c check"]} />
    </Box>
  );
}

function ConfigInputHandler({
  client,
  payload,
  setPayload,
  setStatus,
}: {
  client: JsonRpcClient;
  payload: ConfigBootstrap;
  setPayload: (payload: ConfigBootstrap) => void;
  setStatus: (status: Status) => void;
}) {
  useInput((input) => {
    const providerIndex = providerKeys.indexOf(
      input as (typeof providerKeys)[number],
    );
    if (providerIndex >= 0) {
      const provider = payload.provider_choices[providerIndex]?.name;
      if (provider) {
        void requestBootstrap(client, "config.select_provider", {
          provider,
          draft: payload.draft,
        }).then((next) => {
          setPayload(next);
          setStatus({ message: `Selected ${provider}`, error: false });
        }, setErrorStatus(setStatus));
      }
      return;
    }

    if (input === "s") {
      void requestBootstrap(client, "config.save", {
        draft: payload.draft,
      }).then((next) => {
        setPayload(next);
        setStatus({ message: "Saved configuration", error: false });
      }, setErrorStatus(setStatus));
      return;
    }

    if (input === "r") {
      void requestBootstrap(client, "config.reload", {
        selected_provider: payload.selected_provider,
      }).then((next) => {
        setPayload(next);
        setStatus({ message: "Reloaded configuration", error: false });
      }, setErrorStatus(setStatus));
      return;
    }

    if (input === "c") {
      void requestBootstrap(client, "config.check_provider", {
        selected_provider: payload.selected_provider,
        draft: payload.draft,
      }).then((next) => {
        setPayload(next);
        setStatus({ message: "Provider check complete", error: false });
      }, setErrorStatus(setStatus));
    }
  });
  return null;
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
