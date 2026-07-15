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
  type ConfigFormGroup,
  type ConfigFormSection,
} from "../rpc/schema.js";
import type { RpcClient } from "../rpc/client.js";
import { ConfigForm } from "./ConfigForm.js";
import { KeyHelp } from "../ui/KeyHelp.js";
import { StatusLine } from "../ui/StatusLine.js";
import { theme } from "../ui/theme.js";
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
  providerCatalog: Record<string, string>;
  workflows: Record<string, string>;
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
  const localFormValuesRef = useRef(localFormValues);
  const synchronizedPayload = useRef(payload);

  // Refresh the draft only after an actual bootstrap response. OpenTUI may
  // render several times for one keyboard event; resetting it on those renders
  // loses all but the last typed character.
  useEffect(() => {
    if (synchronizedPayload.current === payload) {
      return;
    }
    synchronizedPayload.current = payload;
    const nextValues = formValuesWithSelectedProvider(
      payload.form_values,
      payload,
    );
    localFormValuesRef.current = nextValues;
    setLocalFormValues(nextValues);
  }, [payload]);

  const providerChoices = payload.provider_choices;
  const formFields = configFieldsWithProviderChoice(payload);

  const suggestions = modelSuggestions(payload);
  const suggestionSource = modelSuggestionSource(payload);
  const suggestionError = modelSuggestionError(payload);
  const checkSummary = providerCheckSummary(payload);
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
    const nextValues = withFieldValue(localFormValuesRef.current, key, value);
    localFormValuesRef.current = nextValues;
    setLocalFormValues(nextValues);
  };

  const submitField = (
    formValues: ConfigFormValues = localFormValuesRef.current,
  ) => {
    setIsEditing(false);
    if (!client || busy || operationInFlight.current) {
      return;
    }
    const draftValues = draftFormValues(formValues);
    const draft = draftWithFormValues(
      payload.draft,
      draftValues,
      payload.selected_provider,
    );

    void runConfigOperation({
      client,
      method: "config.update_draft",
      params: {
        selected_provider: payload.selected_provider,
        draft,
        provider: draftValues.provider,
        provider_catalog: draftValues.providerCatalog,
        workflows: draftValues.workflows,
        dreaming: draftValues.dreaming,
        ingest: draftValues.ingest,
        release: draftValues.release,
      },
      pendingMessage: "Updating draft settings",
      successMessage: "Draft settings updated — press s to save",
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
        const resetValues = formValuesWithSelectedProvider(
          payload.form_values,
          payload,
        );
        localFormValuesRef.current = resetValues;
        setLocalFormValues(resetValues);
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
            localFormValuesRef.current,
            focusedField,
          );
          const currentIndex = Math.max(choices.indexOf(currentVal), 0);
          const delta = left ? -1 : 1;
          const nextIndex =
            (currentIndex + delta + choices.length) % choices.length;
          handleFieldChange(focusedField.key, choices[nextIndex]);
        } else if (enter) {
          const currentVal = effectiveValueForField(
            localFormValuesRef.current,
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
            withFieldValue(
              localFormValuesRef.current,
              focusedField.key,
              currentVal,
            ),
          );
        }
      } else {
        const currentVal = effectiveValueForField(
          localFormValuesRef.current,
          focusedField,
        );
        if (keyboardKey.name === "backspace") {
          handleFieldChange(focusedField.key, currentVal.slice(0, -1));
        } else if (enter) {
          submitField();
        } else {
          const char = printableSearchChar(keyboardKey);
          if (char !== null) {
            handleFieldChange(focusedField.key, currentVal + char);
          }
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
      <box
        flexDirection="column"
        width={dimensions.width}
        height={dimensions.height}
      >
        <text>Terminal too small</text>
        <text fg={theme.accentMuted}>
          {dimensions.width}x{dimensions.height}; minimum {MIN_TERMINAL_WIDTH}x
          {MIN_TERMINAL_HEIGHT}
        </text>
        <text fg={theme.accentMuted}>
          Resize terminal to edit Hieronymus config.
        </text>
      </box>
    );
  }

  if (layout.kind !== "wide") {
    const compactHeight = panelHeight(layout, 13);
    const compactVisibleFormRows = Math.max(1, compactHeight - 7);
    const compactErrors = [...payload.validation.errors, ...detailErrors].slice(
      0,
      2,
    );

    return (
      <box flexDirection="column" width={dimensions.width}>
        <ConfigHeader
          width={dimensions.width}
          sections={payload.form_schema.sections}
        />

        <box
          flexDirection="column"
          marginTop={1}
          height={compactHeight}
          borderStyle="rounded"
          borderColor={theme.accentPrimary}
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
          />
        </box>

        <box marginTop={1} flexDirection="column">
          <ConfigDiagnostics
            models={suggestions}
            source={suggestionSource}
            suggestionError={suggestionError}
            checkSummary={checkSummary}
          />
          {compactErrors.map((error) => (
            <text key={error} fg={theme.statusError}>
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

  const widePanelRows = panelHeight(layout, 10);
  const widePanelWidth = Math.min(96, dimensions.width - 2);
  const wideFormWidth = Math.max(20, widePanelWidth - 4);
  const wideVisibleFormRows =
    dimensions.height < 44 &&
    estimatedWideFormRows(payload.form_schema.groups, formFields) >
      widePanelRows
      ? Math.max(1, Math.min(6, widePanelRows - 10))
      : undefined;

  return (
    <box
      flexDirection="column"
      width={Math.min(100, dimensions.width)}
      height={dimensions.height}
    >
      <ConfigHeader
        width={Math.min(100, dimensions.width)}
        sections={payload.form_schema.sections}
      />

      <box
        flexDirection="column"
        marginTop={1}
        width={widePanelWidth}
        height={widePanelRows}
        borderStyle="rounded"
        borderColor={theme.accentPrimary}
        paddingX={1}
      >
        <ConfigForm
          groups={payload.form_schema.groups}
          fields={formFields}
          formValues={localFormValues}
          focusedFieldIndex={focusedFieldIndex}
          isEditing={isEditing}
          focused
          width={wideFormWidth}
          visibleRows={wideVisibleFormRows}
        />
      </box>

      <box marginTop={1} flexDirection="column">
        <ConfigDiagnostics
          models={suggestions}
          source={suggestionSource}
          suggestionError={suggestionError}
          checkSummary={checkSummary}
        />
        {payload.validation.errors.map((error) => (
          <text key={error} fg={theme.statusError}>
            {error}
          </text>
        ))}
        {detailErrors.map((error) => (
          <text key={error} fg={theme.statusError}>
            {error}
          </text>
        ))}
      </box>

      <SearchPrompt active={searchActive} query={searchText} />
      <StatusLine message={status.message} error={status.error} busy={busy} />
      <KeyHelp
        keys={[
          "↑↓ field",
          "Enter edit",
          "/ search",
          "q quit",
          `${providerKeyRange(providerChoices)} provider`,
          "s save",
          "r reload",
          "c check",
        ]}
      />
    </box>
  );
}

function ConfigHeader({
  width,
  sections,
}: {
  width: number;
  sections: ConfigFormSection[];
}) {
  const labels = sectionLabels(sections);
  return (
    <box flexDirection="column" width={width} height={2}>
      <text width={width}>Hieronymus Config</text>
      <text width={width} fg={theme.accentMuted}>
        {labels.join(" | ")}
      </text>
    </box>
  );
}

function ConfigDiagnostics({
  models,
  source,
  suggestionError,
  checkSummary,
}: {
  models: string[];
  source: string;
  suggestionError: string;
  checkSummary: { ok: boolean; text: string } | null;
}) {
  return (
    <box flexDirection="column">
      <text>{modelSummary(models, source)}</text>
      <text fg={theme.statusError}>{suggestionError || " "}</text>
      <text fg={checkSummary?.ok ? theme.statusSuccess : theme.statusError}>
        {checkSummary?.text || " "}
      </text>
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
          "q quit",
          `${providerKeyRange(providerChoices)} provider`,
        ]}
      />
      <KeyHelp keys={["s save", "r reload", "c check"]} />
    </box>
  );
}

function SearchPrompt({ active, query }: { active: boolean; query: string }) {
  if (!active) {
    return null;
  }

  return (
    <box marginTop={1}>
      <text fg={theme.accentPrimary}>Search: {query}</text>
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

function modelSuggestionSource(payload: ConfigBootstrap): string {
  const { suggestions } = payload;
  if (typeof suggestions === "object" && "source" in suggestions) {
    return suggestions.source;
  }
  return "";
}

function modelSuggestionError(payload: ConfigBootstrap): string {
  const { suggestions } = payload;
  if (typeof suggestions === "object" && "error" in suggestions) {
    return suggestions.error;
  }
  return "";
}

function modelSummary(models: string[], source: string): string {
  if (models.length === 0) {
    return "Models: -";
  }
  if (!source || source === "defaults") {
    return `Models: ${models.join(", ")}`;
  }
  return `Models (${source}): ${models.join(", ")}`;
}

function providerCheckSummary(
  payload: ConfigBootstrap,
): { ok: boolean; text: string } | null {
  const result = payload.check_result;
  if (typeof result !== "object" || Object.keys(result).length === 0) {
    return null;
  }
  const name =
    typeof result.name === "string" ? result.name : payload.selected_provider;
  const model =
    typeof result.model === "string" && result.model ? ` ${result.model}` : "";
  const latency =
    typeof result.latency_ms === "number" ? ` ${result.latency_ms}ms` : "";
  if (result.ok === true) {
    return { ok: true, text: `Check: ${name} ok${model}${latency}` };
  }
  const error =
    typeof result.error === "string" && result.error
      ? ` - ${result.error}`
      : "";
  return { ok: false, text: `Check: ${name} failed${model}${latency}${error}` };
}

function sectionLabels(sections: ConfigFormSection[]): string[] {
  const labels = sections
    .map((section) => section.label.trim())
    .filter((label) => label.length > 0);
  if (labels.length > 0) {
    return labels;
  }
  return ["Providers", "Workflows", "Dreaming", "Ingest", "Release"];
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
  values: ConfigBootstrap["form_values"],
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
    providerCatalog: flattenProviderCatalogFormValues(values.provider_catalog),
    workflows: { ...values.workflows },
    dreaming: { ...values.dreaming },
    ingest: { ...values.ingest },
    release: { ...values.release },
  };
}

function flattenProviderCatalogFormValues(
  values: ConfigBootstrap["form_values"]["provider_catalog"],
): Record<string, string> {
  const flat: Record<string, string> = {};

  for (const [key, value] of Object.entries(values)) {
    if (isScalarFormValue(value)) {
      flat[key] = String(value);
      continue;
    }
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
      continue;
    }
    for (const [nestedKey, nestedValue] of Object.entries(value)) {
      if (isScalarFormValue(nestedValue)) {
        flat[`${key}.${nestedKey}`] = String(nestedValue);
      }
    }
  }

  return flat;
}

function isScalarFormValue(value: unknown): value is string | number | boolean {
  return (
    typeof value === "string" ||
    typeof value === "number" ||
    typeof value === "boolean"
  );
}

function draftFormValues(values: ConfigFormValues): ConfigFormValues {
  const provider = { ...values.provider };
  delete provider.__selected;

  return {
    provider,
    providerCatalog: { ...values.providerCatalog },
    workflows: { ...values.workflows },
    dreaming: { ...values.dreaming },
    ingest: { ...values.ingest },
    release: { ...values.release },
  };
}

function draftWithFormValues(
  draft: ConfigBootstrap["draft"],
  values: ConfigFormValues,
  selectedProvider: string,
): ConfigBootstrap["draft"] {
  return {
    ...draft,
    provider_catalog: providerCatalogDraftWithFormValues(
      draft.provider_catalog,
      values.providerCatalog,
      selectedProvider,
    ),
    workflows: sectionDraftWithFormValues(
      draft.workflows,
      values.workflows,
    ) as ConfigBootstrap["draft"]["workflows"],
    dreaming: sectionDraftWithFormValues(draft.dreaming, values.dreaming),
    ingest: sectionDraftWithFormValues(
      draft.ingest,
      values.ingest,
    ) as ConfigBootstrap["draft"]["ingest"],
    release: sectionDraftWithFormValues(draft.release, values.release),
  };
}

function providerCatalogDraftWithFormValues(
  section: ConfigBootstrap["draft"]["provider_catalog"],
  formValues: Record<string, string>,
  selectedProvider: string,
): ConfigBootstrap["draft"]["provider_catalog"] {
  const canonicalSection: Record<string, unknown> = { ...section };
  delete canonicalSection.profile;
  const canonicalFormValues: Record<string, string> = {};

  for (const [key, value] of Object.entries(formValues)) {
    canonicalFormValues[
      key.startsWith("profile.")
        ? `profiles.${selectedProvider}.${key.slice(8)}`
        : key
    ] = value;
  }

  return sectionDraftWithFormValues(
    canonicalSection,
    canonicalFormValues,
  ) as ConfigBootstrap["draft"]["provider_catalog"];
}

function sectionDraftWithFormValues(
  section: Record<string, unknown>,
  formValues: Record<string, string>,
): Record<string, unknown> {
  const next = { ...section };
  for (const [key, value] of Object.entries(formValues)) {
    setDottedDraftValue(next, key, value);
  }
  return next;
}

function setDottedDraftValue(
  target: Record<string, unknown>,
  key: string,
  value: string,
) {
  const parts = key.split(".").filter(Boolean);
  if (parts.length === 0) {
    return;
  }

  let cursor = target;
  for (const part of parts.slice(0, -1)) {
    const child = cursor[part];
    const next =
      child !== null && typeof child === "object" && !Array.isArray(child)
        ? { ...(child as Record<string, unknown>) }
        : {};
    cursor[part] = next;
    cursor = next;
  }
  cursor[parts[parts.length - 1]] = value;
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
  if (key.startsWith("provider_catalog.")) {
    return values.providerCatalog[key.slice(17)] || "";
  }
  if (key.startsWith("workflows.")) {
    return values.workflows[key.slice(10)] || "";
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
  const providerCatalogDraft = { ...values.providerCatalog };
  const workflowsDraft = { ...values.workflows };
  const dreamingDraft = { ...values.dreaming };
  const ingestDraft = { ...values.ingest };
  const releaseDraft = { ...values.release };

  if (key.startsWith("provider.")) {
    providerDraft[key.slice(9)] = value;
  } else if (key.startsWith("provider_catalog.")) {
    providerCatalogDraft[key.slice(17)] = value;
  } else if (key.startsWith("workflows.")) {
    workflowsDraft[key.slice(10)] = value;
  } else if (key.startsWith("dreaming.")) {
    dreamingDraft[key.slice(9)] = value;
  } else if (key.startsWith("ingest.")) {
    ingestDraft[key.slice(7)] = value;
  } else if (key.startsWith("release.")) {
    releaseDraft[key.slice(8)] = value;
  }

  return {
    provider: providerDraft,
    providerCatalog: providerCatalogDraft,
    workflows: workflowsDraft,
    dreaming: dreamingDraft,
    ingest: ingestDraft,
    release: releaseDraft,
  };
}

function providerIndexForInput(input: string, providerCount: number) {
  const index = providerKeys(providerCount).indexOf(input);
  return index >= 0 ? index : -1;
}

function estimatedWideFormRows(
  groups: ConfigFormGroup[],
  fields: ConfigFormField[],
): number {
  const formHeaderRows = 2;
  const groupChromeRows = 4;
  const activeHintRows = 2;
  const fieldCounts = new Map<string, number>();
  for (const field of fields) {
    fieldCounts.set(field.group, (fieldCounts.get(field.group) ?? 0) + 1);
  }

  let rows = formHeaderRows;
  for (const group of groups) {
    const fieldCount = fieldCounts.get(group.id) ?? 0;
    if (fieldCount === 0) {
      continue;
    }

    rows += groupChromeRows;
    rows += group.description ? 1 : 0;
    rows += fieldCount;
  }

  rows += activeHintRows;
  return rows;
}
