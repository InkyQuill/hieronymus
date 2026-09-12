import { expect, test } from "bun:test";
import { numericRun, validateRun } from "./desktop-ci";
const good = {
  repository: { full_name: "owner/repo" },
  head_repository: { full_name: "owner/repo" },
  path: ".github/workflows/desktop-candidate.yml",
  event: "workflow_dispatch",
  status: "completed",
  conclusion: "success",
  head_sha: "a".repeat(40),
};
test("run provenance binds exact successful workflow and source", () => {
  expect(
    validateRun(good, "owner/repo", "desktop-candidate", "a".repeat(40)),
  ).toBe("a".repeat(40));
  for (const patch of [
    { conclusion: "failure" },
    { path: ".github/workflows/other.yml" },
    { head_sha: "b".repeat(40) },
    { event: "pull_request" },
    { head_repository: { full_name: "attacker/repo" } },
  ])
    expect(() =>
      validateRun(
        { ...good, ...patch },
        "owner/repo",
        "desktop-candidate",
        "a".repeat(40),
      ),
    ).toThrow();
});
test("run identifiers cannot become options or paths", () => {
  expect(numericRun("123")).toBe("123");
  for (const s of ["-1", "../1", "1;echo", "", "0"])
    expect(() => numericRun(s)).toThrow();
});

import {
  mkdtempSync,
  mkdirSync,
  writeFileSync,
  readdirSync,
  copyFileSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { inventory, verifyCandidate } from "./desktop-ci";
import {
  TARGETS,
  desktopTarget,
  platformArtifactName,
  modelArtifactName,
  MODEL_NAME,
  MODEL_REVISION,
  modelMembers,
} from "./desktop-targets";

test("candidate inventory binds all four exact payloads and rejects extra or changed attachments", async () => {
  const root = mkdtempSync(join(tmpdir(), "candidate-inventory-"));
  const merged = join(root, "merged");
  mkdirSync(merged);
  const hash = (s: string) => createHash("sha256").update(s).digest("hex");
  try {
    for (const target of TARGETS) {
      const directory = join(root, target);
      mkdirSync(directory);
      const metadata = {
        format_version: 2,
        version: "0.9.0",
        target,
        channel: "stable",
        signature: null,
        platform: {
          archive: platformArtifactName("0.9.0", target),
          sha256: hash(target),
        },
        model: {
          name: MODEL_NAME,
          revision: MODEL_REVISION,
          archive: modelArtifactName(),
          sha256: hash("shared model"),
          members: modelMembers(),
        },
      };
      const files: Record<string, string> = {
        [desktopTarget(target).metadata]: JSON.stringify(metadata),
        [metadata.platform.archive]: target,
        [metadata.model.archive]: "shared model",
        [`evidence-${target}.json`]: JSON.stringify({ symbols: [] }),
        [`assets-${target}.json`]: "assets",
        [`install-desktop-${target}.${target.includes("windows") ? "ps1" : "sh"}`]:
          "installer",
      };
      if (!target.includes("windows")) {
        files["install.sh"] = "shared installer";
        files["desktop-metadata.awk"] = "shared decoder";
      }
      for (const [name, content] of Object.entries(files))
        writeFileSync(join(directory, name), content);
      await inventory(directory, target, "a".repeat(40));
      for (const name of readdirSync(directory))
        copyFileSync(join(directory, name), join(merged, name));
    }
    expect(
      (await verifyCandidate(merged, "a".repeat(40))).length,
    ).toBeGreaterThan(20);
    await expect(verifyCandidate(merged, "b".repeat(40))).rejects.toThrow(
      "source/target",
    );
    const unknown = join(merged, "unverified.exe");
    writeFileSync(unknown, "surprise");
    await expect(verifyCandidate(merged, "a".repeat(40))).rejects.toThrow(
      "unknown candidate attachment",
    );
    rmSync(unknown);
    writeFileSync(join(merged, "install.sh"), "replaced installer");
    await expect(verifyCandidate(merged, "a".repeat(40))).rejects.toThrow(
      "digest mismatch",
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
