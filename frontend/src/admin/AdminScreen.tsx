import React, { useEffect, useRef, useState } from "react";
import {
  useKeyboard,
  useRenderer,
  useTerminalDimensions,
} from "@opentui/react";
import { z } from "zod";
import {
  AdminConfigEditorSchema,
  AdminDreamStatusSchema,
  AdminShortTermStatusSchema,
  AdminSnapshotSchema,
  type AdminBootstrap,
  type AdminCommand,
  type AdminConfigEditor,
  type AdminDreamStatus,
  type AdminHeader,
  type AdminSnapshot,
  type AdminShortTermStatus,
} from "../rpc/schema.js";
import type { RpcClient } from "../rpc/client.js";
import { KeyHelp } from "../ui/KeyHelp.js";
import { StatusLine } from "../ui/StatusLine.js";
import { FocusableList } from "../ui/FocusableList.js";
import {
  classifyTerminalLayout,
  MIN_TERMINAL_HEIGHT,
  MIN_TERMINAL_WIDTH,
  panelHeight,
  panelWidth,
} from "../ui/responsive.js";
import { AdminTable } from "./AdminTable.js";
import { DetailPane } from "./DetailPane.js";
import { CommandPalette, commandsForView } from "./CommandPalette.js";
import { HelpOverlay } from "./HelpOverlay.js";
import { Spinner } from "../ui/Spinner.js";
import { Gauge } from "../ui/Gauge.js";
import { type DialogState, closedDialog, DialogOverlay } from "./dialogs.js";
import {
  isConfirmKey,
  isDownKey,
  isEscapeKey,
  isLeftKey,
  isRightKey,
  isUpKey,
  printableSearchChar,
  type KeyboardInput,
} from "../ui/keyboard.js";

type Props = {
  initial: AdminBootstrap;
  client: RpcClient | undefined;
  showCommands?: boolean;
};

type Status = {
  message: string;
  error: boolean;
};

const crystalMutationViews = new Set(["Crystals", "Lessons"]);
const AdminOperationResponseSchema = z
  .object({
    stats: z.record(z.number()).optional(),
    snapshot: AdminSnapshotSchema,
    short_term_status: AdminShortTermStatusSchema.optional(),
    dream_status: AdminDreamStatusSchema.optional(),
    config_editor: AdminConfigEditorSchema.optional(),
  })
  .passthrough();

export function AdminScreen({ initial, client, showCommands = false }: Props) {
  const renderer = useRenderer();
  const dimensions = useTerminalDimensions();
  const layout = classifyTerminalLayout(dimensions.width, dimensions.height);
  const contentWidth = panelWidth(layout);
  const [snapshot, setSnapshot] = useState<AdminSnapshot>(initial.snapshot);
  const [stats, setStats] = useState(initial.stats);
  const [shortTermStatus, setShortTermStatus] = useState(
    initial.short_term_status,
  );
  const [dreamStatus, setDreamStatus] = useState(initial.dream_status);
  const [configEditor, setConfigEditor] = useState(initial.config_editor);
  const [commandsOpen, setCommandsOpen] = useState(showCommands);
  const [helpOpen, setHelpOpen] = useState(false);
  const [searchActive, setSearchActive] = useState(false);
  const [searchText, setSearchText] = useState("");
  const [activePanel, setActivePanel] = useState<"views" | "table" | "detail">(
    "views",
  );
  const [dialog, setDialog] = useState<DialogState>(closedDialog);
  const [status, setStatus] = useState<Status>({
    message: serviceStatus(initial.service.running),
    error: false,
  });
  const operationInFlight = useRef(false);

  useEffect(() => {
    if (layout.kind === "too-small" && dialog.kind !== "none") {
      setDialog(closedDialog);
    }
  }, [dialog.kind, layout.kind]);

  const selectedViewIndex = Math.max(initial.views.indexOf(snapshot.view), 0);
  const viewKeyLimit = Math.min(initial.views.length, 9);
  const paletteCommands = commandsForView(
    initial.command_options,
    snapshot.view,
    Boolean(snapshot.selected),
  );
  const [selectedCommandIndex, setSelectedCommandIndex] = useState(0);
  const clampCommandIndex = (index: number) =>
    Math.min(Math.max(index, 0), Math.max(paletteCommands.length - 1, 0));

  const selectRow = (
    row: AdminSnapshot["rows"][number],
    successMessage = `Selected ${row.label}`,
  ) => {
    if (!client || operationInFlight.current) {
      return;
    }
    void runSnapshotOperation({
      client,
      method: "admin.snapshot",
      params: {
        view: snapshot.view,
        selected_id: row.id,
      },
      successMessage,
      setSnapshot,
      setStats,
      setShortTermStatus,
      setDreamStatus,
      setConfigEditor,
      setStatus,
      operationInFlight,
    });
  };

  const loadView = (view: string) => {
    if (!client || operationInFlight.current) {
      return;
    }
    void runSnapshotOperation({
      client,
      method: "admin.snapshot",
      params: { view },
      successMessage: `Loaded ${view}`,
      setSnapshot,
      setStats,
      setShortTermStatus,
      setDreamStatus,
      setConfigEditor,
      setStatus,
      operationInFlight,
    });
  };

  const runSnapshotCommand = (
    method: string,
    params: Record<string, unknown>,
    successMessage: string,
    detailFromResponse?: (
      response: Record<string, unknown>,
    ) => AdminSnapshot["detail"] | undefined,
  ) => {
    if (!client || operationInFlight.current) {
      return;
    }
    void runSnapshotOperation({
      client,
      method,
      params: { ...params, view: snapshot.view },
      successMessage,
      setSnapshot,
      setStats,
      setShortTermStatus,
      setDreamStatus,
      setConfigEditor,
      setStatus,
      operationInFlight,
      detailFromResponse,
    });
  };

  const runSelectedSnapshotCommand = (
    method: string,
    successMessage: string,
    detailFromResponse?: (
      response: Record<string, unknown>,
    ) => AdminSnapshot["detail"] | undefined,
  ) => {
    const selectedId = snapshot.selected?.id;
    if (selectedId === undefined) {
      setStatus({ message: "No row selected", error: true });
      return;
    }
    runSnapshotCommand(
      method,
      { id: selectedId },
      successMessage,
      detailFromResponse,
    );
  };

  const runInspectionCommand = (method: string, successMessage: string) => {
    const selectedId = snapshot.selected?.id;
    if (!client || operationInFlight.current || selectedId === undefined) {
      return;
    }
    operationInFlight.current = true;
    setStatus({ message: `Working: ${successMessage}`, error: false });
    void client
      .request(method, { id: selectedId })
      .then((response) => {
        setSnapshot({
          ...snapshot,
          detail: inspectionDetail(method, response),
        });
        setStatus({ message: successMessage, error: false });
      })
      .catch((error) => {
        setStatus({
          message: error instanceof Error ? error.message : String(error),
          error: true,
        });
      })
      .finally(() => {
        operationInFlight.current = false;
      });
  };

  const executeCommand = (
    command: (AdminCommand & { disabled?: boolean }) | undefined,
  ) => {
    if (!command) {
      return;
    }
    if (
      command.disabled ||
      (command.requires_selection && !snapshot.selected)
    ) {
      setStatus({
        message: `${command.label} needs a selected row`,
        error: true,
      });
      return;
    }
    setCommandsOpen(false);
    if (command.id === "add_memory") {
      setDialog({ kind: "add", error: "" });
      return;
    }
    if (command.id === "edit_memory") {
      handleInput("e");
      return;
    }
    if (command.id === "delete_selected") {
      handleInput("d");
      return;
    }
    if (command.id === "merge_selected") {
      handleInput("m");
      return;
    }
    if (command.id === "split_crystal") {
      handleInput("s");
      return;
    }
    if (command.id === "reinforce_crystal") {
      handleInput("+");
      return;
    }
    if (command.id === "decay_crystal") {
      handleInput("-");
      return;
    }
    if (command.id === "approve_proposal") {
      runSelectedSnapshotCommand("admin.approve_proposal", "Approved proposal");
      return;
    }
    if (command.id === "reject_proposal") {
      runSelectedSnapshotCommand("admin.reject_proposal", "Rejected proposal");
      return;
    }
    if (command.id === "run_manual_dreaming") {
      runSnapshotCommand(
        "admin.run_manual_dreaming",
        {},
        "Ran manual dreaming",
      );
      return;
    }
    if (command.id === "review_dream_output") {
      runSelectedSnapshotCommand(
        "admin.dream_review",
        "Loaded dream review",
        dreamReviewDetail,
      );
      return;
    }
    if (command.id === "inspect_provenance") {
      runInspectionCommand("admin.provenance", "Loaded provenance");
      return;
    }
    if (command.id === "inspect_recall_reasons") {
      runInspectionCommand("admin.recall_reasons", "Loaded recall reasons");
    }
  };

  const cancelSearch = () => {
    setSearchActive(false);
    setSearchText("");
  };

  const submitSearch = () => {
    const query = searchText.trim();
    if (!query) {
      setStatus({ message: "Search query is empty", error: true });
      return;
    }

    const match = snapshot.rows.find((row) => rowMatchesQuery(row, query));
    if (!match) {
      setStatus({
        message: `No ${snapshot.view} row matches "${query}"`,
        error: true,
      });
      return;
    }

    setSearchActive(false);
    setSearchText("");
    if (!client) {
      setStatus({ message: "Search needs admin client", error: true });
      return;
    }
    selectRow(match, `Found ${match.label}`);
  };

  const handleInput = (input: string, ctrl = false) => {
    if (input === "q") {
      client?.close?.();
      renderer.destroy();
    }

    if (ctrl && input === "p") {
      setSelectedCommandIndex(0);
      setHelpOpen(false);
      setCommandsOpen((open) => !open);
      return;
    }

    const viewIndex = viewIndexForInput(input, initial.views.length);
    if (viewIndex >= 0) {
      const view = initial.views[viewIndex];
      if (view) {
        loadView(view);
      }
      return;
    }

    if (input === "f") {
      setStatus({ message: "Filter command selected", error: false });
      return;
    }

    // Modal dialog triggers
    if (input === "a") {
      setDialog({ kind: "add", error: "" });
      return;
    }

    if (input === "e") {
      const selected = snapshot.selected;
      if (selected) {
        if (snapshot.view === "Concepts") {
          setDialog({
            kind: "rename",
            error: "",
            entityId: selected.id,
            entityType: "concept",
            initialTitle: selected.label,
          });
        } else if (crystalMutationViews.has(snapshot.view)) {
          setDialog({
            kind: "edit",
            error: "",
            entityId: selected.id,
            entityType: "crystal",
            initialTitle: selected.label,
            initialText: snapshot.detail.body,
          });
        } else {
          setStatus({
            message: "Edit not supported for this view",
            error: true,
          });
        }
      } else {
        setStatus({ message: "No row selected to edit", error: true });
      }
      return;
    }

    if (input === "m") {
      const selected = snapshot.selected;
      if (selected) {
        if (snapshot.view === "Concepts" || snapshot.view === "Crystals") {
          setDialog({
            kind: "merge",
            error: "",
            entityId: selected.id,
            entityType: snapshot.view === "Concepts" ? "concept" : "crystal",
          });
        } else {
          setStatus({
            message: "Merge only supported for Concepts or Crystals",
            error: true,
          });
        }
      } else {
        setStatus({ message: "No row selected to merge", error: true });
      }
      return;
    }

    if (input === "s") {
      const selected = snapshot.selected;
      if (selected && crystalMutationViews.has(snapshot.view)) {
        setDialog({
          kind: "split",
          error: "",
          entityId: selected.id,
          entityType: "crystal",
        });
      } else {
        setStatus({
          message: "Split only supported for crystals/lessons",
          error: true,
        });
      }
      return;
    }

    if (!client || operationInFlight.current) {
      return;
    }

    const selectedId = snapshot.selected?.id;
    if (selectedId === undefined) {
      return;
    }

    if (input === "d") {
      if (snapshot.view === "Concepts") {
        setDialog({
          kind: "delete",
          error: "",
          entityId: selectedId,
          entityType: "concept",
        });
      } else if (crystalMutationViews.has(snapshot.view)) {
        setDialog({
          kind: "delete",
          error: "",
          entityId: selectedId,
          entityType: "crystal",
        });
      } else {
        setStatus({
          message: "Delete not supported for this view",
          error: true,
        });
      }
      return;
    }

    if (!crystalMutationViews.has(snapshot.view)) {
      return;
    }

    if (input === "+") {
      void runSnapshotOperation({
        client,
        method: "admin.reinforce_crystal",
        params: { id: selectedId, view: snapshot.view },
        successMessage: "Reinforced crystal",
        setSnapshot,
        setStats,
        setShortTermStatus,
        setDreamStatus,
        setConfigEditor,
        setStatus,
        operationInFlight,
      });
      return;
    }

    if (input === "-") {
      void runSnapshotOperation({
        client,
        method: "admin.decay_crystal",
        params: { id: selectedId, view: snapshot.view },
        successMessage: "Decayed crystal",
        setSnapshot,
        setStats,
        setShortTermStatus,
        setDreamStatus,
        setConfigEditor,
        setStatus,
        operationInFlight,
      });
    }
  };

  useKeyboard((key) => {
    // Only handle keyboard shortcuts if dialog is NOT open
    if (dialog.kind !== "none") {
      return;
    }

    if (layout.kind === "too-small") {
      if (key.name === "q") {
        client?.close?.();
        renderer.destroy();
      }
      return;
    }

    const keyboardKey = key as KeyboardInput;
    const ctrl = keyboardKey.ctrl;
    const tab = keyboardKey.name === "tab";
    const shift = keyboardKey.shift;
    const up = isUpKey(keyboardKey);
    const down = isDownKey(keyboardKey);
    const left = isLeftKey(keyboardKey);
    const right = isRightKey(keyboardKey);
    const enter = isConfirmKey(keyboardKey);
    const escape = isEscapeKey(keyboardKey);

    if (searchActive) {
      if (escape) {
        cancelSearch();
        return;
      }
      if (enter) {
        submitSearch();
        return;
      }
      if (keyboardKey.name === "backspace") {
        setSearchText((current) => current.slice(0, -1));
        return;
      }

      const char = printableSearchChar(keyboardKey);
      if (char !== null) {
        setSearchText((current) => current + char);
      }
      return;
    }

    if (helpOpen) {
      if (ctrl && keyboardKey.name === "p") {
        setSelectedCommandIndex(0);
        setHelpOpen(false);
        setCommandsOpen(true);
        return;
      }
      if (escape || keyboardKey.name === "?") {
        setHelpOpen(false);
      }
      return;
    }

    if (keyboardKey.name === "?") {
      setHelpOpen(true);
      setCommandsOpen(false);
      return;
    }

    if (commandsOpen) {
      if (ctrl && keyboardKey.name === "p") {
        setCommandsOpen(false);
        return;
      }
      if (escape) {
        setCommandsOpen(false);
        return;
      }
      if (down) {
        setSelectedCommandIndex((index) => clampCommandIndex(index + 1));
        return;
      }
      if (up) {
        setSelectedCommandIndex((index) => clampCommandIndex(index - 1));
        return;
      }
      if (enter) {
        const command =
          paletteCommands[clampCommandIndex(selectedCommandIndex)];
        executeCommand(command);
        return;
      }
      return;
    }

    // 1. Focus Cycling
    if (tab) {
      setActivePanel((current) => {
        if (shift) {
          if (current === "views") return "detail";
          if (current === "table") return "views";
          return "table";
        } else {
          if (current === "views") return "table";
          if (current === "table") return "detail";
          return "views";
        }
      });
      return;
    }

    // 2. Panel Navigation
    if (activePanel === "views") {
      if (up) {
        const nextIndex = Math.max(0, selectedViewIndex - 1);
        const view = initial.views[nextIndex];
        if (view && view !== snapshot.view) {
          loadView(view);
        }
        return;
      }
      if (down) {
        const nextIndex = Math.min(
          initial.views.length - 1,
          selectedViewIndex + 1,
        );
        const view = initial.views[nextIndex];
        if (view && view !== snapshot.view) {
          loadView(view);
        }
        return;
      }
      if (right) {
        setActivePanel("table");
        return;
      }
    }

    if (activePanel === "table") {
      if (up) {
        if (operationInFlight.current) {
          return;
        }
        const currentIndex = snapshot.rows.findIndex(
          (r) => r.id === snapshot.selected?.id,
        );
        if (currentIndex > 0) {
          const prevRow = snapshot.rows[currentIndex - 1];
          if (prevRow) {
            selectRow(prevRow);
          }
        }
        return;
      }
      if (down) {
        if (operationInFlight.current) {
          return;
        }
        const currentIndex = snapshot.rows.findIndex(
          (r) => r.id === snapshot.selected?.id,
        );
        if (currentIndex >= 0 && currentIndex < snapshot.rows.length - 1) {
          const nextRow = snapshot.rows[currentIndex + 1];
          if (nextRow) {
            selectRow(nextRow);
          }
        }
        return;
      }
      if (left) {
        setActivePanel("views");
        return;
      }
      if (right) {
        setActivePanel("detail");
        return;
      }
    }

    if (activePanel === "detail") {
      if (left) {
        setActivePanel("table");
        return;
      }
    }

    // 3. Fallback to hotkeys/input handling
    if (keyboardKey.name === "/") {
      setSearchActive(true);
      setSearchText("");
      return;
    }

    const command = commandForKey(
      paletteCommands,
      keyboardKey,
      Boolean(snapshot.selected),
    );
    if (command) {
      executeCommand(command);
      return;
    }

    handleInput(keyboardKey.name, ctrl);
  });

  async function handleDialogSubmit(params: Record<string, any>) {
    if (!client || operationInFlight.current) {
      return;
    }
    let method = "";
    let successMessage = "";
    const view = snapshot.view;

    if (dialog.kind === "delete") {
      if (dialog.entityType === "concept") {
        method = "admin.archive_concept";
        successMessage = "Archived concept";
      } else if (dialog.entityType === "memory") {
        method = "admin.remove_short_term_memory";
        successMessage = "Removed short-term memory";
      } else {
        method = "admin.delete_crystal";
        successMessage = "Deleted crystal";
      }
    } else if (dialog.kind === "add") {
      method = "admin.add_crystal";
      successMessage = "Added crystal";
    } else if (dialog.kind === "edit") {
      method = "admin.edit_crystal";
      successMessage = "Edited memory";
    } else if (dialog.kind === "rename") {
      method = "admin.rename_concept";
      successMessage = "Renamed concept";
    } else if (dialog.kind === "merge") {
      if (dialog.entityType === "concept") {
        method = "admin.merge_concepts";
        successMessage = "Merged concepts";
      } else {
        method = "admin.merge_crystals";
        successMessage = "Merged crystals";
      }
    } else if (dialog.kind === "split") {
      method = "admin.split_crystal";
      successMessage = "Split crystal";
    }

    if (!method) return;

    operationInFlight.current = true;
    setStatus({ message: `Working: ${successMessage}`, error: false });

    try {
      const response = await client.request(method, {
        ...params,
        view,
      });
      const next = AdminOperationResponseSchema.parse(response);
      setSnapshot(next.snapshot);
      if (next.stats) {
        setStats(next.stats);
      }
      if (next.short_term_status) {
        setShortTermStatus(next.short_term_status);
      }
      if (next.dream_status) {
        setDreamStatus(next.dream_status);
      }
      if (next.config_editor) {
        setConfigEditor(next.config_editor);
      }
      setStatus({ message: successMessage, error: false });
      setDialog(closedDialog);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setDialog((prev) => ({ ...prev, error: message }));
      setStatus({ message, error: true });
    } finally {
      operationInFlight.current = false;
    }
  }

  if (layout.kind === "too-small") {
    return (
      <box flexDirection="column" width={dimensions.width}>
        <text>Terminal too small</text>
        <text fg="gray">
          {dimensions.width}x{dimensions.height}; minimum {MIN_TERMINAL_WIDTH}x
          {MIN_TERMINAL_HEIGHT}
        </text>
        <text fg="gray">Resize terminal to use Hieronymus admin.</text>
      </box>
    );
  }

  if (layout.kind !== "wide") {
    const compactPaneHeight = panelHeight(layout, 11);
    const compactScrollWidth = Math.max(20, contentWidth - 2);
    const compactTableWidth = Math.min(48, compactScrollWidth);
    const compactDetailWidth = Math.min(56, compactScrollWidth);
    const compactScrollHeight = Math.max(4, compactPaneHeight - 2);

    return (
      <box
        flexDirection="column"
        width={dimensions.width}
        height={dimensions.height}
      >
        <text>
          {initial.header.logo.text} {initial.header.product} Admin{" "}
          {initial.header.version}
        </text>
        <text fg="gray">
          {snapshot.view} · {layout.kind} {dimensions.width}x{dimensions.height}
        </text>
        <text>{formatStats(stats)}</text>
        <text>
          Short-term pending {shortTermStatus.pending_count} · Dream{" "}
          {dreamStatus.state}
        </text>

        <box
          flexDirection="column"
          marginTop={1}
          height={compactPaneHeight}
          borderStyle="rounded"
          borderColor="cyan"
          title={
            helpOpen || commandsOpen
              ? compactPanelLabel(activePanel, snapshot.view)
              : undefined
          }
          paddingX={1}
        >
          {helpOpen ? (
            <>
              <HelpOverlay
                commands={initial.command_options}
                view={snapshot.view}
                width={contentWidth}
              />
            </>
          ) : commandsOpen ? (
            <>
              <CommandPalette
                commands={paletteCommands}
                selectedIndex={clampCommandIndex(selectedCommandIndex)}
                width={contentWidth}
              />
            </>
          ) : activePanel === "views" ? (
            <>
              <text fg="cyan">Views</text>
              <FocusableList
                items={initial.views}
                selectedIndex={selectedViewIndex}
                label={(view) => view}
                focused
              />
            </>
          ) : activePanel === "table" ? (
            <>
              <text fg="cyan">{snapshot.view}</text>
              <box marginTop={1}>
                <AdminTable
                  rows={snapshot.rows}
                  selectedId={snapshot.selected?.id ?? null}
                  focused
                  width={compactTableWidth}
                  height={Math.max(4, compactScrollHeight - 1)}
                />
              </box>
            </>
          ) : (
            <>
              <text fg="cyan">Detail Inspector</text>
              <box marginTop={1}>
                <DetailPane
                  detail={snapshot.detail}
                  width={compactDetailWidth}
                  height={Math.max(4, compactScrollHeight - 1)}
                />
              </box>
            </>
          )}
        </box>

        <SearchPrompt active={searchActive} query={searchText} />
        <box flexDirection="row" marginTop={1}>
          <text fg="gray">
            {footerText(
              footerKeys({
                commandsOpen,
                helpOpen,
                searchActive,
                viewKeyLimit,
              }),
            )}
          </text>
        </box>
        <StatusLine message={status.message} error={status.error} />
        <DialogOverlay
          state={dialog}
          onClose={() => setDialog(closedDialog)}
          onSubmit={handleDialogSubmit}
        />
      </box>
    );
  }

  return (
    <box
      flexDirection="column"
      width={Math.min(136, dimensions.width)}
      height={dimensions.height}
    >
      <box
        flexDirection="column"
        borderStyle="rounded"
        borderColor="gray"
        paddingX={1}
        paddingY={1}
      >
        <Header header={initial.header} />
        <text>{formatStats(stats)}</text>
        <StatusPanels
          shortTermStatus={shortTermStatus}
          dreamStatus={dreamStatus}
        />
        <ConfigSummary configEditor={configEditor} />
      </box>

      <box flexDirection="row" marginTop={1} height={24}>
        {/* Left pane: Views */}
        <box
          flexDirection="column"
          width={28}
          borderStyle="rounded"
          borderColor={activePanel === "views" ? "cyan" : "gray"}
          paddingX={1}
        >
          <text fg={activePanel === "views" ? "cyan" : undefined}>Views</text>
          <FocusableList
            items={initial.views}
            selectedIndex={selectedViewIndex}
            label={(view) => view}
            focused={activePanel === "views"}
          />
        </box>

        {/* Middle pane: Data Table */}
        <box
          flexDirection="column"
          width={50}
          borderStyle="rounded"
          borderColor={activePanel === "table" ? "cyan" : "gray"}
          paddingX={1}
        >
          <text fg={activePanel === "table" ? "cyan" : undefined}>
            {snapshot.view}
          </text>
          <AdminTable
            rows={snapshot.rows}
            selectedId={snapshot.selected?.id ?? null}
            focused={activePanel === "table"}
          />
        </box>

        {/* Right pane: Detail Inspector */}
        <box
          flexDirection="column"
          width={58}
          borderStyle="rounded"
          borderColor={activePanel === "detail" ? "cyan" : "gray"}
          paddingX={1}
        >
          <text fg={activePanel === "detail" ? "cyan" : undefined}>
            Detail Inspector
          </text>
          {helpOpen ? (
            <HelpOverlay
              commands={initial.command_options}
              view={snapshot.view}
            />
          ) : commandsOpen ? (
            <CommandPalette
              commands={paletteCommands}
              selectedIndex={clampCommandIndex(selectedCommandIndex)}
            />
          ) : (
            <DetailPane detail={snapshot.detail} />
          )}
        </box>
      </box>

      <SearchPrompt active={searchActive} query={searchText} />
      <StatusLine message={status.message} error={status.error} />
      <KeyHelp
        keys={footerKeys({
          commandsOpen,
          helpOpen,
          searchActive,
          viewKeyLimit,
        })}
      />
      <DialogOverlay
        state={dialog}
        onClose={() => setDialog(closedDialog)}
        onSubmit={handleDialogSubmit}
      />
    </box>
  );
}

function footerKeys({
  commandsOpen,
  helpOpen,
  searchActive,
  viewKeyLimit,
}: {
  commandsOpen: boolean;
  helpOpen: boolean;
  searchActive: boolean;
  viewKeyLimit: number;
}) {
  if (searchActive) {
    return ["Enter search", "Esc cancel", "Backspace edit"];
  }
  if (helpOpen) {
    return ["Esc close help", "? close help", "Ctrl+P commands"];
  }
  if (commandsOpen) {
    return ["↑/↓ or j/k move", "Enter run", "Esc close", "? help"];
  }
  return [
    "Tab pane",
    "/ search",
    `1-${viewKeyLimit} view`,
    "↑/↓ or hjkl move",
    "Ctrl+P commands",
    "? help",
    "q quit",
  ];
}

function commandForKey(
  commands: Array<AdminCommand & { disabled: boolean }>,
  key: KeyboardInput,
  hasSelection: boolean,
): (AdminCommand & { disabled: boolean }) | undefined {
  if (key.ctrl || key.meta || key.option) {
    return undefined;
  }
  return commands.find((command) => {
    if (command.disabled || (command.requires_selection && !hasSelection)) {
      return false;
    }
    return commandKeyMatches(command.key, key);
  });
}

function commandKeyMatches(commandKey: string, key: KeyboardInput): boolean {
  if (commandKey === "enter") {
    return isConfirmKey(key);
  }
  if (commandKey.length === 1) {
    if (
      key.name === commandKey ||
      key.raw === commandKey ||
      key.sequence === commandKey
    ) {
      return true;
    }
    if (commandKey.toLocaleLowerCase() !== key.name) {
      return false;
    }
    const expectsShift = commandKey !== commandKey.toLocaleLowerCase();
    return expectsShift ? Boolean(key.shift) : true;
  }
  return commandKey === key.name;
}

function footerText(keys: string[]): string {
  return keys.join(" ");
}

function SearchPrompt({ active, query }: { active: boolean; query: string }) {
  if (!active) {
    return null;
  }

  return (
    <box marginTop={1}>
      <text fg="cyan">Search: {query}</text>
    </box>
  );
}

function rowMatchesQuery(
  row: AdminSnapshot["rows"][number],
  query: string,
): boolean {
  const normalizedQuery = query.toLocaleLowerCase();
  return searchableValues(row).some((value) =>
    value.toLocaleLowerCase().includes(normalizedQuery),
  );
}

function searchableValues(value: unknown): string[] {
  if (typeof value === "string") {
    return [value];
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return [String(value)];
  }
  if (Array.isArray(value)) {
    return value.flatMap(searchableValues);
  }
  if (value && typeof value === "object") {
    return Object.values(value as Record<string, unknown>).flatMap(
      searchableValues,
    );
  }
  return [];
}

function viewIndexForInput(input: string, viewCount: number) {
  if (!/^[1-9]$/.test(input)) {
    return -1;
  }
  const index = Number(input) - 1;
  return index < Math.min(viewCount, 9) ? index : -1;
}

async function runSnapshotOperation({
  client,
  method,
  params,
  successMessage,
  setSnapshot,
  setStats,
  setShortTermStatus,
  setDreamStatus,
  setConfigEditor,
  setStatus,
  operationInFlight,
  detailFromResponse,
}: {
  client: RpcClient | undefined;
  method: string;
  params: Record<string, unknown>;
  successMessage: string;
  setSnapshot: (snapshot: AdminSnapshot) => void;
  setStats: (stats: Record<string, number>) => void;
  setShortTermStatus: (status: AdminShortTermStatus) => void;
  setDreamStatus: (status: AdminDreamStatus) => void;
  setConfigEditor: (configEditor: AdminConfigEditor) => void;
  setStatus: (status: Status) => void;
  operationInFlight: React.MutableRefObject<boolean>;
  detailFromResponse?: (
    response: Record<string, unknown>,
  ) => AdminSnapshot["detail"] | undefined;
}) {
  if (!client) {
    return;
  }
  operationInFlight.current = true;
  setStatus({ message: `Working: ${successMessage}`, error: false });
  try {
    const response = await client.request(method, params);
    const next = AdminOperationResponseSchema.parse(response);
    const detail = detailFromResponse?.(next);
    setSnapshot(detail ? { ...next.snapshot, detail } : next.snapshot);
    if (next.stats) {
      setStats(next.stats);
    }
    if (next.short_term_status) {
      setShortTermStatus(next.short_term_status);
    }
    if (next.dream_status) {
      setDreamStatus(next.dream_status);
    }
    if (next.config_editor) {
      setConfigEditor(next.config_editor);
    }
    setStatus({ message: successMessage, error: false });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    setStatus({ message, error: true });
  } finally {
    operationInFlight.current = false;
  }
}

function dreamReviewDetail(response: Record<string, unknown>) {
  return {
    title: "Dream Review",
    subtitle: "admin.dream_review",
    body: JSON.stringify(response.review ?? {}, null, 2),
    fields: [],
  };
}

function inspectionDetail(method: string, response: Record<string, unknown>) {
  if (method === "admin.provenance") {
    return {
      title: "Provenance",
      subtitle: "admin.provenance",
      body: JSON.stringify(response.provenance ?? {}, null, 2),
      fields: [],
    };
  }
  return {
    title: "Recall Reasons",
    subtitle: "admin.recall_reasons",
    body: JSON.stringify(response.reasons ?? [], null, 2),
    fields: [],
  };
}

function Header({ header }: { header: AdminHeader }) {
  return (
    <>
      <text>
        {header.logo.text} {header.product} Admin {header.version}
      </text>
      <text fg="gray">{header.tagline}</text>
    </>
  );
}

function StatusPanels({
  shortTermStatus,
  dreamStatus,
}: {
  shortTermStatus: AdminShortTermStatus;
  dreamStatus: AdminDreamStatus;
}) {
  const drain = shortTermStatus.drain_in_progress
    ? `  drain ${shortTermStatus.drain_completed}/${shortTermStatus.drain_total} (${formatPercent(
        shortTermStatus.drain_progress,
      )}) remaining ${shortTermStatus.drain_remaining}`
    : "";
  const dream = [
    `Dream ${dreamStatus.state}`,
    dreamStatus.current_phase ? `phase ${dreamStatus.current_phase}` : "",
    dreamStatus.progress > 0
      ? `progress ${formatPercent(dreamStatus.progress)}`
      : "",
    dreamStatus.run_id === null ? "" : `run ${dreamStatus.run_id}`,
    dreamStatus.cycle_id === null ? "" : `cycle ${dreamStatus.cycle_id}`,
  ]
    .filter(Boolean)
    .join("  ");

  return (
    <box flexDirection="column" marginTop={1}>
      <box flexDirection="row">
        {shortTermStatus.drain_in_progress && (
          <box marginRight={1}>
            <Spinner />
          </box>
        )}
        <text>
          Short-term pending {shortTermStatus.pending_count} / min{" "}
          {shortTermStatus.min_pending_short_term_memories} / max{" "}
          {shortTermStatus.max_pending_short_term_memories}
          {shortTermStatus.urgent ? " urgent" : ""}
          {drain}
        </text>
      </box>
      <Gauge
        label="Short-term"
        value={shortTermStatus.pending_count}
        max={shortTermStatus.max_pending_short_term_memories}
        fg={shortTermStatus.urgent ? "yellow" : "cyan"}
      />
      {shortTermStatus.drain_in_progress ? (
        <Gauge
          label="Drain"
          value={shortTermStatus.drain_completed}
          max={shortTermStatus.drain_total}
          fg="cyan"
        />
      ) : null}
      <box flexDirection="row" marginTop={0}>
        {dreamStatus.state !== "idle" && dreamStatus.state !== "DISABLED" && (
          <box marginRight={1}>
            <Spinner />
          </box>
        )}
        <text>{dream}</text>
      </box>
      {dreamStatus.progress > 0 ? (
        <Gauge
          label="Dream"
          value={Math.round(dreamStatus.progress * 100)}
          max={100}
          fg="cyan"
        />
      ) : null}
    </box>
  );
}

function ConfigSummary({ configEditor }: { configEditor: AdminConfigEditor }) {
  const providerNames = Object.keys(configEditor.providers);
  const workflowNames = Object.entries(configEditor.workflows).map(
    ([name, workflow]) => `${name}:${String(workflow.model ?? "")}`,
  );
  const promptNames = Object.keys(configEditor.prompts);
  const thresholdNames = Object.keys(configEditor.thresholds);
  const warnings = configEditor.model_cache_warnings;

  return (
    <box flexDirection="column" marginTop={1}>
      <text>
        Config providers {providerNames.join(", ") || "none"} workflows{" "}
        {workflowNames.join(", ") || "none"}
      </text>
      <text>
        Prompts {promptNames.join(", ") || "none"} thresholds{" "}
        {thresholdNames.length}
        {"  "}model cache warnings {warnings.length}
      </text>
      {warnings[0] ? <text fg="yellow">{warnings[0].message}</text> : null}
    </box>
  );
}

function formatStats(stats: Record<string, number>) {
  return Object.entries(stats)
    .map(([name, value]) => `${name.replaceAll("_", " ")} ${value}`)
    .join("  ");
}

function formatPercent(value: number) {
  return `${Math.round(value * 100)}%`;
}

function serviceStatus(running: boolean) {
  return running ? "Service running" : "Service stopped";
}

function compactPanelLabel(
  activePanel: "views" | "table" | "detail",
  view: string,
): string {
  if (activePanel === "views") {
    return "Views";
  }
  if (activePanel === "table") {
    return view;
  }
  return "Detail Inspector";
}
