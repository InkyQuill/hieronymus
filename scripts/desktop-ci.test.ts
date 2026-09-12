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
  existsSync,
  readFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { inventory, verifyCandidate, downloadCandidates } from "./desktop-ci";
import {
  TARGETS,
  desktopTarget,
  platformArtifactName,
  modelArtifactName,
  MODEL_NAME,
  MODEL_REVISION,
  modelMembers,
  type OfficialRuntime,
} from "./desktop-targets";

test("candidate downloads isolate extraction and deduplicate identical shared files", async () => {
  const root = mkdtempSync(join(tmpdir(), "candidate-download-"));
  const output = join(root, "result");
  const extractionPaths = new Set<string>();
  try {
    await downloadCandidates(
      ["linux", "macos"],
      output,
      (name, directory) => {
        expect(existsSync(directory)).toBe(false);
        extractionPaths.add(directory);
        mkdirSync(directory);
        writeFileSync(join(directory, "common-model.tar.gz"), "same model");
        writeFileSync(join(directory, "desktop-metadata.awk"), "same decoder");
        writeFileSync(join(directory, `${name}.json`), name);
      },
      async (directory) => {
        expect(existsSync(output)).toBe(false);
        expect(readdirSync(directory).sort()).toEqual([
          "common-model.tar.gz",
          "desktop-metadata.awk",
          "linux.json",
          "macos.json",
        ]);
      },
    );
    expect(extractionPaths.size).toBe(2);
    expect(readFileSync(join(output, "common-model.tar.gz"), "utf8")).toBe(
      "same model",
    );
    expect(readdirSync(root)).toEqual(["result"]);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("failed or conflicting candidate acquisition never publishes a partial output", async () => {
  for (const failure of ["download", "conflict", "verification", "directory"]) {
    const root = mkdtempSync(join(tmpdir(), "candidate-download-failure-"));
    const output = join(root, "result");
    try {
      await expect(
        downloadCandidates(
          ["first", "second"],
          output,
          (name, directory) => {
            if (failure === "download" && name === "second")
              throw Error("download failed");
            mkdirSync(directory);
            if (failure === "directory")
              mkdirSync(join(directory, "unexpected-directory"));
            else
              writeFileSync(
                join(directory, "shared"),
                failure === "conflict" ? name : "same",
              );
          },
          async () => {
            if (failure === "verification") throw Error("verification failed");
          },
        ),
      ).rejects.toThrow();
      expect(existsSync(output)).toBe(false);
      expect(readdirSync(root)).toEqual([]);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  }
});

test("candidate downloads preserve an existing output directory", async () => {
  const root = mkdtempSync(join(tmpdir(), "candidate-download-existing-"));
  try {
    writeFileSync(join(root, "existing"), "preserve");
    await expect(
      downloadCandidates(
        ["one"],
        root,
        () => {
          throw Error("must not acquire");
        },
        async () => {},
      ),
    ).rejects.toThrow("already exists");
    expect(readFileSync(join(root, "existing"), "utf8")).toBe("preserve");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("candidate inventory binds all four exact payloads and rejects extra or changed attachments", async () => {
  const root = mkdtempSync(join(tmpdir(), "candidate-inventory-"));
  const merged = join(root, "merged");
  mkdirSync(merged);
  const hash = (s: string) => createHash("sha256").update(s).digest("hex");
  const intel = structuredClone(
    desktopTarget("x86_64-apple-darwin").runtime,
  ) as OfficialRuntime;
  const runtimeBytes = "synthetic reviewed runtime";
  intel.sha256 = hash(runtimeBytes);
  intel.size = runtimeBytes.length;
  const sourceReceipt = JSON.stringify({
    format_version: 1,
    target: "x86_64-apple-darwin",
    architecture: "x86_64",
    runtime_version: "1.28.0",
    source_repository: intel.source!.repository,
    source_revision: intel.source!.revision,
    archive: intel.archive,
    sha256: intel.sha256,
    members: intel.members,
  });
  intel.source!.receipt_sha256 = hash(sourceReceipt);
  const runtimeFor = (target: string) =>
    target === "x86_64-apple-darwin" ? intel : desktopTarget(target).runtime;
  const verify = (commit = "a".repeat(40)) =>
    verifyCandidate(merged, commit, runtimeFor);
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
      if (target === "x86_64-apple-darwin") {
        files[intel.archive] = runtimeBytes;
        files["onnxruntime-source-build-receipt.json"] = sourceReceipt;
      }
      for (const [name, content] of Object.entries(files))
        writeFileSync(join(directory, name), content);
      await inventory(directory, target, "a".repeat(40));
      for (const name of readdirSync(directory))
        copyFileSync(join(directory, name), join(merged, name));
    }
    expect((await verify()).length).toBeGreaterThan(20);
    await expect(verify("b".repeat(40))).rejects.toThrow("source/target");
    const unknown = join(merged, "unverified.exe");
    writeFileSync(unknown, "surprise");
    await expect(verify()).rejects.toThrow("unknown candidate attachment");
    rmSync(unknown);
    writeFileSync(join(merged, intel.archive), "substituted runtime");
    await expect(verify()).rejects.toThrow("digest mismatch");
    writeFileSync(join(merged, intel.archive), runtimeBytes);
    writeFileSync(
      join(merged, "onnxruntime-source-build-receipt.json"),
      "substituted receipt",
    );
    await expect(verify()).rejects.toThrow("digest mismatch");
    writeFileSync(
      join(merged, "onnxruntime-source-build-receipt.json"),
      sourceReceipt,
    );
    writeFileSync(join(merged, "install.sh"), "replaced installer");
    await expect(verify()).rejects.toThrow("digest mismatch");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
