import React, { useEffect, useRef, useState } from "react";
import { useKeyboard, useRenderer } from "@opentui/react";
import { z } from "zod";
import {
  AdminConfigEditorSchema,
  AdminDreamStatusSchema,
  AdminShortTermStatusSchema,
  AdminSnapshotSchema,
  type AdminBootstrap,
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
import { AdminTable } from "./AdminTable.js";
import { DetailPane } from "./DetailPane.js";
import { CommandPalette } from "./CommandPalette.js";
import { Spinner } from "../ui/Spinner.js";
import {
  type DialogState,
  closedDialog,
  DialogOverlay,
} from "./dialogs.js";

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
  const [snapshot, setSnapshot] = useState(initial.snapshot);
  const [stats, setStats] = useState(initial.stats);
  const [shortTermStatus, setShortTermStatus] = useState(
    initial.short_term_status,
  );
  const [dreamStatus, setDreamStatus] = useState(initial.dream_status);
  const [configEditor, setConfigEditor] = useState(initial.config_editor);
  const [commandsOpen, setCommandsOpen] = useState(showCommands);
  const [activePanel, setActivePanel] = useState<"views" | "table" | "detail">(
    "views",
  );
  const [dialog, setDialog] = useState<DialogState>(closedDialog);
  const [status, setStatus] = useState<Status>({
    message: serviceStatus(initial.service.running),
    error: false,
  });
  const operationInFlight = useRef(false);

  const selectedViewIndex = Math.max(initial.views.indexOf(snapshot.view), 0);
  const viewKeyLimit = Math.min(initial.views.length, 9);

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

  const handleInput = (input: string, ctrl = false) => {
    if (input === "q") {
      client?.close?.();
      renderer.destroy();
    }

    if (ctrl && input === "p") {
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
        setStatus({ message: "Split only supported for crystals/lessons", error: true });
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
      } else if (snapshot.view === "Short-Term Sessions") {
        setDialog({
          kind: "delete",
          error: "",
          entityId: selectedId,
          entityType: "memory",
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

    const ctrl = key.ctrl;
    const tab = key.name === "tab";
    const shift = key.shift;
    const upArrow = key.name === "up";
    const downArrow = key.name === "down";

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
      if (upArrow) {
        const nextIndex = Math.max(0, selectedViewIndex - 1);
        const view = initial.views[nextIndex];
        if (view && view !== snapshot.view) {
          loadView(view);
        }
        return;
      }
      if (downArrow) {
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
    }

    if (activePanel === "table") {
      if (upArrow) {
        if (operationInFlight.current) {
          return;
        }
        const currentIndex = snapshot.rows.findIndex(
          (r) => r.id === snapshot.selected?.id,
        );
        if (currentIndex > 0) {
          const prevRow = snapshot.rows[currentIndex - 1];
          if (prevRow) {
            void runSnapshotOperation({
              client,
              method: "admin.snapshot",
              params: {
                view: snapshot.view,
                selected_id: prevRow.id,
                filters: snapshot.filters,
              },
              successMessage: `Selected ${prevRow.label}`,
              setSnapshot,
              setStats,
              setShortTermStatus,
              setDreamStatus,
              setConfigEditor,
              setStatus,
              operationInFlight,
            });
          }
        }
        return;
      }
      if (downArrow) {
        if (operationInFlight.current) {
          return;
        }
        const currentIndex = snapshot.rows.findIndex(
          (r) => r.id === snapshot.selected?.id,
        );
        if (currentIndex >= 0 && currentIndex < snapshot.rows.length - 1) {
          const nextRow = snapshot.rows[currentIndex + 1];
          if (nextRow) {
            void runSnapshotOperation({
              client,
              method: "admin.snapshot",
              params: {
                view: snapshot.view,
                selected_id: nextRow.id,
                filters: snapshot.filters,
              },
              successMessage: `Selected ${nextRow.label}`,
              setSnapshot,
              setStats,
              setShortTermStatus,
              setDreamStatus,
              setConfigEditor,
              setStatus,
              operationInFlight,
            });
          }
        }
        return;
      }
    }

    // 3. Fallback to hotkeys/input handling
    handleInput(key.name, ctrl);
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

  if (dialog.kind !== "none") {
    return (
      <box flexDirection="column" width={136} height={20} alignItems="center" justifyContent="center">
        <DialogOverlay
          state={dialog}
          onClose={() => setDialog(closedDialog)}
          onSubmit={handleDialogSubmit}
        />
      </box>
    );
  }

  return (
    <box flexDirection="column" width={136}>
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
          <text fg={activePanel === "views" ? "cyan" : undefined}>
            Views
          </text>
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
          <DetailPane detail={snapshot.detail} />
          {commandsOpen ? <CommandPalette view={snapshot.view} /> : null}
        </box>
      </box>

      <StatusLine message={status.message} error={status.error} />
      <KeyHelp
        keys={[
          "Tab focus",
          `1-${viewKeyLimit} view`,
          "+/- reinforce/decay",
          "a add",
          "e edit",
          "d delete",
          "m merge",
          "s split",
          "f filter",
          "q quit",
        ]}
      />
    </box>
  );
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
}) {
  if (!client) {
    return;
  }
  operationInFlight.current = true;
  setStatus({ message: `Working: ${successMessage}`, error: false });
  try {
    const response = await client.request(method, params);
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
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    setStatus({ message, error: true });
  } finally {
    operationInFlight.current = false;
  }
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
      <box flexDirection="row" marginTop={0}>
        {dreamStatus.state !== "idle" && dreamStatus.state !== "DISABLED" && (
          <box marginRight={1}>
            <Spinner />
          </box>
        )}
        <text>{dream}</text>
      </box>
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
