import type {
  ProviderCondition,
  ProviderReadiness,
  ReadinessLevel,
  ReadinessSummary,
} from "./types";

export type ReadinessView = {
  level: "Ready" | "Degraded" | "Starting" | "Unknown";
  reasons: string[];
  providers: Array<{
    provider: string;
    model: string;
    condition: "Untested" | "Healthy" | "Failed";
    reason: string | null;
  }>;
};

const UNKNOWN_READINESS: ReadinessView = {
  level: "Unknown",
  reasons: ["Readiness is not available from this daemon."],
  providers: [],
};

const levelLabels: Record<ReadinessLevel, ReadinessView["level"]> = {
  ready: "Ready",
  degraded: "Degraded",
  starting: "Starting",
};

const conditionLabels: Record<
  ProviderCondition,
  ReadinessView["providers"][number]["condition"]
> = {
  untested: "Untested",
  healthy: "Healthy",
  failed: "Failed",
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isStringArray(value: unknown): value is string[] {
  return (
    Array.isArray(value) && value.every((item) => typeof item === "string")
  );
}

function parseProvider(value: unknown): ProviderReadiness | null {
  if (!isRecord(value)) return null;
  if (
    typeof value.provider !== "string" ||
    typeof value.model !== "string" ||
    !Number.isSafeInteger(value.revision) ||
    (value.revision as number) < 0 ||
    !(
      value.condition === "untested" ||
      value.condition === "healthy" ||
      value.condition === "failed"
    ) ||
    !(value.observed_at === null || typeof value.observed_at === "string") ||
    !(value.reason === null || typeof value.reason === "string") ||
    !(value.capabilities === undefined || isStringArray(value.capabilities))
  ) {
    return null;
  }
  return {
    capabilities: value.capabilities,
    provider: value.provider,
    model: value.model,
    revision: value.revision as number,
    condition: value.condition,
    observed_at: value.observed_at,
    reason: value.reason,
  };
}

function parseSummary(value: unknown): ReadinessSummary | null {
  if (!isRecord(value) || "schema_version" in value) return null;
  if (
    !(
      value.level === "ready" ||
      value.level === "degraded" ||
      value.level === "starting"
    ) ||
    !isStringArray(value.reasons) ||
    !Array.isArray(value.providers)
  ) {
    return null;
  }
  const providers = value.providers.map(parseProvider);
  if (providers.some((provider) => provider === null)) return null;
  return {
    level: value.level,
    reasons: value.reasons,
    providers: providers as ProviderReadiness[],
  };
}

/**
 * Verifies the current unversioned daemon DTO and projects only its approved
 * display fields. Unknown shapes fail closed instead of inferring health from
 * dreaming history or arbitrary diagnostics.
 */
export function formatReadiness(value: unknown): ReadinessView {
  const summary = parseSummary(value);
  if (summary === null) return UNKNOWN_READINESS;
  return {
    level: levelLabels[summary.level],
    reasons: summary.reasons,
    providers: summary.providers.map((provider) => ({
      provider: provider.provider,
      model: provider.model,
      condition: conditionLabels[provider.condition],
      reason: provider.reason,
    })),
  };
}
