import { afterEach, describe, expect, it } from "bun:test";
import { ConfigScreen } from "./ConfigScreen.js";
import type { RpcClient } from "../rpc/client.js";
import type {
  ConfigBootstrap,
  ConfigFormField,
  ProviderName,
} from "../rpc/schema.js";
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";

function formSchema(
  fields: ConfigFormField[] = [
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
    groups: [
      {
        id: "provider",
        label: "Provider",
        description: "Connection settings for the selected dream provider.",
      },
    ],
    fields,
  };
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
  return createOpenTuiHarness({ width: 132, height: 36 });
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
    expect(output).toContain("Tab pane");
    expect(output).not.toContain(
      "/tmp/dream.conf | /tmp/ingest.conf | /tmp/release.conf",
    );
  });

  it("renders a too-small config message below the minimum width", async () => {
    const { render, waitForFrame } = setupSizedTest(49, 20);

    await render(<ConfigScreen initial={payload()} client={undefined} />);

    const output = await waitForFrame((frame) =>
      frame.includes("Terminal too small"),
    );
    expect(output).toContain("49x20");
    expect(output).toContain("minimum 50x20");
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
          form_schema: { groups: [], fields: [] },
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
