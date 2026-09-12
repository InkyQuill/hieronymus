import { afterEach, expect, test } from "bun:test";
import { mkdtempSync, writeFileSync, rmSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { requiredChecks, validateRecord } from "./check-desktop-evidence";
const roots: string[] = [];
afterEach(() =>
  roots
    .splice(0)
    .forEach((root) => rmSync(root, { recursive: true, force: true })),
);
function fixture() {
  const root = mkdtempSync(join(tmpdir(), "desktop-evidence-"));
  roots.push(root);
  const sha = (s: string) => createHash("sha256").update(s).digest("hex");
  writeFileSync(join(root, "capture.txt"), "observed native behavior");
  const identity = {
    candidate_commit: "a".repeat(40),
    target: "x86_64-unknown-linux-gnu",
    artifact_sha256: sha("platform"),
    model_sha256: sha("model"),
    metadata_sha256: sha("metadata"),
    assets_sha256: sha("assets"),
    signing: "unsigned-waiver",
  };
  const record = {
    ...identity,
    os_version: "Linux test fixture",
    desktop: "kde",
    session: "wayland",
    scale: [1, 2],
    checks: requiredChecks(identity.target).map((name) => ({
      name,
      result: "pass",
      evidence_path: "capture.txt",
      evidence_sha256: sha("observed native behavior"),
    })),
  };
  return { root, identity, record };
}
test("fully populated native record passes", async () => {
  const f = fixture();
  await validateRecord(f.record, f.root, f.identity);
});
test("present null qualification is invalid", async () => {
  const f = fixture();
  await expect(
    validateRecord({ ...f.record, qualification: null }, f.root, f.identity),
  ).rejects.toThrow("invalid qualification status");
});
test("Intel desktop qualification cannot claim qualified or partial coverage", async () => {
  const f = fixture();
  const identity = { ...f.identity, target: "x86_64-apple-darwin" };
  for (const qualification of ["qualified", "partial"]) {
    await expect(
      validateRecord(
        { ...f.record, ...identity, qualification },
        f.root,
        identity,
      ),
    ).rejects.toThrow("Intel macOS must remain unqualified");
  }
});
test("partial coverage keeps verified passes and records explicit gaps", async () => {
  const f = fixture();
  f.record.checks[0].result = "unavailable";
  await validateRecord(
    {
      ...f.record,
      scale: [1],
      qualification: "partial",
      qualification_reason:
        "Only the available KDE session was tested at standard scale.",
    },
    f.root,
    f.identity,
  );
});
test("unqualified sessions never claim native passes", async () => {
  const f = fixture();
  const record = {
    ...f.record,
    scale: [],
    qualification: "unqualified",
    qualification_reason:
      "No Intel Mac is available for native desktop testing.",
  };
  await expect(validateRecord(record, f.root, f.identity)).rejects.toThrow(
    "cannot claim",
  );
  record.checks.forEach((check) => (check.result = "unavailable"));
  await validateRecord(record, f.root, f.identity);
  record.checks[0].result = "fail";
  await expect(validateRecord(record, f.root, f.identity)).rejects.toThrow(
    "must pass",
  );
});
test("limited coverage still requires exact evidence and a reason", async () => {
  const f = fixture();
  f.record.checks[0].result = "unavailable";
  const record = {
    ...f.record,
    qualification: "partial",
    qualification_reason: "",
  };
  await expect(validateRecord(record, f.root, f.identity)).rejects.toThrow(
    "concrete reason",
  );
  record.qualification_reason =
    "The high DPI display was not available for this test.";
  writeFileSync(join(f.root, "capture.txt"), "changed");
  await expect(validateRecord(record, f.root, f.identity)).rejects.toThrow(
    "digest",
  );
});
test("absent interactive-menu evidence fails", async () => {
  const f = fixture();
  f.record.checks = f.record.checks.filter(
    (c) => c.name !== "interactive-menu",
  );
  await expect(validateRecord(f.record, f.root, f.identity)).rejects.toThrow(
    "interactive-menu",
  );
});
test("changed artifact hash fails", async () => {
  const f = fixture();
  f.record.artifact_sha256 = "f".repeat(64);
  await expect(validateRecord(f.record, f.root, f.identity)).rejects.toThrow(
    "artifact_sha256",
  );
});
test("unavailable is not pass", async () => {
  const f = fixture();
  f.record.checks[0].result = "unavailable";
  await expect(validateRecord(f.record, f.root, f.identity)).rejects.toThrow(
    "must pass",
  );
});
test("failure is not pass", async () => {
  const f = fixture();
  f.record.checks[0].result = "fail";
  await expect(validateRecord(f.record, f.root, f.identity)).rejects.toThrow(
    "must pass",
  );
});
test("missing or changed evidence fails", async () => {
  const f = fixture();
  writeFileSync(join(f.root, "capture.txt"), "changed");
  await expect(validateRecord(f.record, f.root, f.identity)).rejects.toThrow(
    "digest",
  );
});
test("duplicate check cannot conceal missing coverage", async () => {
  const f = fixture();
  f.record.checks[1] = f.record.checks[0];
  await expect(validateRecord(f.record, f.root, f.identity)).rejects.toThrow(
    "duplicate",
  );
});
test("evidence cannot escape through a path or symlink", async () => {
  const f = fixture();
  f.record.checks[0].evidence_path = "../capture.txt";
  await expect(validateRecord(f.record, f.root, f.identity)).rejects.toThrow(
    "path",
  );
  symlinkSync(join(f.root, "capture.txt"), join(f.root, "alias"));
  f.record.checks[0].evidence_path = "alias";
  await expect(validateRecord(f.record, f.root, f.identity)).rejects.toThrow(
    "regular",
  );
});
test("candidate commit and DPI coverage are required", async () => {
  const f = fixture();
  f.record.candidate_commit = "b".repeat(40);
  await expect(validateRecord(f.record, f.root, f.identity)).rejects.toThrow(
    "candidate_commit",
  );
  f.record.candidate_commit = f.identity.candidate_commit;
  f.record.scale = [1];
  await expect(validateRecord(f.record, f.root, f.identity)).rejects.toThrow(
    "scale",
  );
});
test("unknown checks are rejected", async () => {
  const f = fixture();
  f.record.checks.push({ ...f.record.checks[0], name: "invented-pass" });
  await expect(validateRecord(f.record, f.root, f.identity)).rejects.toThrow(
    "unknown check",
  );
});
test("missing file is rejected", async () => {
  const f = fixture();
  f.record.checks[0].evidence_path = "missing.log";
  await expect(validateRecord(f.record, f.root, f.identity)).rejects.toThrow();
});
