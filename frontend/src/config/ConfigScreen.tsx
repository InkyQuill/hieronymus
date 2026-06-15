import React, { useEffect, useRef, useState } from "react";
import {
  useKeyboard,
  useRenderer,
  useTerminalDimensions,
} from "@opentui/react";
import {
  ConfigBootstrapSchema,
  type ConfigBootstrap,
  type ConfigFormField,
  type ConfigFormSection,
} from "../rpc/schema.js";
import type { RpcClient } from "../rpc/client.js";
import { ConfigForm } from "./ConfigForm.js";
import { KeyHelp } from "../ui/KeyHelp.js";
import { StatusLine } from "../ui/StatusLine.js";
import {
  classifyTerminalLayout,
  MIN_TERMINAL_HEIGHT,
  MIN_TERMINAL_WIDTH,
  panelHeight,
  panelWidth,
} from "../ui/responsive.js";
import {
  isConfirmKey,
  isDownKey,
  isEscapeKey,
  isLeftKey,
  isRightKey,
  isUpKey,
  printableSearchChar,
  type KeyboardInput,
} from "../ui/keyboard.js";

const SYNTHETIC_PROVIDER_KEY = "provider.__selected";

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
  const dimensions = useTerminalDimensions();
  const layout = classifyTerminalLayout(dimensions.width, dimensions.height);
  const contentWidth = panelWidth(layout);
  const [payload, setPayload] = useState(initial);
  const [status, setStatus] = useState<Status>({
    message: "Ready",
    error: false,
  });
  const [busy, setBusy] = useState(false);
  const operationInFlight = useRef(false);

  const [focusedFieldIndex, setFocusedFieldIndex] = useState(0);
  const [isEditing, setIsEditing] = useState(false);
  const [searchActive, setSearchActive] = useState(false);
  const [searchText, setSearchText] = useState("");

  // Local form state draft to prevent lag on keystrokes
  const [localFormValues, setLocalFormValues] = useState(() =>
    formValuesWithSelectedProvider(payload.form_values, payload),
  );

  // Keep local values in sync when payload changes
  useEffect(() => {
    setLocalFormValues(
      formValuesWithSelectedProvider(payload.form_values, payload),
    );
  }, [payload]);

  const providerChoices = payload.provider_choices;
  const formFields = configFieldsWithProviderChoice(payload);

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
    const draftValues = draftFormValues(formValues);

    void runConfigOperation({
      client,
      method: "config.update_draft",
      params: {
        selected_provider: payload.selected_provider,
        draft: payload.draft,
        provider: draftValues.provider,
        dreaming: draftValues.dreaming,
        ingest: draftValues.ingest,
        release: draftValues.release,
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

  const cancelSearch = () => {
    setSearchActive(false);
    setSearchText("");
  };

  const submitSearch = () => {
    const query = searchText.trim();
    if (!query) {
      setStatus({ message: "Search query is empty", error: true });
      return;
    }

    const matchIndex = findConfigFieldMatch(
      query,
      formFields,
      payload.form_schema.groups,
      payload.form_schema.sections,
    );
    if (matchIndex < 0) {
      setStatus({ message: `No config field matches "${query}"`, error: true });
      return;
    }

    setFocusedFieldIndex(matchIndex);
    setSearchActive(false);
    setStatus({
      message: `Found ${formFields[matchIndex]?.label ?? query}`,
      error: false,
    });
  };

  useKeyboard((key) => {
    const keyboardKey = key as KeyboardInput;
    const tab = keyboardKey.name === "tab";
    const up = isUpKey(keyboardKey);
    const down = isDownKey(keyboardKey);
    const left = isLeftKey(keyboardKey);
    const right = isRightKey(keyboardKey);
    const enter = isConfirmKey(keyboardKey);
    const escape = isEscapeKey(keyboardKey);

    if (searchActive) {
      if (escape) {
        cancelSearch();
        return;
      }
      if (enter) {
        submitSearch();
        return;
      }
      if (keyboardKey.name === "backspace") {
        setSearchText((current) => current.slice(0, -1));
        return;
      }

      const char = printableSearchChar(keyboardKey);
      if (char !== null) {
        setSearchText((current) => current + char);
      }
      return;
    }

    // 1. Ignore Tab outside search mode; this screen has one editor.
    if (tab) return;

    // 2. Editing toggle handling
    if (isEditing) {
      if (escape) {
        // Discard edits
        setLocalFormValues(
          formValuesWithSelectedProvider(payload.form_values, payload),
        );
        setIsEditing(false);
        return;
      }

      const focusedField = formFields[focusedFieldIndex];
      if (focusedField?.type === "toggle" || focusedField?.type === "choice") {
        if (
          left ||
          right ||
          keyboardKey.name === "space" ||
          keyboardKey.name === " "
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
          if (focusedField.key === SYNTHETIC_PROVIDER_KEY) {
            const provider = providerNameForDisplay(
              providerChoices,
              currentVal,
            );
            const providerIndex = providerChoices.findIndex(
              (choice) => choice.name === provider,
            );
            if (providerIndex >= 0) {
              selectProviderByIndex(providerIndex);
            }
            setIsEditing(false);
            return;
          }
          submitField(
            withFieldValue(localFormValues, focusedField.key, currentVal),
          );
        }
      }
      return;
    }

    // 3. Field navigation
    if (up) {
      setFocusedFieldIndex((prev) => Math.max(0, prev - 1));
      return;
    }
    if (down) {
      setFocusedFieldIndex((prev) =>
        Math.min(Math.max(formFields.length - 1, 0), prev + 1),
      );
      return;
    }
    if (enter && formFields.length > 0) {
      setIsEditing(true);
      return;
    }

    // 4. Global hotkeys
    if (keyboardKey.name === "/") {
      setSearchActive(true);
      setSearchText("");
      return;
    }

    if (keyboardKey.name === "q") {
      client?.close();
      renderer.destroy();
    }

    if (keyboardKey.name === "s") {
      handleSave();
      return;
    }

    if (keyboardKey.name === "r") {
      handleReload();
      return;
    }

    if (keyboardKey.name === "c") {
      handleCheck();
      return;
    }

    // Numeric provider selection shortcuts
    const providerIndex = providerIndexForInput(
      keyboardKey.name,
      providerChoices.length,
    );
    if (providerIndex >= 0) {
      selectProviderByIndex(providerIndex);
    }
  });

  if (layout.kind === "too-small") {
    return (
      <box flexDirection="column" width={dimensions.width}>
        <text>Terminal too small</text>
        <text fg="gray">
          {dimensions.width}x{dimensions.height}; minimum {MIN_TERMINAL_WIDTH}x
          {MIN_TERMINAL_HEIGHT}
        </text>
        <text fg="gray">Resize terminal to edit Hieronymus config.</text>
      </box>
    );
  }

  if (layout.kind !== "wide") {
    const compactHeight = panelHeight(layout, 13);
    const compactVisibleFormRows = Math.max(0, compactHeight - 4);
    const compactErrors = [...payload.validation.errors, ...detailErrors].slice(
      0,
      2,
    );

    return (
      <box flexDirection="column" width={dimensions.width}>
        <text>Hieronymus Config</text>
        <text fg="gray">Provider/API | Dreaming | Ingest | Release</text>

        <box
          flexDirection="column"
          marginTop={1}
          height={compactHeight}
          borderStyle="rounded"
          borderColor="cyan"
          paddingX={1}
        >
          <ConfigForm
            groups={payload.form_schema.groups}
            fields={formFields}
            formValues={localFormValues}
            focusedFieldIndex={focusedFieldIndex}
            isEditing={isEditing}
            focused
            width={contentWidth}
            visibleRows={compactVisibleFormRows}
            onFieldChange={handleFieldChange}
            onSubmitField={submitField}
          />
        </box>

        <box marginTop={1} flexDirection="column">
          <text>
            Models: {suggestions.length > 0 ? suggestions.join(", ") : "-"}
          </text>
          {compactErrors.map((error) => (
            <text key={error} fg="red">
              {error}
            </text>
          ))}
        </box>

        <SearchPrompt active={searchActive} query={searchText} />
        <StatusLine message={status.message} error={status.error} busy={busy} />
        <CompactKeyHelp providerChoices={providerChoices} />
      </box>
    );
  }

  return (
    <box flexDirection="column" width={Math.min(100, dimensions.width)}>
      <text>Hieronymus Config</text>
      <text fg="gray">Provider/API | Dreaming | Ingest | Release</text>

      <box
        flexDirection="column"
        marginTop={1}
        width={Math.min(96, dimensions.width)}
        borderStyle="rounded"
        borderColor="cyan"
        paddingX={1}
      >
        <ConfigForm
          groups={payload.form_schema.groups}
          fields={formFields}
          formValues={localFormValues}
          focusedFieldIndex={focusedFieldIndex}
          isEditing={isEditing}
          focused
          onFieldChange={handleFieldChange}
          onSubmitField={submitField}
        />
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

      <SearchPrompt active={searchActive} query={searchText} />
      <StatusLine message={status.message} error={status.error} busy={busy} />
      <KeyHelp keys={["↑↓ field", "Enter edit", "/ search", "q quit"]} />
      <KeyHelp
        keys={[
          `${providerKeyRange(providerChoices)} provider`,
          "s save",
          "r reload",
          "c check",
        ]}
      />
    </box>
  );
}

function CompactKeyHelp({
  providerChoices,
}: {
  providerChoices: ConfigBootstrap["provider_choices"];
}) {
  return (
    <box flexDirection="column">
      <KeyHelp
        keys={[
          "↑↓ field",
          "Enter edit",
          "/ search",
          `${providerKeyRange(providerChoices)} provider`,
        ]}
      />
      <KeyHelp keys={["s save", "r reload", "c check", "q quit"]} />
    </box>
  );
}

function SearchPrompt({ active, query }: { active: boolean; query: string }) {
  if (!active) {
    return null;
  }

  return (
    <box marginTop={1}>
      <text fg="cyan">Search: {query}</text>
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

function providerDisplayName(
  providerChoices: ConfigBootstrap["provider_choices"],
  provider: string,
): string {
  return (
    providerChoices.find((choice) => choice.name === provider)?.display_name ??
    provider
  );
}

function providerNameForDisplay(
  providerChoices: ConfigBootstrap["provider_choices"],
  displayName: string,
): ConfigBootstrap["provider_choices"][number]["name"] | undefined {
  return providerChoices.find((choice) => choice.display_name === displayName)
    ?.name;
}

function configFieldsWithProviderChoice(
  payload: ConfigBootstrap,
): ConfigFormField[] {
  const currentDisplayName = providerDisplayName(
    payload.provider_choices,
    payload.selected_provider,
  );
  const providerField: ConfigFormField = {
    key: SYNTHETIC_PROVIDER_KEY,
    group: "provider",
    section: "dream",
    label: "Provider",
    hint: "Provider family used for dreaming workflows and model checks.",
    placeholder: "",
    type: "choice",
    choices: payload.provider_choices.map((choice) => choice.display_name),
    default: currentDisplayName,
    redacted: false,
  };

  return [
    providerField,
    ...payload.form_schema.fields.filter(
      (field) => field.key !== SYNTHETIC_PROVIDER_KEY,
    ),
  ];
}

function formValuesWithSelectedProvider(
  values: ConfigFormValues,
  payload: ConfigBootstrap,
): ConfigFormValues {
  return {
    provider: {
      ...values.provider,
      __selected: providerDisplayName(
        payload.provider_choices,
        payload.selected_provider,
      ),
    },
    dreaming: { ...values.dreaming },
    ingest: { ...values.ingest },
    release: { ...values.release },
  };
}

function draftFormValues(values: ConfigFormValues): ConfigFormValues {
  const provider = { ...values.provider };
  delete provider.__selected;

  return {
    provider,
    dreaming: { ...values.dreaming },
    ingest: { ...values.ingest },
    release: { ...values.release },
  };
}

function findConfigFieldMatch(
  query: string,
  fields: ConfigFormField[],
  groups: Array<{
    id: string;
    section?: string;
    label?: string;
    description?: string;
  }>,
  sections: ConfigFormSection[],
): number {
  const normalizedQuery = normalizeSearchText(query);
  const groupById = new Map(groups.map((group) => [group.id, group]));
  const sectionById = new Map(sections.map((section) => [section.id, section]));

  return fields.findIndex((field) => {
    const group = groupById.get(field.group);
    const sectionId = field.section || group?.section || "";
    const section = sectionById.get(sectionId);
    const values = [
      field.label,
      field.key,
      field.hint,
      field.group,
      field.section,
      group?.label,
      group?.description,
      section?.label,
      section?.description,
    ];

    return values.some((value) =>
      normalizeSearchText(value ?? "").includes(normalizedQuery),
    );
  });
}

function normalizeSearchText(value: string): string {
  return value.trim().toLocaleLowerCase();
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
