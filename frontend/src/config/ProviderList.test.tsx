import { afterEach, describe, expect, it } from "bun:test";
import { ProviderList } from "./ProviderList.js";
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});

describe("ProviderList", () => {
  it("selects one provider without mounting editor controls", async () => {
    const opened: string[] = [];
    const { render, mockInput, waitForFrame } = createOpenTuiHarness({
      width: 90,
      height: 24,
    });

    await render(
      <ProviderList
        providers={[
          {
            id: "openai",
            name: "OpenAI",
            type: "openai",
            url: "https://api.openai.com/v1",
            key_configured: true,
            model: "gpt-4.1-mini",
            timeout_seconds: 30,
          },
          {
            id: "google",
            name: "Google",
            type: "google",
            url: "",
            key_configured: false,
            model: "",
            timeout_seconds: 30,
          },
        ]}
        active
        onOpen={(provider) => opened.push(provider.id)}
      />,
    );

    await mockInput.press("down");
    await mockInput.press("enter");

    await waitForFrame((frame) => frame.includes("Google"));
    expect(opened).toEqual(["google"]);
  });
});
