import React from "react";
import { describe, expect, it } from "vitest";
import { render } from "ink-testing-library";
import { ConfigScreen } from "./ConfigScreen.js";
import type { RpcClient } from "../rpc/client.js";
import type { ConfigBootstrap, ProviderName } from "../rpc/schema.js";

function payload(selectedProvider: ProviderName = "openai"): ConfigBootstrap {
  return {
    config_paths: { settings_path: "/tmp/settings.toml" },
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
    draft: { dreaming: { active_provider: selectedProvider }, providers: {} },
    form_values: {
      provider: {
        model:
          selectedProvider === "gemini" ? "gemini-2.5-flash" : "gpt-4.1-mini",
        api_key_env:
          selectedProvider === "gemini" ? "GEMINI_API_KEY" : "OPENAI_API_KEY",
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
          "api_key_env",
          selectedProvider === "gemini" ? "GEMINI_API_KEY" : "OPENAI_API_KEY",
        ],
      ],
      errors: [],
    },
  };
}

describe("ConfigScreen", () => {
  it("renders one provider family selector instead of provider rows", () => {
    const app = render(<ConfigScreen initial={payload()} client={undefined} />);

    expect(app.lastFrame()).toContain("OpenAI compatible");
    expect(app.lastFrame()).toContain("Gemini");
    expect(app.lastFrame()).toContain("Anthropic");
    expect(app.lastFrame()).not.toContain("Deterministic");
  });

  it("renders model suggestions when present", () => {
    const app = render(<ConfigScreen initial={payload()} client={undefined} />);

    expect(app.lastFrame()).toContain("gpt-4.1-mini");
  });

  it("renders a placeholder when model suggestions are absent", () => {
    const app = render(
      <ConfigScreen
        initial={{ ...payload(), suggestions: {} }}
        client={undefined}
      />,
    );

    expect(app.lastFrame()).toContain("Models: -");
  });

  it("renders provider shortcut help from provider choices", () => {
    const app = render(
      <ConfigScreen
        initial={{
          ...payload(),
          provider_choices: payload().provider_choices.slice(0, 2),
        }}
        client={undefined}
      />,
    );

    expect(app.lastFrame()).toContain("1-2 provider");
    expect(app.lastFrame()).not.toContain("1/2/3 provider");
  });

  it("selects a provider through the configured RPC", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.resolve(payload("gemini"));
    });
    const app = render(<ConfigScreen initial={payload()} client={client} />);

    await nextTick();
    app.stdin.write("2");
    await waitFor(() => expect(app.lastFrame()).toContain("Selected gemini"));

    expect(calls).toEqual([
      {
        method: "config.select_provider",
        params: {
          provider: "gemini",
          draft: payload().draft,
        },
      },
    ]);
    expect(app.lastFrame()).toContain("> Gemini");
    expect(app.lastFrame()).toContain("gemini-2.5-flash");
  });

  it("ignores further action keys while an operation is in flight", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const deferred = deferredPayload();
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return deferred.promise;
    });
    const app = render(<ConfigScreen initial={payload()} client={client} />);

    await nextTick();
    app.stdin.write("2");
    app.stdin.write("3");
    await waitFor(() => expect(calls).toHaveLength(1));

    expect(calls[0]).toMatchObject({
      method: "config.select_provider",
      params: { provider: "gemini" },
    });

    deferred.resolve(payload("gemini"));
    await waitFor(() => expect(app.lastFrame()).toContain("Selected gemini"));
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
    const app = render(<ConfigScreen initial={payload()} client={client} />);

    await nextTick();
    app.stdin.write("q");
    await waitFor(() => expect(closeCalls).toBe(1));
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

async function nextTick() {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

async function waitFor(assertion: () => void) {
  let lastError: unknown;
  for (let attempt = 0; attempt < 20; attempt += 1) {
    try {
      assertion();
      return;
    } catch (error) {
      lastError = error;
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
  }
  throw lastError;
}
