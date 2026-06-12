import React from "react";
import { act } from "react";
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/react/test-utils";
import { ConfigScreen } from "./ConfigScreen.js";
import type { RpcClient } from "../rpc/client.js";
import type { ConfigBootstrap, ProviderName } from "../rpc/schema.js";

function payload(selectedProvider: ProviderName = "openai"): ConfigBootstrap {
  return {
    config_paths: {
      settings_path: "/tmp/settings.toml",
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
        api_key: selectedProvider === "gemini" ? "gemini-secret" : "openai-secret",
        api_path:
          selectedProvider === "openai" ? "https://api.openai.com/v1" : "",
        timeout_seconds: "30",
      },
      dreaming: {
        autostart_enabled: "no",
        min_interval_minutes: "30",
        new_short_term_memory_threshold: "25",
        max_cycles_per_autostart: "1",
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
    validation: { ok: true, errors: [] },
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

async function setupTest() {
  let node: React.ReactNode = null;
  let setup: Awaited<ReturnType<typeof testRender>> | null = null;
  const ensureSetup = async () => {
    setup ??= await testRender(node, { width: 120, height: 36 });
    return setup;
  };
  const flush = async () => {
    const current = await ensureSetup();
    await act(async () => {
      await current.flush();
    });
  };
  const input = {
    type: async (value: string) => {
      const current = await ensureSetup();
      for (const key of value) {
        act(() => {
          current.mockInput.pressKey(key);
        });
      }
      await flush();
    },
  };
  return {
    root: {
      render: (next: React.ReactNode) => {
        node = next;
      },
    },
    mockInput: input,
    flush,
    captureCharFrame: () => setup?.captureCharFrame() ?? "",
    waitFor: async (predicate: () => boolean | Promise<boolean>) => {
      const current = await ensureSetup();
      for (let index = 0; index < 25; index += 1) {
        await act(async () => {
          await Promise.resolve();
          await current.renderOnce();
        });
        if (await predicate()) {
          return;
        }
      }
      throw new Error("Timed out waiting for predicate");
    },
  };
}

describe("ConfigScreen", () => {
  it("renders one provider family selector instead of provider rows", async () => {
    const { root, flush, captureCharFrame } = await setupTest();

    root.render(<ConfigScreen initial={payload()} client={undefined} />);
    await flush();

    const output = captureCharFrame();
    expect(output).toContain("OpenAI compatible");
    expect(output).toContain("Gemini");
    expect(output).toContain("Anthropic");
    expect(output).not.toContain("Deterministic");
  });

  it("renders model suggestions when present", async () => {
    const { root, flush, captureCharFrame } = await setupTest();

    root.render(<ConfigScreen initial={payload()} client={undefined} />);
    await flush();

    expect(captureCharFrame()).toContain("gpt-4.1-mini");
  });

  it("renders a placeholder when model suggestions are absent", async () => {
    const { root, flush, captureCharFrame } = await setupTest();

    root.render(
      <ConfigScreen
        initial={{ ...payload(), suggestions: {} }}
        client={undefined}
      />,
    );
    await flush();

    expect(captureCharFrame()).toContain("Models: -");
  });

  it("renders provider shortcut help from provider choices", async () => {
    const { root, flush, captureCharFrame } = await setupTest();

    root.render(
      <ConfigScreen
        initial={{
          ...payload(),
          provider_choices: payload().provider_choices.slice(0, 2),
        }}
        client={undefined}
      />,
    );
    await flush();

    const output = captureCharFrame();
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
    const { root, mockInput, flush, captureCharFrame, waitFor } =
      await setupTest();

    root.render(<ConfigScreen initial={payload()} client={client} />);
    await flush();

    await mockInput.type("2");

    await waitFor(async () => {
      const frame = captureCharFrame();
      return frame.includes("Selected gemini");
    });

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
    const { root, mockInput, flush, captureCharFrame, waitFor } =
      await setupTest();

    root.render(<ConfigScreen initial={payload()} client={client} />);
    await flush();

    await mockInput.type("2");
    await mockInput.type("3");

    await waitFor(async () => calls.length >= 1);

    expect(calls[0]).toMatchObject({
      method: "config.select_provider",
      params: { provider: "gemini" },
    });

    deferred.resolve(payload("gemini"));

    await waitFor(async () => {
      const frame = captureCharFrame();
      return frame.includes("Selected gemini");
    });
    expect(calls).toHaveLength(1);
  });

  it("closes the client when q is pressed", async () => {
    let closeCalls = 0;
    const client = fakeClient(
      () => Promise.reject(new Error("unexpected request")),
      () => {
        closeCalls += 1;
      },
    );
    const { root, mockInput, flush, waitFor } = await setupTest();

    root.render(<ConfigScreen initial={payload()} client={client} />);
    await flush();

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
