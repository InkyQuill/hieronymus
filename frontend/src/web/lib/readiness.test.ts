import { render, screen } from "@testing-library/svelte";
import { describe, expect, test } from "vitest";
import AdminDashboard from "../components/AdminDashboard.svelte";
import { formatReadiness } from "./readiness";
import type { AdminDashboard as AdminDashboardPayload } from "./types";

const dashboard = (readiness?: unknown): AdminDashboardPayload => ({
  header: {
    product: "Hieronymus",
    version: "v0.8.0",
    tagline: "Translation memory",
  },
  stats: {},
  views: [],
  short_term_status: {},
  dream_status: {},
  ...(readiness === undefined ? {} : { readiness }),
});

describe("formatReadiness", () => {
  test.each([
    ["missing", undefined],
    ["null", null],
    ["unknown level", { level: "excellent", reasons: [], providers: [] }],
    ["missing reasons", { level: "ready", providers: [] }],
    ["malformed reasons", { level: "ready", reasons: [{}], providers: [] }],
    [
      "unsupported schema marker",
      { schema_version: 2, level: "ready", reasons: [], providers: [] },
    ],
  ])("classifies %s readiness as unknown", (_name, value) => {
    expect(formatReadiness(value)).toEqual({
      level: "Unknown",
      reasons: ["Readiness is not available from this daemon."],
      providers: [],
    });
  });

  test("preserves an untested provider as explicit server-owned state", () => {
    expect(
      formatReadiness({
        level: "ready",
        reasons: [],
        providers: [
          {
            capabilities: ["coverage_audit"],
            provider: "primary",
            model: "translator-v1",
            revision: 4,
            condition: "untested",
            observed_at: null,
            reason: "External provider not yet verified",
          },
        ],
      }),
    ).toEqual({
      level: "Ready",
      reasons: [],
      providers: [
        {
          provider: "primary",
          model: "translator-v1",
          condition: "No recent calls",
          reason: "No calls observed since the server started or provider settings changed. Previous Dreaming runs remain in processing history.",
        },
      ],
    });
  });
});

test("dashboard displays the server's exact degraded semantic reason", () => {
  render(AdminDashboard, {
    props: {
      dashboard: dashboard({
        level: "degraded",
        reasons: ["Semantic index rebuilding"],
        providers: [],
      }),
      onDream: () => {},
    },
  });

  expect(screen.getByText("Degraded")).toBeTruthy();
  expect(screen.getByText("Semantic index rebuilding")).toBeTruthy();
  expect(screen.queryByText("Ready")).toBeNull();
});

test("dashboard renders only known readiness fields", () => {
  render(AdminDashboard, {
    props: {
      dashboard: dashboard({
        level: "ready",
        reasons: [],
        providers: [],
        api_key: "SENTINEL-SECRET",
        diagnostics: { raw_response: "SENTINEL-RAW-BODY" },
      }),
      onDream: () => {},
    },
  });

  expect(screen.getByText("Ready")).toBeTruthy();
  expect(screen.queryByText(/SENTINEL/)).toBeNull();
});

test("memory indexing shows live progress and failures independently of service readiness", async () => {
  const value = dashboard({ level: "ready", reasons: [], providers: [] });
  value.memory_indexing = { state: "indexing", total: 100, indexed: 32, pending: 68, detail: null };
  const view = render(AdminDashboard, { props: { dashboard: value, onDream: () => {} } });
  expect(screen.getByText("Indexing in the background")).toBeTruthy();
  expect(screen.getByText(/32 \/ 100 memories indexed/)).toBeTruthy();
  expect((screen.getByRole("progressbar") as HTMLProgressElement).value).toBe(32);
  await view.rerender({ dashboard: { ...value, memory_indexing: { ...value.memory_indexing, state: "failed", detail: "Model unavailable" } }, onDream: () => {} });
  expect(screen.getByRole("alert").textContent).toBe("Model unavailable");
});
