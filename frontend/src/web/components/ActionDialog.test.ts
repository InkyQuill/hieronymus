import { render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import type { AdminCommand, AdminRow } from "../lib/types";
import ActionDialog from "./ActionDialog.svelte";

function command(
  id: string,
  label: string,
  requires_selection = true,
): AdminCommand {
  return {
    id,
    label,
    hint: `${label} hint`,
    key: "",
    group: "Memory",
    views: ["Crystals"],
    requires_selection,
  };
}

const row: AdminRow = {
  id: 7,
  kind: "crystal",
  label: "Hero name",
  status: "active",
  scope: "series:s1",
  language_pair: "ja -> en",
  quality_label: "strong",
  tags: [],
};

test("add_memory gathers series and text and posts the typed body", async () => {
  const user = userEvent.setup();
  const onSubmit = vi.fn();
  render(ActionDialog, {
    props: {
      command: command("add_memory", "Add Memory", false),
      view: "Crystals",
      row: null,
      onSubmit,
      onClose: vi.fn(),
    },
  });

  // Accessible: the first field is labelled and focused.
  const series = screen.getByLabelText("Series slug");
  await waitFor(() => expect(document.activeElement).toBe(series));
  await user.type(series, "my-series");
  await user.type(
    screen.getByLabelText("Memory text"),
    "The moon has two names.",
  );
  await user.click(screen.getByRole("button", { name: "Add Memory" }));

  expect(onSubmit).toHaveBeenCalledWith({
    view: "Crystals",
    series: "my-series",
    text: "The moon has two names.",
    title: "",
  });
});

test("edit_memory prefills the current text and posts the edit", async () => {
  const user = userEvent.setup();
  const onSubmit = vi.fn();
  render(ActionDialog, {
    props: {
      command: command("edit_memory", "Edit Memory"),
      view: "Crystals",
      row,
      currentText: "The hero is Alto.",
      onSubmit,
      onClose: vi.fn(),
    },
  });

  const text = screen.getByLabelText("Memory text");
  expect((text as HTMLTextAreaElement).value).toBe("The hero is Alto.");
  await user.clear(text);
  await user.type(text, "The hero is Alto Verren.");
  await user.click(screen.getByRole("button", { name: "Edit Memory" }));

  // The title field was not touched, so no `title` is sent — the backend
  // keeps the stored title.
  expect(onSubmit).toHaveBeenCalledWith({
    view: "Crystals",
    id: 7,
    text: "The hero is Alto Verren.",
  });
});

test("edit_memory never rewrites the title of an untitled crystal when it is untouched", async () => {
  const user = userEvent.setup();
  const onSubmit = vi.fn();
  // An untitled crystal: `label` is an excerpt of its own text.
  const untitled: AdminRow = {
    ...row,
    label: "The hero is Alto and lives in…",
  };
  render(ActionDialog, {
    props: {
      command: command("edit_memory", "Edit Memory"),
      view: "Crystals",
      row: untitled,
      currentText: "The hero is Alto and lives in Verel.",
      onSubmit,
      onClose: vi.fn(),
    },
  });

  await user.type(screen.getByLabelText("Memory text"), " He is twenty.");
  await user.click(screen.getByRole("button", { name: "Edit Memory" }));

  const body = onSubmit.mock.calls[0][0];
  expect(body).not.toHaveProperty("title");
  expect(body.text).toContain("He is twenty.");
});

test("edit_memory sends the title once the user edits it", async () => {
  const user = userEvent.setup();
  const onSubmit = vi.fn();
  render(ActionDialog, {
    props: {
      command: command("edit_memory", "Edit Memory"),
      view: "Crystals",
      row,
      currentText: "The hero is Alto.",
      onSubmit,
      onClose: vi.fn(),
    },
  });

  await user.clear(screen.getByLabelText("Title"));
  await user.type(screen.getByLabelText("Title"), "Protagonist");
  await user.click(screen.getByRole("button", { name: "Edit Memory" }));

  expect(onSubmit).toHaveBeenCalledWith({
    view: "Crystals",
    id: 7,
    text: "The hero is Alto.",
    title: "Protagonist",
  });
});

test("edit_memory does not send the title after a type-then-revert", async () => {
  const user = userEvent.setup();
  const onSubmit = vi.fn();
  const untitled: AdminRow = {
    ...row,
    label: "The hero is Alto and lives in…",
  };
  render(ActionDialog, {
    props: {
      command: command("edit_memory", "Edit Memory"),
      view: "Crystals",
      row: untitled,
      currentText: "The hero is Alto and lives in Verel.",
      onSubmit,
      onClose: vi.fn(),
    },
  });

  const titleField = screen.getByLabelText("Title");
  await user.type(titleField, " typo");
  await user.clear(titleField);
  await user.type(titleField, "The hero is Alto and lives in…");
  await user.click(screen.getByRole("button", { name: "Edit Memory" }));

  expect(onSubmit.mock.calls[0][0]).not.toHaveProperty("title");
});

test("split_crystal keeps part text bound after removing a middle part", async () => {
  const user = userEvent.setup();
  const onSubmit = vi.fn();
  render(ActionDialog, {
    props: {
      command: command("split_crystal", "Split Crystal"),
      view: "Crystals",
      row,
      onSubmit,
      onClose: vi.fn(),
    },
  });

  await user.click(screen.getByRole("button", { name: "Add another part" }));
  await user.type(screen.getByLabelText("Part 1 text"), "first");
  await user.type(screen.getByLabelText("Part 2 text"), "middle");
  await user.type(screen.getByLabelText("Part 3 text"), "last");
  // Remove the middle part — the remaining two must keep their own text.
  await user.click(screen.getByRole("button", { name: "Remove part 2" }));
  await user.click(
    screen.getByLabelText(/apply this change to the stored memory/i),
  );
  await user.click(screen.getByRole("button", { name: "Split Crystal" }));

  expect(onSubmit).toHaveBeenCalledWith({
    view: "Crystals",
    id: 7,
    confirmed: true,
    parts: ["first", "last"],
  });
});

test("split_crystal collects 2+ parts and requires an explicit confirm", async () => {
  const user = userEvent.setup();
  const onSubmit = vi.fn();
  render(ActionDialog, {
    props: {
      command: command("split_crystal", "Split Crystal"),
      view: "Crystals",
      row,
      onSubmit,
      onClose: vi.fn(),
    },
  });

  const submit = screen.getByRole("button", { name: "Split Crystal" });
  expect((submit as HTMLButtonElement).disabled).toBe(true);

  await user.type(screen.getByLabelText("Part 1 text"), "The city is Verel.");
  await user.type(
    screen.getByLabelText("Part 2 text"),
    "Verel sits on the river.",
  );
  await user.click(screen.getByRole("button", { name: "Add another part" }));
  await user.type(
    screen.getByLabelText("Part 3 text"),
    "The river is the Kess.",
  );

  expect((submit as HTMLButtonElement).disabled).toBe(true);
  await user.click(
    screen.getByLabelText(/apply this change to the stored memory/i),
  );
  expect((submit as HTMLButtonElement).disabled).toBe(false);
  await user.click(submit);

  expect(onSubmit).toHaveBeenCalledWith({
    view: "Crystals",
    id: 7,
    confirmed: true,
    parts: [
      "The city is Verel.",
      "Verel sits on the river.",
      "The river is the Kess.",
    ],
  });
});

test("reject_proposal requires a reason before it will post", async () => {
  const user = userEvent.setup();
  const onSubmit = vi.fn();
  render(ActionDialog, {
    props: {
      command: command("reject_proposal", "Reject Proposal"),
      view: "Proposals",
      row: { ...row, id: 3, kind: "strict concept" },
      onSubmit,
      onClose: vi.fn(),
    },
  });

  const submit = screen.getByRole("button", { name: "Reject Proposal" });
  expect((submit as HTMLButtonElement).disabled).toBe(true);
  await user.type(
    screen.getByLabelText(/Reason/),
    "duplicate of an existing concept",
  );
  await user.click(submit);

  expect(onSubmit).toHaveBeenCalledWith({
    view: "Proposals",
    id: 3,
    reason: "duplicate of an existing concept",
  });
});

test("a cancelled confirmation posts nothing", async () => {
  const user = userEvent.setup();
  const onSubmit = vi.fn();
  const onClose = vi.fn();
  render(ActionDialog, {
    props: {
      command: command("delete_selected", "Delete Selected"),
      view: "Crystals",
      row,
      selectedIds: [7],
      onSubmit,
      onClose,
    },
  });

  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onSubmit).not.toHaveBeenCalled();
  expect(onClose).toHaveBeenCalled();
});

test("merge_selected needs at least two ids and merged text", async () => {
  const user = userEvent.setup();
  const onSubmit = vi.fn();
  render(ActionDialog, {
    props: {
      command: command("merge_selected", "Merge Selected"),
      view: "Crystals",
      row,
      selectedIds: [7, 8],
      onSubmit,
      onClose: vi.fn(),
    },
  });

  await user.type(
    screen.getByLabelText("Merged memory text"),
    "The hero is Alto; the city is Verel.",
  );
  await user.click(
    screen.getByLabelText(/apply this change to the stored memory/i),
  );
  await user.click(screen.getByRole("button", { name: "Merge Selected" }));

  expect(onSubmit).toHaveBeenCalledWith({
    view: "Crystals",
    ids: [7, 8],
    text: "The hero is Alto; the city is Verel.",
    title: "",
    confirmed: true,
  });
});
