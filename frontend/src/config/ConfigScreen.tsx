import React, { useEffect, useRef, useState } from "react";
import { Box, Text, useApp, useStdin, useInput } from "ink";
import { ConfigBootstrapSchema, type ConfigBootstrap } from "../rpc/schema.js";
import type { RpcClient } from "../rpc/client.js";
import { ConfigForm } from "./ConfigForm.js";
import { ProviderSelector } from "./ProviderSelector.js";
import { KeyHelp } from "../ui/KeyHelp.js";
import { StatusLine } from "../ui/StatusLine.js";

type Props = {
  initial: ConfigBootstrap;
  client: RpcClient | undefined;
};

type Status = {
  message: string;
  error: boolean;
};

export function ConfigScreen({ initial, client }: Props) {
  const { exit } = useApp();
  const { stdin, isRawModeSupported } = useStdin();
  const [payload, setPayload] = useState(initial);
  const [status, setStatus] = useState<Status>({
    message: "Ready",
    error: false,
  });
  const [busy, setBusy] = useState(false);
  const operationInFlight = useRef(false);

  // Focus and local form state
  const [activePanel, setActivePanel] = useState<"provider" | "form">("provider");
  const [focusedFieldIndex, setFocusedFieldIndex] = useState(0);
  const [isEditing, setIsEditing] = useState(false);

  // Local form state draft to prevent lag on keystrokes
  const [localFormValues, setLocalFormValues] = useState({
    provider: { ...payload.form_values.provider },
    dreaming: { ...payload.form_values.dreaming },
  });

  // Keep local values in sync when payload changes
  useEffect(() => {
    setLocalFormValues({
      provider: { ...payload.form_values.provider },
      dreaming: { ...payload.form_values.dreaming },
    });
  }, [payload.form_values]);

  const canUseInkInput = Boolean(
    isRawModeSupported &&
      typeof stdin.ref === "function" &&
      typeof stdin.unref === "function",
  );

  const providerChoices = payload.provider_choices;
  const selectedProvider = payload.selected_provider;

  const currentProviderIndex = Math.max(
    providerChoices.findIndex((p) => p.name === selectedProvider),
    0,
  );

  const suggestions = modelSuggestions(payload);
  const detailErrors = getDetailErrors(payload);

  const selectProviderByIndex = (index: number) => {
    const provider = providerChoices[index]?.name;
    if (provider && client && !busy && !operationInFlight.current) {
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
  };

  const handleFieldChange = (key: string, value: string) => {
    setLocalFormValues((prev) => {
      const providerDraft = { ...prev.provider };
      const dreamingDraft = { ...prev.dreaming };

      if (key.startsWith("provider.")) {
        providerDraft[key.slice(9)] = value;
      } else if (key.startsWith("dreaming.")) {
        dreamingDraft[key.slice(9)] = value;
      }

      return { provider: providerDraft, dreaming: dreamingDraft };
    });
  };

  const submitField = () => {
    setIsEditing(false);
    if (!client || busy || operationInFlight.current) {
      return;
    }

    void runConfigOperation({
      client,
      method: "config.update_draft",
      params: {
        selected_provider: payload.selected_provider,
        provider: localFormValues.provider,
        dreaming: localFormValues.dreaming,
      },
      pendingMessage: "Updating draft settings",
      successMessage: "Draft settings updated",
      setBusy,
      setPayload,
      setStatus,
      operationInFlight,
    });
  };

  const handleSave = () => {
    if (!client || busy || operationInFlight.current) return;
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
  };

  const handleReload = () => {
    if (!client || busy || operationInFlight.current) return;
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
  };

  const handleCheck = () => {
    if (!client || busy || operationInFlight.current) return;
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
  };

  const handleInput = (input: string, key?: any) => {
    const ctrl = key ? key.ctrl : false;
    const tab = key ? key.tab : (input === "\t");
    const shift = key ? key.shift : false;
    const upArrow = key ? key.upArrow : false;
    const downArrow = key ? key.downArrow : false;
    const leftArrow = key ? key.leftArrow : false;
    const rightArrow = key ? key.rightArrow : false;
    const enter = key ? key.return : (input === "\r" || input === "\n");
    const escape = key ? key.escape : false;

    // 1. Focus Cycling
    if (tab) {
      if (isEditing) {
        submitField();
      }
      setActivePanel((current) => (current === "provider" ? "form" : "provider"));
      return;
    }

    // 2. Editing toggle handling
    if (isEditing) {
      if (escape) {
        // Discard edits
        setLocalFormValues({
          provider: { ...payload.form_values.provider },
          dreaming: { ...payload.form_values.dreaming },
        });
        setIsEditing(false);
        return;
      }

      if (focusedFieldIndex === 4) {
        // Autostart toggle key logic (Left/Right arrow or Space)
        if (leftArrow || rightArrow || input === " ") {
          const currentVal = localFormValues.dreaming.autostart_enabled || "no";
          handleFieldChange("dreaming.autostart_enabled", currentVal === "yes" ? "no" : "yes");
        } else if (enter) {
          submitField();
        }
      }
      return;
    }

    // 3. Panel navigation
    if (activePanel === "provider") {
      if (upArrow) {
        const nextIndex = Math.max(0, currentProviderIndex - 1);
        if (nextIndex !== currentProviderIndex) {
          selectProviderByIndex(nextIndex);
        }
        return;
      }
      if (downArrow) {
        const nextIndex = Math.min(providerChoices.length - 1, currentProviderIndex + 1);
        if (nextIndex !== currentProviderIndex) {
          selectProviderByIndex(nextIndex);
        }
        return;
      }
    }

    if (activePanel === "form") {
      if (upArrow) {
        setFocusedFieldIndex((prev) => Math.max(0, prev - 1));
        return;
      }
      if (downArrow) {
        setFocusedFieldIndex((prev) => Math.min(7, prev + 1));
        return;
      }
      if (enter) {
        setIsEditing(true);
        return;
      }
    }

    // 4. Global hotkeys
    if (input === "q") {
      client?.close();
      exit();
      return;
    }

    if (input === "s") {
      handleSave();
      return;
    }

    if (input === "r") {
      handleReload();
      return;
    }

    if (input === "c") {
      handleCheck();
      return;
    }

    // Numeric provider selection shortcuts
    const providerIndex = providerIndexForInput(input, providerChoices.length);
    if (providerIndex >= 0) {
      selectProviderByIndex(providerIndex);
    }
  };

  useInput(
    (input, key) => {
      handleInput(input, key);
    },
    { isActive: canUseInkInput },
  );

  useEffect(() => {
    if (canUseInkInput) {
      return undefined;
    }

    const onData = (chunk: Buffer | string) => {
      const text = String(chunk);
      handleInput(text[0] ?? "");
    };

    stdin.on("data", onData);
    return () => {
      stdin.off("data", onData);
    };
  }, [canUseInkInput, stdin, activePanel, isEditing, focusedFieldIndex, localFormValues, payload]);

  return (
    <Box flexDirection="column" width={100}>
      <Text bold>Hieronymus Config</Text>
      <Text dimColor>{payload.config_paths.settings_path}</Text>

      <Box flexDirection="row" marginTop={1}>
        {/* Left Column: Provider Selector */}
        <Box
          flexDirection="column"
          width={28}
          borderStyle="round"
          borderColor={activePanel === "provider" ? "cyan" : "gray"}
          paddingX={1}
        >
          <Text bold color={activePanel === "provider" ? "cyan" : undefined}>
            Providers
          </Text>
          <ProviderSelector
            choices={providerChoices}
            selected={selectedProvider}
            focused={activePanel === "provider"}
          />
        </Box>

        {/* Right Column: Configuration Form */}
        <Box
          flexDirection="column"
          width={70}
          borderStyle="round"
          borderColor={activePanel === "form" ? "cyan" : "gray"}
          paddingX={1}
        >
          <ConfigForm
            formValues={localFormValues}
            focusedFieldIndex={focusedFieldIndex}
            isEditing={isEditing}
            focused={activePanel === "form"}
            onFieldChange={handleFieldChange}
            onSubmitField={submitField}
          />
        </Box>
      </Box>

      <Box marginTop={1} flexDirection="column">
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
      </Box>

      <StatusLine
        message={busy ? `Working: ${status.message}` : status.message}
        error={status.error}
      />
      <KeyHelp
        keys={[
          "Tab focus",
          `${providerKeyRange(providerChoices)} provider`,
          "s save",
          "r reload",
          "c check",
          "q quit",
        ]}
      />
    </Box>
  );
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
  client: RpcClient;
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
    const message = error instanceof Error ? error.message : String(error);
    setStatus({ message, error: true });
  } finally {
    operationInFlight.current = false;
    setBusy(false);
  }
}

function requestBootstrap(
  client: RpcClient,
  method: string,
  params: Record<string, unknown>,
): Promise<ConfigBootstrap> {
  return client
    .request(method, params)
    .then((response) => ConfigBootstrapSchema.parse(response));
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

function providerKeys(providerCount: number): string[] {
  return Array.from({ length: Math.min(providerCount, 9) }, (_, index) =>
    String(index + 1),
  );
}

function providerKeyRange(
  providerChoices: ConfigBootstrap["provider_choices"],
) {
  const keys = providerKeys(providerChoices.length);
  if (keys.length === 0) {
    return "-";
  }
  if (keys.length === 1) {
    return keys[0];
  }
  return `${keys[0]}-${keys[keys.length - 1]}`;
}

function providerIndexForInput(input: string, providerCount: number) {
  const index = providerKeys(providerCount).indexOf(input);
  return index >= 0 ? index : -1;
}
