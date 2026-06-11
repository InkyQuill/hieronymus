import React, { useEffect, useRef, useState } from "react";
import { Box, Text, useApp, useInput, useStdin } from "ink";
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
  const { exit } = useApp();
  const { stdin, isRawModeSupported } = useStdin();
  const [snapshot, setSnapshot] = useState(initial.snapshot);
  const [stats, setStats] = useState(initial.stats);
  const [shortTermStatus, setShortTermStatus] = useState(
    initial.short_term_status,
  );
  const [dreamStatus, setDreamStatus] = useState(initial.dream_status);
  const [configEditor, setConfigEditor] = useState(initial.config_editor);
  const [commandsOpen, setCommandsOpen] = useState(showCommands);
  const [status, setStatus] = useState<Status>({
    message: serviceStatus(initial.service.running),
    error: false,
  });
  const operationInFlight = useRef(false);
  const canUseInkInput = Boolean(
    isRawModeSupported &&
    typeof stdin.ref === "function" &&
    typeof stdin.unref === "function",
  );

  const handleInput = (input: string, ctrl = false) => {
    if (input === "q") {
      client?.close?.();
      exit();
      return;
    }

    if (ctrl && input === "p") {
      setCommandsOpen((open) => !open);
      return;
    }

    const viewIndex = viewIndexForInput(input, initial.views.length);
    if (viewIndex >= 0) {
      if (!client || operationInFlight.current) {
        return;
      }
      const view = initial.views[viewIndex];
      if (view) {
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
      }
      return;
    }

    if (input === "f") {
      setStatus({ message: "Filter command selected", error: false });
      return;
    }

    if (input === "e") {
      setStatus({ message: "Edit command selected", error: false });
      return;
    }

    if (!client || operationInFlight.current) {
      return;
    }

    const selectedId = snapshot.selected?.id;
    if (selectedId === undefined || !crystalMutationViews.has(snapshot.view)) {
      return;
    }

    if (input === "d") {
      void runSnapshotOperation({
        client,
        method: "admin.delete_crystal",
        params: { id: selectedId, view: snapshot.view, confirmed: true },
        successMessage: "Deleted crystal",
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

  useInput(
    (input, key) => {
      handleInput(input, key.ctrl);
    },
    { isActive: canUseInkInput },
  );

  useEffect(() => {
    if (canUseInkInput) {
      return undefined;
    }

    const onData = (chunk: Buffer | string) => {
      const text = String(chunk);
      if (text === "\u0010") {
        handleInput("p", true);
        return;
      }
      handleInput(text[0] ?? "");
    };

    stdin.on("data", onData);
    return () => {
      stdin.off("data", onData);
    };
  }, [canUseInkInput, handleInput, stdin]);

  const selectedViewIndex = Math.max(initial.views.indexOf(snapshot.view), 0);
  const viewKeyLimit = Math.min(initial.views.length, 9);

  return (
    <Box flexDirection="column">
      <Header header={initial.header} />
      <Box marginTop={1}>
        <Box flexDirection="column" width={28}>
          <Text bold>Views</Text>
          <FocusableList
            items={initial.views}
            selectedIndex={selectedViewIndex}
            label={(view) => view}
          />
        </Box>
        <Box flexDirection="column">
          <Text>{formatStats(stats)}</Text>
          <StatusPanels
            shortTermStatus={shortTermStatus}
            dreamStatus={dreamStatus}
          />
          <ConfigSummary configEditor={configEditor} />
          <Box marginTop={1}>
            <AdminTable
              rows={snapshot.rows}
              selectedId={snapshot.selected?.id ?? null}
            />
            <DetailPane detail={snapshot.detail} />
            {commandsOpen ? <CommandPalette view={snapshot.view} /> : null}
          </Box>
        </Box>
      </Box>
      <StatusLine message={status.message} error={status.error} />
      <KeyHelp
        keys={[
          `1-${viewKeyLimit} view`,
          "ctrl+p commands",
          "+ reinforce",
          "- decay",
          "d delete",
          "f filter",
          "e edit",
          "q quit",
        ]}
      />
    </Box>
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
  client: RpcClient;
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
    <Box flexDirection="column">
      <Text bold>
        {header.logo.text} {header.product} Admin {header.version}
      </Text>
      <Text dimColor>{header.tagline}</Text>
    </Box>
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
    <Box flexDirection="column" marginTop={1}>
      <Text>
        Short-term pending {shortTermStatus.pending_count} / min{" "}
        {shortTermStatus.min_pending_short_term_memories} / max{" "}
        {shortTermStatus.max_pending_short_term_memories}
        {shortTermStatus.urgent ? " urgent" : ""}
        {drain}
      </Text>
      <Text>{dream}</Text>
    </Box>
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
    <Box flexDirection="column" marginTop={1}>
      <Text>
        Config providers {providerNames.join(", ") || "none"} workflows{" "}
        {workflowNames.join(", ") || "none"}
      </Text>
      <Text>
        Prompts {promptNames.join(", ") || "none"} thresholds{" "}
        {thresholdNames.length}
        {"  "}model cache warnings {warnings.length}
      </Text>
      {warnings[0] ? <Text color="yellow">{warnings[0].message}</Text> : null}
    </Box>
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
