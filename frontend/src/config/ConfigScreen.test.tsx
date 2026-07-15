import { afterEach, describe, expect, it } from "bun:test";
import { ConfigScreen } from "./ConfigScreen.js";
import type { RpcClient } from "../rpc/client.js";
import type { ProviderList } from "../rpc/schema.js";
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});

const initial: ProviderList = {
  providers: [
    {
      id: "openai",
      name: "OpenAI",
      type: "openai",
      url: "https://api.openai.com/v1",
      key_configured: true,
      model: "gpt-4.1-mini",
      timeout_seconds: 30,
    },
  ],
  error: "",
};

describe("ConfigScreen", () => {
  it("closes the bridge when q is pressed outside a modal", async () => {
    let closed = false;
    const client: RpcClient = {
      request: () => Promise.reject(new Error("not used")),
      close: () => {
        closed = true;
      },
    };
    const { render, mockInput } = createOpenTuiHarness({
      width: 100,
      height: 32,
    });

    await render(<ConfigScreen initial={initial} client={client} />);
    await mockInput.press("q");

    expect(closed).toBeTrue();
  });

  it("opens a single modal and saves only its profile patch", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const client: RpcClient = {
      request: (method, params) => {
        calls.push({ method, params });
        if (method === "config.provider_detail") {
          return Promise.resolve({ provider: initial.providers[0], error: "" });
        }
        if (method === "config.save_provider") {
          return Promise.resolve({
            provider: { ...params.provider, key_configured: true },
            error: "",
          });
        }
        return Promise.reject(new Error(`unexpected ${method}`));
      },
      close: () => {},
    };
    const { render, mockInput, waitForFrame } = createOpenTuiHarness({
      width: 100,
      height: 32,
    });

    await render(<ConfigScreen initial={initial} client={client} />);
    await mockInput.press("enter");
    await waitForFrame((frame) => frame.includes("Edit provider: openai"));
    await mockInput.press("down");
    await mockInput.press("down");
    await mockInput.press("enter");
    await mockInput.type("test-key");
    await mockInput.press("enter");
    await mockInput.press("s");

    await waitForFrame((frame) => frame.includes("Provider saved"));
    expect(calls).toHaveLength(2);
    expect(calls[1]).toMatchObject({
      method: "config.save_provider",
      params: {
        provider: {
          id: "openai",
          key: "test-key",
        },
      },
    });
  });
});
