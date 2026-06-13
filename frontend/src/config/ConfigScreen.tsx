import React, { useEffect, useRef, useState } from "react";
import { useKeyboard, useRenderer } from "@opentui/react";
import {
  ConfigBootstrapSchema,
  type ConfigBootstrap,
  type ConfigFormField,
} from "../rpc/schema.js";
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

type ConfigFormValues = {
  provider: Record<string, string>;
  dreaming: Record<string, string>;
  ingest: Record<string, string>;
  release: Record<string, string>;
};

export function ConfigScreen({ initial, client }: Props) {
  const renderer = useRenderer();
  const [payload, setPayload] = useState(initial);
  const [status, setStatus] = useState<Status>({
    message: "Ready",
    error: false,
  });
  const [busy, setBusy] = useState(false);
  const operationInFlight = useRef(false);

  // Focus and local form state
  const [activePanel, setActivePanel] = useState<"provider" | "form">(
    "provider",
  );
  const [focusedFieldIndex, setFocusedFieldIndex] = useState(0);
  const [isEditing, setIsEditing] = useState(false);

  // Local form state draft to prevent lag on keystrokes
  const [localFormValues, setLocalFormValues] = useState({
    provider: { ...payload.form_values.provider },
    dreaming: { ...payload.form_values.dreaming },
    ingest: { ...payload.form_values.ingest },
    release: { ...payload.form_values.release },
  });

  // Keep local values in sync when payload changes
  useEffect(() => {
    setLocalFormValues({
      provider: { ...payload.form_values.provider },
      dreaming: { ...payload.form_values.dreaming },
      ingest: { ...payload.form_values.ingest },
      release: { ...payload.form_values.release },
    });
  }, [payload.form_values]);

  const providerChoices = payload.provider_choices;
  const selectedProvider = payload.selected_provider;
  const formFields = payload.form_schema.fields;

  const currentProviderIndex = Math.max(
    providerChoices.findIndex((p) => p.name === selectedProvider),
    0,
  );

  const suggestions = modelSuggestions(payload);
  const detailErrors = getDetailErrors(payload);

  useEffect(() => {
    setFocusedFieldIndex((index) =>
      Math.min(index, Math.max(formFields.length - 1, 0)),
    );
  }, [formFields.length]);

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
    setLocalFormValues((prev) => withFieldValue(prev, key, value));
  };

  const submitField = (formValues: ConfigFormValues = localFormValues) => {
    setIsEditing(false);
    if (!client || busy || operationInFlight.current) {
      return;
    }

    void runConfigOperation({
      client,
      method: "config.update_draft",
      params: {
        selected_provider: payload.selected_provider,
        provider: formValues.provider,
        dreaming: formValues.dreaming,
        ingest: formValues.ingest,
        release: formValues.release,
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

  useKeyboard((key) => {
    const ctrl = key.ctrl;
    const tab = key.name === "tab";
    const shift = key.shift;
    const upArrow = key.name === "up";
    const downArrow = key.name === "down";
    const leftArrow = key.name === "left";
    const rightArrow = key.name === "right";
    const enter = key.name === "enter" || key.name === "return";
    const escape = key.name === "escape";

    // 1. Focus Cycling
    if (tab) {
      if (isEditing) {
        submitField();
      }
      setActivePanel((current) =>
        current === "provider" ? "form" : "provider",
      );
      return;
    }

    // 2. Editing toggle handling
    if (isEditing) {
      if (escape) {
        // Discard edits
        setLocalFormValues({
          provider: { ...payload.form_values.provider },
          dreaming: { ...payload.form_values.dreaming },
          ingest: { ...payload.form_values.ingest },
          release: { ...payload.form_values.release },
        });
        setIsEditing(false);
        return;
      }

      const focusedField = formFields[focusedFieldIndex];
      if (focusedField?.type === "toggle" || focusedField?.type === "choice") {
        if (
          leftArrow ||
          rightArrow ||
          key.name === "space" ||
          key.name === " "
        ) {
          const choices = focusedField.choices.length
            ? focusedField.choices
            : ["yes", "no"];
          const currentVal = effectiveValueForField(
            localFormValues,
            focusedField,
          );
          const currentIndex = Math.max(choices.indexOf(currentVal), 0);
          handleFieldChange(
            focusedField.key,
            choices[(currentIndex + 1) % choices.length],
          );
        } else if (enter) {
          const currentVal = effectiveValueForField(
            localFormValues,
            focusedField,
          );
          submitField(
            withFieldValue(localFormValues, focusedField.key, currentVal),
          );
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
        const nextIndex = Math.min(
          providerChoices.length - 1,
          currentProviderIndex + 1,
        );
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
        setFocusedFieldIndex((prev) =>
          Math.min(Math.max(formFields.length - 1, 0), prev + 1),
        );
        return;
      }
      if (enter && formFields.length > 0) {
        setIsEditing(true);
        return;
      }
    }

    // 4. Global hotkeys
    if (key.name === "q") {
      client?.close();
      renderer.destroy();
    }

    if (key.name === "s") {
      handleSave();
      return;
    }

    if (key.name === "r") {
      handleReload();
      return;
    }

    if (key.name === "c") {
      handleCheck();
      return;
    }

    // Numeric provider selection shortcuts
    const providerIndex = providerIndexForInput(
      key.name,
      providerChoices.length,
    );
    if (providerIndex >= 0) {
      selectProviderByIndex(providerIndex);
    }
  });

  return (
    <box flexDirection="column" width={100}>
      <text>Hieronymus Config</text>
      <text fg="gray">
        {[
          payload.config_paths.dream_config_path,
          payload.config_paths.ingest_config_path,
          payload.config_paths.release_config_path,
        ]
          .filter(Boolean)
          .join(" | ")}
      </text>

      <box flexDirection="row" marginTop={1}>
        {/* Left Column: Provider Selector */}
        <box
          flexDirection="column"
          width={28}
          borderStyle="rounded"
          borderColor={activePanel === "provider" ? "cyan" : "gray"}
          paddingX={1}
        >
          <text fg={activePanel === "provider" ? "cyan" : undefined}>
            Providers
          </text>
          <ProviderSelector
            choices={providerChoices}
            selected={selectedProvider}
            focused={activePanel === "provider"}
            onSelect={selectProviderByIndex}
          />
        </box>

        {/* Right Column: Configuration Form */}
        <box
          flexDirection="column"
          width={70}
          borderStyle="rounded"
          borderColor={activePanel === "form" ? "cyan" : "gray"}
          paddingX={1}
        >
          <ConfigForm
            fields={formFields}
            formValues={localFormValues}
            focusedFieldIndex={focusedFieldIndex}
            isEditing={isEditing}
            focused={activePanel === "form"}
            onFieldChange={handleFieldChange}
            onSubmitField={submitField}
          />
        </box>
      </box>

      <box marginTop={1} flexDirection="column">
        <text>
          Models: {suggestions.length > 0 ? suggestions.join(", ") : "-"}
        </text>
        {payload.validation.errors.map((error) => (
          <text key={error} fg="red">
            {error}
          </text>
        ))}
        {detailErrors.map((error) => (
          <text key={error} fg="red">
            {error}
          </text>
        ))}
      </box>

      <StatusLine message={status.message} error={status.error} busy={busy} />
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
    </box>
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

function valueForField(values: ConfigFormValues, key: string): string {
  if (key.startsWith("provider.")) {
    return values.provider[key.slice(9)] || "";
  }
  if (key.startsWith("dreaming.")) {
    return values.dreaming[key.slice(9)] || "";
  }
  if (key.startsWith("ingest.")) {
    return values.ingest[key.slice(7)] || "";
  }
  if (key.startsWith("release.")) {
    return values.release[key.slice(8)] || "";
  }
  return "";
}

function effectiveValueForField(
  values: ConfigFormValues,
  field: ConfigFormField,
): string {
  const value = valueForField(values, field.key);
  if ((field.type === "toggle" || field.type === "choice") && !value) {
    return field.default || field.choices[0] || "";
  }
  return value;
}

function withFieldValue(
  values: ConfigFormValues,
  key: string,
  value: string,
): ConfigFormValues {
  const providerDraft = { ...values.provider };
  const dreamingDraft = { ...values.dreaming };
  const ingestDraft = { ...values.ingest };
  const releaseDraft = { ...values.release };

  if (key.startsWith("provider.")) {
    providerDraft[key.slice(9)] = value;
  } else if (key.startsWith("dreaming.")) {
    dreamingDraft[key.slice(9)] = value;
  } else if (key.startsWith("ingest.")) {
    ingestDraft[key.slice(7)] = value;
  } else if (key.startsWith("release.")) {
    releaseDraft[key.slice(8)] = value;
  }

  return {
    provider: providerDraft,
    dreaming: dreamingDraft,
    ingest: ingestDraft,
    release: releaseDraft,
  };
}

function providerIndexForInput(input: string, providerCount: number) {
  const index = providerKeys(providerCount).indexOf(input);
  return index >= 0 ? index : -1;
}
