import { afterEach, describe, expect, it } from "bun:test";
import { ConfigScreen } from "./ConfigScreen.js";
import type { RpcClient } from "../rpc/client.js";
import type { ConfigBootstrap, ProviderName } from "../rpc/schema.js";
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";

type TestConfigFormField = {
  key: string;
  group: string;
  section?: string;
  label: string;
  hint: string;
  placeholder: string;
  type: "text" | "secret" | "number" | "toggle" | "choice";
  choices: string[];
  default: string;
  minimum?: number;
  redacted: boolean;
};

function formSchema(
  fields: TestConfigFormField[] = [
    {
      key: "provider.model",
      group: "provider",
      label: "Model",
      hint: "Model name used by the selected dream provider.",
      placeholder: "gpt-4.1-mini",
      type: "text" as const,
      choices: [],
      default: "",
      redacted: false,
    },
    {
      key: "provider.api_key",
      group: "provider",
      label: "API Key",
      hint: "Stored as plaintext in dream.conf and redacted in UI payloads.",
      placeholder: "stored in dream.conf",
      type: "secret" as const,
      choices: [],
      default: "",
      redacted: true,
    },
  ],
) {
  return {
    sections: [
      { id: "dream", label: "Dream", description: "dream.conf" },
      { id: "ingest", label: "Ingest", description: "ingest.conf" },
      { id: "release", label: "Release", description: "release.conf" },
    ],
    groups: [
      {
        id: "provider",
        section: "dream",
        label: "Provider",
        description: "Connection settings for the selected dream provider.",
      },
      {
        id: "dreaming",
        section: "dream",
        label: "Dreaming",
        description:
          "Autostart thresholds for turning short-term memory into durable memory.",
      },
      {
        id: "ingest",
        section: "ingest",
        label: "Ingestion",
        description: "Limits for short-term memory and Learn ingestion.",
      },
      {
        id: "release",
        section: "release",
        label: "Updates",
        description: "Managed install update channel.",
      },
    ],
    fields: fields.map((field) => ({
      ...field,
      section: field.section ?? sectionForField(field.key),
    })),
  };
}

function sectionForField(key: string): string {
  if (key.startsWith("ingest.")) return "ingest";
  if (key.startsWith("release.")) return "release";
  return "dream";
}

function payload(selectedProvider: ProviderName = "openai"): ConfigBootstrap {
  return {
    config_paths: {
      dream_config_path: "/tmp/dream.conf",
      ingest_config_path: "/tmp/ingest.conf",
      release_config_path: "/tmp/release.conf",
    },
    provider_choices: [
      {
        name: "openai" as const,
        display_name: "OpenAI compatible",
        requires_api_key: true,
        supports_api_path: true,
      },
      {
        name: "gemini" as const,
        display_name: "Gemini",
        requires_api_key: true,
        supports_api_path: false,
      },
      {
        name: "anthropic" as const,
        display_name: "Anthropic",
        requires_api_key: true,
        supports_api_path: false,
      },
    ],
    selected_provider: selectedProvider,
    release: { update_channel: "stable", update_target: "latest" },
    ingest: { short_memory: {}, learn: {} },
    draft: {
      dream: {
        dreaming: {},
        providers: {},
        workflows: {},
      },
      ingest: { short_memory: {}, learn: {} },
      dreaming: { active_provider: selectedProvider },
      providers: {},
      workflows: {},
      release: { update_channel: "stable" },
    },
    form_values: {
      provider: {
        model:
          selectedProvider === "gemini" ? "gemini-2.5-flash" : "gpt-4.1-mini",
        api_key:
          selectedProvider === "gemini" ? "gemini-secret" : "openai-secret",
        api_path:
          selectedProvider === "openai" ? "https://api.openai.com/v1" : "",
        timeout_seconds: "30",
      },
      dreaming: {
        autostart_enabled: "no",
        min_interval_minutes: "30",
        new_short_term_memory_threshold: "25",
      },
      ingest: {
        warning_sentence_count: "6",
        rejection_sentence_count: "30",
        max_block_chars: "1200",
      },
      release: {
        update_channel: "stable",
      },
    },
    form_schema: formSchema(),
    validation: { ok: true, errors: [], field_errors: {} },
    check_result: {},
    suggestions: {
      provider: selectedProvider,
      models: [
        selectedProvider === "gemini" ? "gemini-2.5-flash" : "gpt-4.1-mini",
      ],
      source: "defaults",
      error: "",
    },
    detail: {
      title: `${selectedProvider} dreaming provider`,
      fields: [
        [
          "api_key",
          selectedProvider === "gemini" ? "gemini-secret" : "openai-secret",
        ],
      ],
      errors: [],
    },
  };
}

function setupTest() {
  return createOpenTuiHarness({ width: 136, height: 36 });
}

function setupSizedTest(width: number, height: number) {
  return createOpenTuiHarness({ width, height });
}

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});

describe("ConfigScreen", () => {
  it("renders config as a single active pane at 80x24", async () => {
    const { render, waitForFrame } = setupSizedTest(80, 24);

    await render(<ConfigScreen initial={payload()} client={undefined} />);

    const output = await waitForFrame((frame) =>
      frame.includes("Hieronymus Config"),
    );
    expect(output).toContain("Providers");
    expect(output).toContain("OpenAI compatible");
    expect(output).toContain("Config files: Dream | Ingest | Release");
    expect(output).toContain("Tab pane");
    expect(output).not.toContain(
      "/tmp/dream.conf | /tmp/ingest.conf | /tmp/release.conf",
    );
  });

  it("keeps compact footer visible with validation and detail errors", async () => {
    const { render, waitForFrame } = setupSizedTest(80, 24);

    await render(
      <ConfigScreen
        initial={{
          ...payload(),
          validation: {
            ok: false,
            errors: [
              "validation error one",
              "validation error two",
              "validation error three",
              "validation error four",
            ],
            field_errors: {},
          },
          detail: {
            title: "Provider check",
            fields: [],
            errors: [
              "detail error one",
              "detail error two",
              "detail error three",
              "detail error four",
            ],
          },
          suggestions: {
            provider: "openai",
            models: ["gpt-4.1-mini", "gpt-4.1", "o4-mini"],
            source: "defaults",
            error: "",
          },
        }}
        client={undefined}
      />,
    );

    const output = await waitForFrame(
      (frame) => frame.includes("q quit") || frame.includes("Ready"),
    );
    expect(output).toContain("q quit");
  });

  it("windows compact config fields without covering status or footer", async () => {
    const manyFields = Array.from({ length: 11 }, (_, index) => ({
      key: `provider.field_${index + 1}`,
      group: "provider",
      label: `Field ${index + 1}`,
      hint: `Field ${index + 1} hint.`,
      placeholder: `value-${index + 1}`,
      type: "text" as const,
      choices: [],
      default: "",
      redacted: false,
    }));
    const initial = {
      ...payload(),
      form_values: {
        ...payload().form_values,
        provider: {
          ...payload().form_values.provider,
          ...Object.fromEntries(
            manyFields.map((field, index) => [
              field.key.slice(9),
              `value-${index + 1}`,
            ]),
          ),
        },
      },
      form_schema: formSchema(manyFields),
    };
    const { render, mockInput, waitForFrame } = setupSizedTest(80, 24);

    await render(<ConfigScreen initial={initial} client={undefined} />);

    await mockInput.press("tab");
    for (let index = 0; index < 10; index += 1) {
      await mockInput.press("down");
    }

    const output = await waitForFrame(
      (frame) =>
        frame.includes("> Field 11: value-11") &&
        (frame.includes("q quit") || frame.includes("Ready")),
    );
    expect(output).toContain("> Field 11: value-11");
    expect(output).toContain("q quit");
  });

  it("renders a too-small config message below the minimum width", async () => {
    const { render, waitForFrame } = setupSizedTest(59, 20);

    await render(<ConfigScreen initial={payload()} client={undefined} />);

    const output = await waitForFrame((frame) =>
      frame.includes("Terminal too small"),
    );
    expect(output).toContain("59x20");
    expect(output).toContain("minimum 60x20");
  });

  it("renders one provider family selector instead of provider rows", async () => {
    const { render, waitForFrame } = setupTest();

    await render(<ConfigScreen initial={payload()} client={undefined} />);

    const output = await waitForFrame((frame) =>
      frame.includes("OpenAI compatible"),
    );
    expect(output).toContain("OpenAI compatible");
    expect(output).toContain("Gemini");
    expect(output).toContain("Anthropic");
    expect(output).not.toContain("Deterministic");
  });

  it("renders model suggestions when present", async () => {
    const { render, waitForFrame } = setupTest();

    await render(<ConfigScreen initial={payload()} client={undefined} />);

    const output = await waitForFrame((frame) =>
      frame.includes("gpt-4.1-mini"),
    );
    expect(output).toContain("gpt-4.1-mini");
  });

  it("renders config fields from backend schema", async () => {
    const { render, waitForFrame } = setupTest();

    await render(
      <ConfigScreen
        initial={{
          ...payload(),
          form_schema: formSchema([
            {
              key: "provider.model",
              group: "provider",
              label: "Backend Model Label",
              hint: "Backend-owned model hint.",
              placeholder: "backend placeholder",
              type: "text",
              choices: [],
              default: "",
              redacted: false,
            },
          ]),
        }}
        client={undefined}
      />,
    );

    const output = await waitForFrame((frame) =>
      frame.includes("Backend Model Label"),
    );
    expect(output).toContain("Backend Model Label");
  });

  it("renders backend-owned config section labels", async () => {
    const { render, waitForFrame } = setupTest();

    await render(<ConfigScreen initial={payload()} client={undefined} />);

    const output = await waitForFrame((frame) =>
      frame.includes("Config files: Dream | Ingest | Release"),
    );
    expect(output).toContain("Config files: Dream | Ingest | Release");
  });

  it("renders a placeholder when model suggestions are absent", async () => {
    const { render, waitForFrame } = setupTest();

    await render(
      <ConfigScreen
        initial={{ ...payload(), suggestions: {} }}
        client={undefined}
      />,
    );

    const output = await waitForFrame((frame) => frame.includes("Models: -"));
    expect(output).toContain("Models: -");
  });

  it("renders provider shortcut help from provider choices", async () => {
    const { render, waitForFrame } = setupTest();

    await render(
      <ConfigScreen
        initial={{
          ...payload(),
          provider_choices: payload().provider_choices.slice(0, 2),
        }}
        client={undefined}
      />,
    );

    const output = await waitForFrame((frame) =>
      frame.includes("1-2] provider"),
    );
    expect(output).toContain("1-2] provider");
    expect(output).not.toContain("1/2/3 provider");
  });

  it("selects a provider through the configured RPC", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.resolve(payload("gemini"));
    });
    const { render, mockInput, captureCharFrame, waitForFrame } = setupTest();

    await render(<ConfigScreen initial={payload()} client={client} />);

    await mockInput.type("2");

    await waitForFrame((frame) => frame.includes("Selected gemini"));

    expect(calls).toEqual([
      {
        method: "config.select_provider",
        params: {
          provider: "gemini",
          draft: payload().draft,
        },
      },
    ]);

    const output = captureCharFrame();
    expect(output).toContain("▶ Gemini");
    expect(output).toContain("gemini-2.5-flash");
  });

  it("moves through providers with j and k like arrow keys", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.resolve(payload(params.provider as ProviderName));
    });
    const { render, mockInput, waitForFrame } = setupTest();

    await render(<ConfigScreen initial={payload()} client={client} />);

    await mockInput.type("j");
    await waitForFrame((frame) => frame.includes("Selected gemini"));

    await mockInput.type("k");
    await waitForFrame((frame) => frame.includes("Selected openai"));

    expect(calls.map((call) => call.params.provider)).toEqual([
      "gemini",
      "openai",
    ]);
  });

  it("moves through form fields with j and k like arrow keys", async () => {
    const { render, mockInput, waitForFrame } = setupTest();

    await render(<ConfigScreen initial={payload()} client={undefined} />);

    await mockInput.press("tab");
    await mockInput.type("j");

    let output = await waitForFrame((frame) => frame.includes("> API Key"));
    expect(output).toContain("> API Key");

    await mockInput.type("k");

    output = await waitForFrame((frame) => frame.includes("> Model"));
    expect(output).toContain("> Model");
  });

  it("moves between config panels with h and l", async () => {
    const { render, mockInput, waitForFrame } = setupTest();

    await render(<ConfigScreen initial={payload()} client={undefined} />);

    await mockInput.type("l");

    let output = await waitForFrame((frame) => frame.includes("> Model"));
    expect(output).toContain("> Model");

    await mockInput.type("h");

    output = await waitForFrame((frame) =>
      frame.includes("▶ OpenAI compatible"),
    );
    expect(output).toContain("▶ OpenAI compatible");
  });

  it("searches config fields from the keyboard", async () => {
    const { render, mockInput, waitForFrame } = setupTest();

    await render(<ConfigScreen initial={payload()} client={undefined} />);

    await mockInput.type("/");

    let output = await waitForFrame((frame) => frame.includes("Search: "));
    expect(output).toContain("Search: ");

    await mockInput.type("api");

    output = await waitForFrame((frame) => frame.includes("Search: api"));
    expect(output).toContain("Search: api");

    await mockInput.press("enter");

    output = await waitForFrame((frame) => frame.includes("> API Key"));
    expect(output).toContain("> API Key");
  });

  it("keeps tab inside active config search mode", async () => {
    const { render, mockInput, waitForFrame } = setupTest();

    await render(<ConfigScreen initial={payload()} client={undefined} />);

    await mockInput.type("/");
    await mockInput.type("mod");
    await waitForFrame((frame) => frame.includes("Search: mod"));

    await mockInput.press("tab");

    const output = await waitForFrame((frame) => frame.includes("Search: mod"));
    expect(output).toContain("Search: mod");
    expect(output).toContain("▶ OpenAI compatible");
    expect(output).not.toContain("> Model");
  });

  it("searches config fields by hint, group, and section metadata", async () => {
    const { render, mockInput, waitForFrame } = setupTest();
    const initial = {
      ...payload(),
      form_schema: formSchema([
        {
          key: "provider.model",
          group: "provider",
          section: "dream",
          label: "Model",
          hint: "unique model hint",
          placeholder: "gpt-4.1-mini",
          type: "text" as const,
          choices: [],
          default: "",
          redacted: false,
        },
        {
          key: "release.update_channel",
          group: "release",
          section: "release",
          label: "Update channel",
          hint: "Release update channel.",
          placeholder: "",
          type: "choice" as const,
          choices: ["stable", "dev"],
          default: "stable",
          redacted: false,
        },
        {
          key: "ingest.warning_sentence_count",
          group: "ingest",
          section: "ingest",
          label: "Memory warn sentences",
          hint: "Warn before rejection.",
          placeholder: "6",
          type: "number" as const,
          choices: [],
          default: "6",
          redacted: false,
        },
      ]),
    };

    await render(<ConfigScreen initial={initial} client={undefined} />);

    await mockInput.type("/");
    await mockInput.type("unique model hint");
    await mockInput.press("enter");
    let output = await waitForFrame((frame) => frame.includes("> Model"));
    expect(output).toContain("> Model");

    await mockInput.type("/");
    await mockInput.type("release.conf");
    await mockInput.press("enter");
    output = await waitForFrame((frame) => frame.includes("> Update channel"));
    expect(output).toContain("> Update channel");

    await mockInput.type("/");
    await mockInput.type("Ingestion");
    await mockInput.press("enter");
    output = await waitForFrame((frame) =>
      frame.includes("> Memory warn sentences"),
    );
    expect(output).toContain("> Memory warn sentences");
  });

  it("ignores further action keys while an operation is in flight", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const deferred = deferredPayload();
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return deferred.promise;
    });
    const { render, mockInput, waitFor, waitForFrame } = setupTest();

    await render(<ConfigScreen initial={payload()} client={client} />);

    await mockInput.type("2");
    await mockInput.type("3");

    await waitFor(async () => calls.length >= 1);

    expect(calls[0]).toMatchObject({
      method: "config.select_provider",
      params: { provider: "gemini" },
    });

    deferred.resolve(payload("gemini"));

    await waitForFrame((frame) => frame.includes("Selected gemini"));
    expect(calls).toHaveLength(1);
  });

  it("updates schema-driven number, toggle, and choice fields from the form panel", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const initial = {
      ...payload(),
      form_schema: formSchema([
        {
          key: "provider.timeout_seconds",
          group: "provider",
          label: "Timeout",
          hint: "Request timeout in seconds.",
          placeholder: "30",
          type: "number" as const,
          choices: [],
          default: "30",
          minimum: 1,
          redacted: false,
        },
        {
          key: "dreaming.autostart_enabled",
          group: "provider",
          label: "Autostart",
          hint: "Start dreaming automatically.",
          placeholder: "",
          type: "toggle" as const,
          choices: ["no", "yes"],
          default: "no",
          redacted: false,
        },
        {
          key: "release.update_channel",
          group: "provider",
          label: "Update channel",
          hint: "Release update channel.",
          placeholder: "",
          type: "choice" as const,
          choices: ["stable", "beta", "dev"],
          default: "stable",
          redacted: false,
        },
      ]),
    };
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.resolve(initial);
    });
    const { render, mockInput, waitFor } = setupTest();

    await render(<ConfigScreen initial={initial} client={client} />);

    await mockInput.press("tab");
    await mockInput.press("enter");
    await mockInput.press("backspace");
    await mockInput.press("backspace");
    await mockInput.type("45");
    await mockInput.press("enter");

    await waitFor(async () => calls.length >= 1);

    await mockInput.press("down");
    await mockInput.press("enter");
    await mockInput.press("right");
    await mockInput.press("enter");

    await waitFor(async () => calls.length >= 2);

    await mockInput.press("down");
    await mockInput.press("enter");
    await mockInput.press("right");
    await mockInput.press("enter");

    await waitFor(async () => calls.length >= 3);

    expect(calls).toEqual([
      {
        method: "config.update_draft",
        params: {
          selected_provider: "openai",
          provider: {
            model: "gpt-4.1-mini",
            api_key: "openai-secret",
            api_path: "https://api.openai.com/v1",
            timeout_seconds: "45",
          },
          dreaming: {
            autostart_enabled: "no",
            min_interval_minutes: "30",
            new_short_term_memory_threshold: "25",
          },
          ingest: {
            warning_sentence_count: "6",
            rejection_sentence_count: "30",
            max_block_chars: "1200",
          },
          release: {
            update_channel: "stable",
          },
        },
      },
      {
        method: "config.update_draft",
        params: {
          selected_provider: "openai",
          provider: {
            model: "gpt-4.1-mini",
            api_key: "openai-secret",
            api_path: "https://api.openai.com/v1",
            timeout_seconds: "30",
          },
          dreaming: {
            autostart_enabled: "yes",
            min_interval_minutes: "30",
            new_short_term_memory_threshold: "25",
          },
          ingest: {
            warning_sentence_count: "6",
            rejection_sentence_count: "30",
            max_block_chars: "1200",
          },
          release: {
            update_channel: "stable",
          },
        },
      },
      {
        method: "config.update_draft",
        params: {
          selected_provider: "openai",
          provider: {
            model: "gpt-4.1-mini",
            api_key: "openai-secret",
            api_path: "https://api.openai.com/v1",
            timeout_seconds: "30",
          },
          dreaming: {
            autostart_enabled: "no",
            min_interval_minutes: "30",
            new_short_term_memory_threshold: "25",
          },
          ingest: {
            warning_sentence_count: "6",
            rejection_sentence_count: "30",
            max_block_chars: "1200",
          },
          release: {
            update_channel: "beta",
          },
        },
      },
    ]);
  });

  it("submits schema-effective choice defaults from the form panel", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const initial = {
      ...payload(),
      form_values: {
        ...payload().form_values,
        release: {},
      },
      form_schema: formSchema([
        {
          key: "release.update_channel",
          group: "release",
          label: "Update channel",
          hint: "Release update channel.",
          placeholder: "",
          type: "choice" as const,
          choices: ["stable", "beta", "dev"],
          default: "stable",
          redacted: false,
        },
      ]),
    };
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.resolve(initial);
    });
    const { render, mockInput, waitFor } = setupTest();

    await render(<ConfigScreen initial={initial} client={client} />);

    await mockInput.press("tab");
    await mockInput.press("enter");
    await mockInput.press("enter");

    await waitFor(async () => calls.length >= 1);

    await mockInput.press("enter");
    await mockInput.press("right");
    await mockInput.press("enter");

    await waitFor(async () => calls.length >= 2);

    expect(calls[0]?.params.release).toEqual({ update_channel: "stable" });
    expect(calls[1]?.params.release).toEqual({ update_channel: "beta" });
  });

  it("ignores form edit keys when the backend returns an empty schema", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.resolve(payload());
    });
    const { render, mockInput } = setupTest();

    await render(
      <ConfigScreen
        initial={{
          ...payload(),
          form_schema: { sections: [], groups: [], fields: [] },
        }}
        client={client}
      />,
    );

    await mockInput.press("tab");
    await mockInput.press("down");
    await mockInput.press("enter");

    expect(calls).toEqual([]);
  });

  it("closes the client when q is pressed", async () => {
    let closeCalls = 0;
    const client = fakeClient(
      () => Promise.reject(new Error("unexpected request")),
      () => {
        closeCalls += 1;
      },
    );
    const { render, mockInput, waitFor } = setupTest();

    await render(<ConfigScreen initial={payload()} client={client} />);

    await mockInput.type("q");

    await waitFor(async () => closeCalls >= 1);
  });
});

function fakeClient(
  request: (
    method: string,
    params: Record<string, unknown>,
  ) => Promise<Record<string, unknown>>,
  close?: () => void,
): RpcClient {
  return { request, close: close ?? (() => {}) };
}

function deferredPayload() {
  let resolve!: (payload: ConfigBootstrap) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<Record<string, unknown>>(
    (promiseResolve, promiseReject) => {
      resolve = (payload) => promiseResolve(payload);
      reject = promiseReject;
    },
  );
  return { promise, resolve, reject };
}
