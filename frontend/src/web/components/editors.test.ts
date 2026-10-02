import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import type {
  DreamSettings,
  ModelCache,
  ProviderDraft,
  ProviderProfile,
} from "../lib/types";
import { refreshModels } from "../lib/api";
vi.mock("../lib/api", () => ({
  refreshModels: vi.fn(async () => ["gpt-5", "gpt-5-mini"]),
}));
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
  await user.click(screen.getByText("Advanced model limits"));
  await user.type(screen.getByLabelText("Context window (tokens)"), "16384");
  await user.click(screen.getByRole("button", { name: "Save profile" }));
  expect(onSave).toHaveBeenCalledWith({
    id: "openai-main",
    name: "Primary OpenAI",
    type: "openai",
    url: "https://api.openai.com/v1",
    key: "",
    timeout_seconds: "30",
    context_window: "16384",
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

test("dreaming loads API models and saves a custom model", async () => {
  const user = userEvent.setup();
  const onSave = vi.fn();
  render(DreamingEditor, {
    props: {
      initial: dream,
      providers: [provider],
      modelCache: { providers: {} },
      onSave,
    },
  });
  expect(
    await screen.findByRole("option", { name: "gpt-5-mini" }),
  ).toBeTruthy();
  expect(refreshModels).toHaveBeenCalledWith(provider.id);
  await user.selectOptions(screen.getByLabelText("Model"), "gpt-5-mini");
  await user.click(screen.getByRole("button", { name: "Save dreaming" }));
  expect(onSave.mock.calls.at(-1)?.[0].workflows.concepts.model).toBe(
    "gpt-5-mini",
  );
  await user.selectOptions(screen.getByLabelText("Model"), "__custom__");
  await user.clear(screen.getByLabelText("Custom model"));
  await user.type(screen.getByLabelText("Custom model"), "private-model");
  await user.click(screen.getByRole("button", { name: "Save dreaming" }));
  expect(onSave.mock.calls.at(-1)?.[0].workflows.concepts.model).toBe(
    "private-model",
  );
});

test("dreaming preserves a saved custom model when discovery fails", async () => {
  vi.mocked(refreshModels).mockRejectedValueOnce(new Error("offline"));
  const initial = structuredClone(dream);
  initial.workflows.concepts.model = "private-model";
  render(DreamingEditor, {
    props: {
      initial,
      providers: [provider],
      modelCache: { providers: {} },
      onSave: vi.fn(),
    },
  });
  expect(await screen.findByText(/Could not load models/)).toBeTruthy();
  expect(
    (screen.getByLabelText("Custom model") as HTMLInputElement).value,
  ).toBe("private-model");
});

test("dreaming task prompts can be edited and restored without changing shared instructions", async () => {
  const user = userEvent.setup();
  const onSave = vi.fn();
  render(DreamingEditor, {
    props: {
      initial: dream,
      providers: [provider],
      modelCache,
      defaultPrompts: { concepts: "Extract supported concepts." },
      onSave,
    },
  });
  await user.click(screen.getByText("Task prompt · Default"));
  const task = screen.getByLabelText("concepts task prompt");
  expect((task as HTMLTextAreaElement).value).toBe(
    "Extract supported concepts.",
  );
  await user.clear(task);
  await user.type(task, "Extract character motivations.");
  await user.click(screen.getByRole("button", { name: "Save dreaming" }));
  expect(onSave.mock.calls.at(-1)?.[0].workflows.concepts.prompt).toBe(
    "Extract character motivations.",
  );
  expect(onSave.mock.calls.at(-1)?.[0].dreaming.general_prompt).toBe(
    dream.dreaming.general_prompt,
  );
  await user.click(
    screen.getByRole("button", { name: "Restore default prompt" }),
  );
  expect((task as HTMLTextAreaElement).value).toBe(
    "Extract supported concepts.",
  );
  await user.click(screen.getByRole("button", { name: "Save dreaming" }));
  expect(onSave.mock.calls.at(-1)?.[0].workflows.concepts.prompt).toBe("");
});

test("provider deletion requires a separate confirmation and can be cancelled", async () => {
  const user = userEvent.setup();
  const onDelete = vi.fn();
  render(ProviderEditor, {
    props: {
      provider,
      onSave: vi.fn(),
      onDelete,
      onRefreshModels: vi.fn(),
      onCheck: vi.fn(),
      onClose: vi.fn(),
    },
  });
  await user.click(screen.getByRole("button", { name: "Delete provider" }));
  expect(onDelete).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Keep provider" }));
  expect(screen.queryByRole("button", { name: "Confirm deletion" })).toBeNull();
  await user.click(screen.getByRole("button", { name: "Delete provider" }));
  await user.click(screen.getByRole("button", { name: "Confirm deletion" }));
  expect(onDelete).toHaveBeenCalledOnce();
});
