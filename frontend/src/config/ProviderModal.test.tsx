import { afterEach, describe, expect, it } from "bun:test";
import { ProviderModal } from "./ProviderModal.js";
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});

describe("ProviderModal", () => {
  it("edits a local API key draft and submits only the profile", async () => {
    const submitted: Array<Record<string, string>> = [];
    const { render, mockInput, waitForFrame } = createOpenTuiHarness({
      width: 100,
      height: 32,
    });

    await render(
      <ProviderModal
        provider={{
          id: "openai",
          name: "OpenAI",
          type: "openai",
          url: "https://api.openai.com/v1",
          key_configured: true,
          model: "gpt-4.1-mini",
          timeout_seconds: 30,
        }}
        onCancel={() => {}}
        onSave={(provider) => submitted.push(provider)}
      />,
    );

    await mockInput.press("down");
    await mockInput.press("down");
    await mockInput.press("enter");
    await mockInput.type("new-key");
    await mockInput.press("enter");
    await mockInput.press("s");

    await waitForFrame((frame) => frame.includes("Saved OpenAI"));
    expect(submitted).toEqual([
      {
        id: "openai",
        name: "OpenAI",
        type: "openai",
        url: "https://api.openai.com/v1",
        key: "new-key",
        model: "gpt-4.1-mini",
        timeout_seconds: "30",
      },
    ]);
  });
});
