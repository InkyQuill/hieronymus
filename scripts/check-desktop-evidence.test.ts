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
