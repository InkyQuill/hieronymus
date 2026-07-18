import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import type {
  DreamSettings,
  ModelCache,
  ProviderDraft,
  ProviderProfile,
} from "../lib/types";
import DreamingEditor from "./DreamingEditor.svelte";
import ProviderEditor from "./ProviderEditor.svelte";

const provider = {
  id: "openai-main",
  name: "OpenAI Main",
  type: "openai",
  url: "https://api.openai.com/v1",
  key_configured: true,
  model: "gpt-5",
  timeout_seconds: 30,
} satisfies ProviderProfile;

const dream = {
  dreaming: {
    enabled: false,
    schedule_interval_minutes: 30,
    min_pending_short_term_memories: 20,
    max_pending_short_term_memories: 200,
    max_short_term_memories_per_cycle: 50,
    not_enough_memories_cycle_threshold: 5,
    max_changed_crystals_per_cycle: 200,
    max_related_concepts_per_cycle: 80,
    max_related_crystals_per_concept: 20,
    max_total_affected_crystals: 500,
    max_short_term_memories_per_run: 500,
    max_long_term_records_affected_per_run: 1000,
    max_relation_records_per_pass: 1000,
    general_prompt: "Keep evidence explicit.",
  },
  workflows: {
    concepts: {
      provider: "openai-main",
      model: "gpt-5",
      enabled: true,
      max_records_per_pass: 20,
    },
  },
} satisfies DreamSettings;

const modelCache = {
  providers: { "openai-main": { models: ["gpt-5"] } },
} satisfies ModelCache;

test("provider editor opens, submits edited fields, and closes", async () => {
  const user = userEvent.setup();
  const onSave = vi.fn<(draft: ProviderDraft) => void>();
  const onClose = vi.fn<() => void>();

  render(ProviderEditor, {
    props: {
      provider,
      models: [],
      onSave,
      onDelete: vi.fn(),
      onRefreshModels: vi.fn(),
      onCheck: vi.fn(),
      onClose,
    },
  });

  const dialog = await screen.findByRole("dialog", {
    name: "Edit OpenAI Main",
  });
  expect(dialog.hasAttribute("open")).toBe(true);
  const name = screen.getByLabelText("Display name");
  await user.clear(name);
  await user.type(name, "Primary OpenAI");
  await user.click(screen.getByRole("button", { name: "Save profile" }));
  expect(onSave).toHaveBeenCalledWith({
    id: "openai-main",
    name: "Primary OpenAI",
    type: "openai",
    url: "https://api.openai.com/v1",
    key: "",
    timeout_seconds: "30",
  });

  await user.click(screen.getByRole("button", { name: "Close editor" }));
  expect(onClose).toHaveBeenCalledOnce();
});

test("dreaming editor submits the toggled schedule state", async () => {
  const user = userEvent.setup();
  const onSave = vi.fn<(settings: DreamSettings) => void>();
  render(DreamingEditor, {
    props: { initial: dream, providers: [provider], modelCache, onSave },
  });

  await user.click(
    screen.getByRole("checkbox", { name: "Enable scheduled dreaming" }),
  );
  await user.click(screen.getByRole("button", { name: "Save dreaming" }));

  expect(onSave).toHaveBeenCalledWith(
    expect.objectContaining({
      dreaming: expect.objectContaining({ enabled: true }),
    }),
  );
});
