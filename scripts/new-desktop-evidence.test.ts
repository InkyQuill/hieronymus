import { expect, test } from "bun:test";
import { observationTemplate } from "./new-desktop-evidence";
import { requiredChecks } from "./check-desktop-evidence";
test("native template never invents observations or capture hashes", () => {
  const r = observationTemplate(
    {
      candidate_commit: "a".repeat(40),
      target: "x86_64-unknown-linux-gnu",
      artifact_sha256: "a".repeat(64),
      model_sha256: "b".repeat(64),
      metadata_sha256: "c".repeat(64),
      assets_sha256: "d".repeat(64),
      signing: "unsigned-waiver",
    },
    "kde",
    "wayland",
    "kde-wayland",
  );
  expect(r.checks.map((c) => c.name)).toEqual(requiredChecks(r.target));
  expect(
    r.checks.every(
      (c) => c.result === "unavailable" && c.evidence_sha256 === null,
    ),
  ).toBe(true);
  expect(r.scale).toEqual([1]);
});
