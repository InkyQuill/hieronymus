#!/usr/bin/env bun
/** Fixed workflow/run identity and immutable candidate inventory transport. */
import {
  readFileSync,
  readdirSync,
  writeFileSync,
  lstatSync,
  mkdtempSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { digest, checkMetadata, checkSource } from "./check-rust-release";
import { TARGETS, desktopTarget, readReleaseV2 } from "./desktop-targets";
import { localFile, verifyFile } from "./check-desktop-evidence";
export function numericRun(value: string) {
  if (!/^[1-9][0-9]{0,19}$/.test(value))
    throw new Error("numeric workflow run ID required");
  return value;
}
export function validateRun(
  run: any,
  repository: string,
  workflow: string,
  commit?: string,
) {
  if (
    run.repository?.full_name !== repository ||
    run.head_repository?.full_name !== repository ||
    run.path !== `.github/workflows/${workflow}.yml` ||
    run.event !== "workflow_dispatch" ||
    run.status !== "completed" ||
    run.conclusion !== "success" ||
    !/^[a-f0-9]{40}$/.test(run.head_sha) ||
    (commit && run.head_sha !== commit)
  )
    throw new Error(
      "unexpected workflow/repository/source or unsuccessful run",
    );
  return run.head_sha as string;
}
function gh(args: string[]) {
  const r = Bun.spawnSync(["gh", ...args], { stdout: "pipe", stderr: "pipe" });
  if (r.exitCode !== 0) throw new Error("GitHub artifact operation failed");
  return r.stdout.toString();
}
export async function inventory(
  directory: string,
  target: string,
  commit: string,
) {
  desktopTarget(target);
  if (!/^[a-f0-9]{40}$/.test(commit))
    throw new Error("exact source commit required");
  const files: Record<string, string> = {};
  for (const name of readdirSync(directory).sort()) {
    if (name.startsWith("candidate-")) continue;
    files[name] = await digest(localFile(directory, name));
  }
  writeFileSync(
    join(directory, `candidate-${target}.json`),
    JSON.stringify({ candidate_commit: commit, target, files }, null, 2) + "\n",
    { flag: "wx" },
  );
}
export async function verifyCandidate(
  directory: string,
  commit: string,
): Promise<string[]> {
  const sourceVersion = checkSource(process.cwd(), undefined, true);
  const all = new Set<string>();
  let modelHash: string | undefined;
  let version: string | undefined;
  for (const target of TARGETS) {
    const name = `candidate-${target}.json`;
    const record = JSON.parse(readFileSync(localFile(directory, name), "utf8"));
    if (
      record.candidate_commit !== commit ||
      record.target !== target ||
      !record.files ||
      typeof record.files !== "object"
    )
      throw new Error("candidate source/target inventory mismatch");
    const metadata = readReleaseV2(
      localFile(directory, desktopTarget(target).metadata),
      target,
    );
    if (version && version !== metadata.version)
      throw new Error("candidate versions differ");
    version = metadata.version;
    if (modelHash && modelHash !== metadata.model.sha256)
      throw new Error("common model differs");
    modelHash = metadata.model.sha256;
    const receipt = JSON.parse(
      readFileSync(localFile(directory, `evidence-${target}.json`), "utf8"),
    );
    const expected = [
      desktopTarget(target).metadata,
      metadata.platform.archive,
      metadata.model.archive,
      `evidence-${target}.json`,
      `assets-${target}.json`,
      `install-desktop-${target}.${target.includes("windows") ? "ps1" : "sh"}`,
      ...(!target.includes("windows")
        ? ["install.sh", "desktop-metadata.awk"]
        : []),
      ...receipt.symbols
        .filter((s: any) => s.original_sha256)
        .map((s: any) => s.debug),
    ];
    if (
      Object.keys(record.files).sort().join("\n") !== expected.sort().join("\n")
    )
      throw new Error("candidate file allowlist mismatch");
    for (const file of expected) {
      await verifyFile(directory, file, record.files[file]);
      all.add(file);
    }
    await checkMetadata(directory, sourceVersion, "stable", target);
    all.add(name);
  }
  if (readdirSync(directory).sort().join("\n") !== [...all].sort().join("\n"))
    throw new Error("unknown candidate attachment");
  return [...all].sort();
}
if (import.meta.main) {
  const [mode, ...args] = Bun.argv.slice(2);
  if (mode === "inventory") await inventory(args[0], args[1], args[2]);
  else if (mode === "verify")
    console.log(JSON.stringify(await verifyCandidate(args[0], args[1])));
  else if (mode === "download") {
    const [id, kind, out, commit] = args;
    if (!["desktop-candidate", "desktop-evidence"].includes(kind))
      throw new Error("unknown workflow");
    const repo = process.env.GITHUB_REPOSITORY!;
    if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo))
      throw new Error("repository required");
    const run = JSON.parse(
      gh(["api", `repos/${repo}/actions/runs/${numericRun(id)}`]),
    );
    const source = validateRun(run, repo, kind, commit);
    const names =
      kind === "desktop-candidate"
        ? TARGETS.map((t) => `candidate-${t}`)
        : ["desktop-evidence"];
    for (const name of names)
      gh(["run", "download", id, "--repo", repo, "--name", name, "--dir", out]);
    if (kind === "desktop-candidate") await verifyCandidate(out, source);
    if (process.env.GITHUB_OUTPUT) {
      const { appendFileSync } = await import("node:fs");
      appendFileSync(process.env.GITHUB_OUTPUT, `commit=${source}\n`);
    }
    console.log(source);
  } else if (mode === "publish") {
    const [directory, commit, ref, evidence, candidateRun] = args;
    const { checkSource } = await import("./check-rust-release");
    checkSource(process.cwd(), ref);
    const { verifyEvidence } = await import("./package-desktop-evidence");
    await verifyEvidence(directory, evidence, commit, candidateRun);
    const files = await verifyCandidate(directory, commit);
    const names = files.filter((n) => !n.startsWith("candidate-"));
    const summaryDirectory = mkdtempSync(
      join(tmpdir(), "hieronymus-qualification-"),
    );
    const summary = join(summaryDirectory, "native-qualification.json");
    const records = JSON.parse(
      readFileSync(localFile(evidence, "records.json"), "utf8"),
    );
    writeFileSync(
      summary,
      JSON.stringify(
        {
          provenance: JSON.parse(
            readFileSync(localFile(evidence, "provenance.json"), "utf8"),
          ),
          records: records.map((path: string) =>
            JSON.parse(readFileSync(localFile(evidence, path), "utf8")),
          ),
        },
        null,
        2,
      ) + "\n",
    );
    try {
      gh([
        "release",
        "create",
        ref.replace("refs/tags/", ""),
        "--verify-tag",
        "--title",
        `hiero ${ref.replace("refs/tags/", "")}`,
        "--notes-file",
        "docs/desktop-release-notes.md",
        summary,
        ...names.map((n) => join(directory, n)),
      ]);
    } finally {
      rmSync(summaryDirectory, { recursive: true, force: true });
    }
  } else throw new Error("expected inventory|verify|download|publish");
}
