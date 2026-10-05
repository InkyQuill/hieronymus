import { render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";
import type { AdminCommand, AdminRow } from "../lib/types";
import ActionDialog from "./ActionDialog.svelte";
import { prepareMergePreview } from "../lib/api";
vi.mock("../lib/api", () => ({ prepareMergePreview: vi.fn() }));
const prepareMergeMock = vi.mocked(prepareMergePreview);
beforeEach(() => {
  prepareMergeMock.mockReset().mockResolvedValue({
    title: "Combined memory",
    text: "Suggested combined memory.",
    source_snapshots: [],
  });
});

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

test("merge_selected prepares an editable proposal and commits only after confirmation", async () => {
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

  const field = await screen.findByLabelText("Merged memory text");
  expect((field as HTMLTextAreaElement).value).toBe(
    "Suggested combined memory.",
  );
  expect(prepareMergeMock).toHaveBeenCalledWith(
    "Crystals",
    [7, 8],
    expect.any(AbortSignal),
  );
  expect(onSubmit).not.toHaveBeenCalled();
  await user.clear(field);
  await user.type(field, "The hero is Alto; the city is Verel.");
  await user.click(
    screen.getByLabelText(/apply this change to the stored memory/i),
  );
  await user.click(screen.getByRole("button", { name: "Merge Selected" }));

  expect(onSubmit).toHaveBeenCalledWith({
    view: "Crystals",
    ids: [7, 8],
    text: "The hero is Alto; the city is Verel.",
    title: "Combined memory",
    confirmed: true,
    source_snapshots: [],
  });
});

test.each([[8], [8, 9]])(
  "case %# delete confirmation describes checked targets %j instead of the detail row",
  async (...ids: number[]) => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(ActionDialog, {
      props: {
        command: command("delete_selected", "Delete Selected"),
        view: "Crystals",
        row,
        selectedIds: ids,
        onSubmit,
        onClose: vi.fn(),
      },
    });
    expect(
      screen.getByText(
        `Deleting ${ids.length} ${ids.length === 1 ? "record" : "records"}.`,
      ),
    ).toBeTruthy();
    const targets = screen.getByRole("list", { name: "Records to delete" });
    expect(
      Array.from(targets.querySelectorAll("li"), (item) =>
        item.textContent?.trim(),
      ),
    ).toEqual(ids.map((id) => `Record ID: ${id}`));
    expect(targets.textContent).not.toContain(row.label);
    const submit = screen.getByRole("button", {
      name: "Delete Selected",
    }) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);
    await user.click(
      screen.getByLabelText(/apply this change to the stored memory/i),
    );
    await user.click(submit);
    expect(onSubmit).toHaveBeenCalledWith({
      view: "Crystals",
      ids,
      confirmed: true,
    });
  },
);

test("delete confirmation falls back to the detail record only without checked ids", async () => {
  const user = userEvent.setup();
  const onSubmit = vi.fn();
  render(ActionDialog, {
    props: {
      command: command("delete_selected", "Delete Selected"),
      view: "Crystals",
      row,
      onSubmit,
      onClose: vi.fn(),
    },
  });
  expect(screen.getByText("Deleting 1 record.")).toBeTruthy();
  expect(
    screen.getByRole("list", { name: "Records to delete" }).textContent,
  ).toContain("Hero name (ID: 7)");
  await user.click(
    screen.getByLabelText(/apply this change to the stored memory/i),
  );
  await user.click(screen.getByRole("button", { name: "Delete Selected" }));
  expect(onSubmit).toHaveBeenCalledWith({
    view: "Crystals",
    ids: [7],
    confirmed: true,
  });
});

test("failed merge preparation leaves originals untouched and can be retried", async () => {
  prepareMergeMock.mockRejectedValueOnce(
    new Error("Dreaming model unavailable"),
  );
  const onSubmit = vi.fn();
  const user = userEvent.setup();
  render(ActionDialog, {
    command: command("merge_selected", "Merge Selected"),
    view: "Crystals",
    selectedIds: [7, 8],
    onSubmit,
    onClose: vi.fn(),
  });
  expect((await screen.findByRole("alert")).textContent).toContain(
    "Dreaming model unavailable",
  );
  expect(screen.queryByLabelText("Merged memory text")).toBeNull();
  expect(onSubmit).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Try preparing again" }));
  expect(
    (
      (await screen.findByLabelText(
        "Merged memory text",
      )) as HTMLTextAreaElement
    ).value,
  ).toBe("Suggested combined memory.");
  expect(onSubmit).not.toHaveBeenCalled();
});

test("a regenerated merge suggestion requires a new confirmation", async () => {
  const user = userEvent.setup();
  render(ActionDialog, {
    command: command("merge_selected", "Merge Selected"),
    view: "Crystals",
    selectedIds: [7, 8],
    onSubmit: vi.fn(),
    onClose: vi.fn(),
  });
  await screen.findByLabelText("Merged memory text");
  const confirmation = screen.getByLabelText(
    /apply this change to the stored memory/i,
  ) as HTMLInputElement;
  await user.click(confirmation);
  let finish:
    | ((proposal: import("../lib/types").MergePreview) => void)
    | undefined;
  const pending = new Promise<import("../lib/types").MergePreview>(
    (resolve) => {
      finish = resolve;
    },
  );
  prepareMergeMock.mockReturnValueOnce(pending);
  await user.click(
    screen.getByRole("button", { name: "Prepare another suggestion" }),
  );
  expect(confirmation.disabled).toBe(true);
  expect(confirmation.checked).toBe(false);
  expect(
    (screen.getByLabelText("Merged title (optional)") as HTMLInputElement)
      .disabled,
  ).toBe(true);
  finish?.({
    title: "Revised suggestion",
    text: "Revised combined memory.",
    source_snapshots: [],
  });
  await waitFor(() =>
    expect(
      (screen.getByLabelText("Merged memory text") as HTMLTextAreaElement)
        .value,
    ).toBe("Revised combined memory."),
  );
  expect(confirmation.checked).toBe(false);
  expect(
    (
      screen.getByRole("button", {
        name: "Merge Selected",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(true);
});
