import React from "react";
import { describe, expect, it } from "vitest";
import { render } from "ink-testing-library";
import { ConfigScreen } from "./ConfigScreen.js";

function payload() {
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
    selected_provider: "openai" as const,
    draft: { dreaming: { active_provider: "openai" }, providers: {} },
    form_values: {
      provider: {
        model: "gpt-4.1-mini",
        api_key_env: "OPENAI_API_KEY",
        api_path: "https://api.openai.com/v1",
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
      provider: "openai" as const,
      models: ["gpt-4.1-mini"],
      source: "defaults",
      error: "",
    },
    detail: {
      title: "openai dreaming provider",
      fields: [["api_key_env", "OPENAI_API_KEY"]] as [string, string][],
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
});
