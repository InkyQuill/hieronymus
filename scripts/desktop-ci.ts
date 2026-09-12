#!/usr/bin/env bun
/** Fixed workflow/run identity and immutable candidate inventory transport. */
import {
  readFileSync,
  readdirSync,
  writeFileSync,
  lstatSync,
  mkdtempSync,
  rmSync,
  mkdirSync,
  copyFileSync,
  renameSync,
  constants,
} from "node:fs";
import { tmpdir } from "node:os";
import { reviewedSource, validateSourceReceipt } from "./reviewed-runtime";
import { dirname, join, resolve } from "node:path";
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
export function githubFailureDetail(
  stderr: string,
  secrets: readonly string[],
) {
  let detail = stderr;
  for (const secret of secrets) {
    if (secret) detail = detail.replaceAll(secret, "[redacted]");
  }
  return detail
    .replace(/https?:\/\/\S+/g, "[URL redacted]")
    .replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g, "")
    .trim()
    .slice(0, 2000);
}
function gh(args: string[]) {
  const r = Bun.spawnSync(["gh", ...args], { stdout: "pipe", stderr: "pipe" });
  if (r.exitCode !== 0) {
    const detail = githubFailureDetail(r.stderr.toString(), [
      process.env.GH_TOKEN ?? "",
      process.env.GITHUB_TOKEN ?? "",
    ]);
    throw new Error(
      `GitHub ${args[0]} ${args[1]} failed (exit ${r.exitCode}): ${detail}`,
    );
  }
  return r.stdout.toString();
}

export function artifactAcquirer(
  id: string,
  repository: string,
  invoke: (args: string[]) => unknown = gh,
) {
  return (name: string, directory: string) => {
    invoke([
      "run",
      "download",
      id,
      "--repo",
      repository,
      "--name",
      name,
      "--dir",
      directory,
    ]);
  };
}

/** Extract independently; publish only a verified union of identical shared files. */
export async function downloadCandidates(
  names: readonly string[],
  output: string,
  acquire: (name: string, directory: string) => void | Promise<void>,
  verify: (directory: string) => Promise<unknown>,
) {
  const destination = resolve(output);
  if (lstatSync(destination, { throwIfNoEntry: false }))
    throw new Error("candidate output already exists");
  mkdirSync(dirname(destination), { recursive: true });
  const staging = mkdtempSync(
    join(dirname(destination), ".candidate-download-"),
  );
  const merged = join(staging, "merged");
  try {
    mkdirSync(merged);
    for (const [index, name] of names.entries()) {
      const artifact = join(staging, `artifact-${index}`);
      await acquire(name, artifact);
      for (const file of readdirSync(artifact)) {
        const source = localFile(artifact, file);
        const dest = join(merged, file);
        if (lstatSync(dest, { throwIfNoEntry: false })) {
          await verifyFile(merged, file, await digest(source));
        } else {
          copyFileSync(source, dest, constants.COPYFILE_EXCL);
        }
      }
    }
    await verify(merged);
    if (lstatSync(destination, { throwIfNoEntry: false }))
      throw new Error("candidate output already exists");
    renameSync(merged, destination);
  } catch (error) {
    try {
      rmSync(staging, { recursive: true, force: true });
    } catch (cleanupError) {
      throw new AggregateError(
        [error, cleanupError],
        "Candidate acquisition failed and its temporary directory could not be removed",
        { cause: error },
      );
    }
    throw error;
  }
  rmSync(staging, { recursive: true, force: true });
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
  runtimeFor: (
    target: string,
  ) => ReturnType<typeof desktopTarget>["runtime"] = (target) =>
    desktopTarget(target).runtime,
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
    const runtime = runtimeFor(target);
    if (runtime.origin === "source-reviewed") {
      const source = reviewedSource(runtime, target);
      expected.push(runtime.archive, "onnxruntime-source-build-receipt.json");
      await verifyFile(directory, runtime.archive, runtime.sha256);
      await verifyFile(
        directory,
        "onnxruntime-source-build-receipt.json",
        source.receipt_sha256,
      );
      validateSourceReceipt(
        JSON.parse(
          readFileSync(
            localFile(directory, "onnxruntime-source-build-receipt.json"),
            "utf8",
          ),
        ),
        runtime,
      );
    }
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
    const acquire = artifactAcquirer(id, repo);
    if (kind === "desktop-candidate")
      await downloadCandidates(names, out, acquire, (directory) =>
        verifyCandidate(directory, source),
      );
    else acquire(names[0], out);
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
