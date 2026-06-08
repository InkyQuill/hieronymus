import React, { useEffect, useRef, useState } from "react";
import { Box, Text, useApp, useInput, useStdin } from "ink";
import { z } from "zod";
import {
  AdminSnapshotSchema,
  type AdminBootstrap,
  type AdminSnapshot,
} from "../rpc/schema.js";
import type { JsonRpcClient } from "../rpc/client.js";
import { KeyHelp } from "../ui/KeyHelp.js";
import { StatusLine } from "../ui/StatusLine.js";
import { FocusableList } from "../ui/FocusableList.js";
import { AdminTable } from "./AdminTable.js";
import { DetailPane } from "./DetailPane.js";
import { CommandPalette } from "./CommandPalette.js";

type Props = {
  initial: AdminBootstrap;
  client: JsonRpcClient | undefined;
  showCommands?: boolean;
};

type Status = {
  message: string;
  error: boolean;
};

const viewKeys = ["1", "2", "3", "4", "5", "6", "7", "8"] as const;
const crystalMutationViews = new Set(["Crystals", "Lessons"]);
const AdminOperationResponseSchema = z
  .object({
    stats: z.record(z.number()).optional(),
    snapshot: AdminSnapshotSchema,
  })
  .passthrough();

export function AdminScreen({ initial, client, showCommands = false }: Props) {
  const { exit } = useApp();
  const { stdin, isRawModeSupported } = useStdin();
  const [snapshot, setSnapshot] = useState(initial.snapshot);
  const [stats, setStats] = useState(initial.stats);
  const [commandsOpen, setCommandsOpen] = useState(showCommands);
  const [status, setStatus] = useState<Status>({
    message: serviceStatus(initial.service.running),
    error: false,
  });
  const operationInFlight = useRef(false);
  const canUseInkInput = Boolean(
    client &&
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

    if (!client) {
      return;
    }

    if (operationInFlight.current) {
      return;
    }

    const viewIndex = viewKeys.indexOf(input as (typeof viewKeys)[number]);
    if (viewIndex >= 0) {
      const view = initial.views[viewIndex];
      if (view) {
        void runSnapshotOperation({
          client,
          method: "admin.snapshot",
          params: { view },
          successMessage: `Loaded ${view}`,
          setSnapshot,
          setStats,
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
    if (!client || canUseInkInput) {
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
  }, [canUseInkInput, client, handleInput, stdin]);

  const selectedViewIndex = Math.max(initial.views.indexOf(snapshot.view), 0);

  return (
    <Box flexDirection="column">
      <Text bold>Hieronymus Admin</Text>
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
          "1-8 view",
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

async function runSnapshotOperation({
  client,
  method,
  params,
  successMessage,
  setSnapshot,
  setStats,
  setStatus,
  operationInFlight,
}: {
  client: JsonRpcClient;
  method: string;
  params: Record<string, unknown>;
  successMessage: string;
  setSnapshot: (snapshot: AdminSnapshot) => void;
  setStats: (stats: Record<string, number>) => void;
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
    setStatus({ message: successMessage, error: false });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    setStatus({ message, error: true });
  } finally {
    operationInFlight.current = false;
  }
}

function formatStats(stats: Record<string, number>) {
  return Object.entries(stats)
    .map(([name, value]) => `${name.replaceAll("_", " ")} ${value}`)
    .join("  ");
}

function serviceStatus(running: boolean) {
  return running ? "Service running" : "Service stopped";
}
