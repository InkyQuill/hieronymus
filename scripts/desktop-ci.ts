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

/** Do not advertise downloads omitted from a partial release. */
export function availableInstallerNotes(
  template: string,
  available: string[],
  targets: readonly string[],
): string {
  if (!targets.includes("x86_64-unknown-linux-gnu")) {
    template = template.replace(
      /^- \*\*Linux x86_64:\*\*[^\n]*\n\n```bash\n[\s\S]*?```\n?/m,
      "",
    );
  }
  return template
    .split("\n")
    .filter((line) => {
      const match = line.match(/^- \*\*(Windows|macOS):\*\*.*\/([^/()]+)\)/);
      return !match || available.includes(match[2]);
    })
    .join("\n");
}
/** Packaging and documentation edits do not change retained binary bytes. */
export function binarySourceChanged(paths: string[]): boolean {
  const packaging = new Set([
    ".github/workflows/release-rust.yml",
    ".github/workflows/installer-checks.yml",
    ".github/workflows/desktop-evidence.yml",
    ".github/workflows/pr.yml",
    "scripts/desktop-ci.ts",
    "scripts/report-release-warnings.sh",
    "scripts/build-installers.ts",
    "scripts/build-native-installer.ts",
    "scripts/install-desktop.sh",
    "scripts/install-desktop.ps1",
    "scripts/install.sh",
    "scripts/release-downloads.ts",
    "scripts/package-desktop-evidence.ts",
    "scripts/check-desktop-evidence.ts",
    "scripts/stage-installer-cache.ts",
  ]);
  return paths.some(
    (p) =>
      !(
        p.startsWith("docs/") ||
        p.startsWith("qualification/") ||
        p.startsWith("scripts/setup/") ||
        p.endsWith(".test.ts") ||
        (!p.includes("/") && p.endsWith(".md")) ||
        packaging.has(p)
      ),
  );
}

function verifyReusableSource(source: string, current: string) {
  if (source === current) return;
  const result = Bun.spawnSync(
    ["git", "diff", "--name-only", source, current],
    { stdout: "pipe", stderr: "pipe" },
  );
  if (
    result.exitCode !== 0 ||
    binarySourceChanged(
      result.stdout.toString().trim().split("\n").filter(Boolean),
    )
  )
    throw new Error(
      "Binary inputs changed; build a new candidate for this source",
    );
}

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
  allowFailedChecks = false,
) {
  if (
    run.repository?.full_name !== repository ||
    run.head_repository?.full_name !== repository ||
    run.path !== `.github/workflows/${workflow}.yml` ||
    run.event !== "workflow_dispatch" ||
    run.status !== "completed" ||
    (run.conclusion !== "success" &&
      !(allowFailedChecks && run.conclusion === "failure")) ||
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
  let detail = stderr.replace(/https?:\/\/\S+/gi, "[URL redacted]");
  for (const secret of secrets
    .filter(Boolean)
    .sort((a, b) => b.length - a.length)) {
    detail = detail.replaceAll(secret, "[redacted]");
  }
  return detail
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
  const targets = TARGETS.filter((t) =>
    lstatSync(join(directory, `candidate-${t}.json`), {
      throwIfNoEntry: false,
    }),
  );
  if (!targets.length) throw new Error("No candidate artifacts available");
  for (const target of targets) {
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
    const source = validateRun(
      run,
      repo,
      kind,
      undefined,
      kind === "desktop-candidate",
    );
    if (commit) verifyReusableSource(source, commit);
    let names =
      kind === "desktop-candidate"
        ? TARGETS.map((t) => `candidate-${t}`)
        : ["desktop-evidence"];
    if (kind === "desktop-candidate") {
      const pages = JSON.parse(
        gh([
          "api",
          "--paginate",
          "--slurp",
          `repos/${repo}/actions/runs/${id}/artifacts`,
        ]),
      );
      const available = new Set(
        pages.flatMap((page: any) =>
          page.artifacts.filter((a: any) => !a.expired).map((a: any) => a.name),
        ),
      );
      names = names.filter((name) => {
        if (available.has(name)) return true;
        console.warn(`::warning::Missing platform artifact: ${name}`);
        return false;
      });
    }
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
    const [directory, commit, ref, evidence, candidateRun, installers] = args;
    const { checkSource } = await import("./check-rust-release");
    checkSource(process.cwd(), ref);
    const { verifyEvidence } = await import("./package-desktop-evidence");
    const retainedCommit = JSON.parse(
      readFileSync(
        localFile(
          directory,
          readdirSync(directory).find(
            (n) => n.startsWith("candidate-") && n.endsWith(".json"),
          )!,
        ),
        "utf8",
      ),
    ).candidate_commit;
    if (!/^[a-f0-9]{40}$/.test(retainedCommit))
      throw new Error("Invalid candidate source");
    verifyReusableSource(retainedCommit, commit);
    await verifyCandidate(directory, retainedCommit);
    try {
      await verifyEvidence(directory, evidence, retainedCommit, candidateRun);
    } catch (error) {
      console.warn(`::warning::Optional desktop evidence: ${error}`);
    }
    const { publicPayloads, nativeInstallerNames } =
      await import("./release-downloads");
    const { renderInstallers } = await import("./build-installers");
    const releases = TARGETS.filter((t) =>
      lstatSync(join(directory, `release-${t}.json`), {
        throwIfNoEntry: false,
      }),
    ).map((target) =>
      readReleaseV2(localFile(directory, `release-${target}.json`), target),
    );
    const names = publicPayloads(directory);
    const setupNames = nativeInstallerNames(releases[0].version);
    const availableSetup = setupNames.filter((name) => {
      if (
        name.endsWith(".exe") &&
        !releases.some((r) => r.target.includes("windows"))
      )
        return false;
      if (
        name.endsWith(".pkg") &&
        !releases.some((r) => r.target.includes("darwin"))
      )
        return false;
      if (lstatSync(join(installers, name), { throwIfNoEntry: false })) {
        localFile(installers, name);
        return true;
      }
      console.warn(`::warning::Native installer unavailable: ${name}`);
      return false;
    });
    if (
      readFileSync(localFile(installers, "install-hieronymus.sh"), "utf8") !==
      renderInstallers(releases)["install-hieronymus.sh"]
    )
      throw new Error("installer source/release mismatch");
    if (
      (availableSetup.includes(setupNames[1]) &&
        readFileSync(localFile(installers, setupNames[1]))
          .subarray(0, 2)
          .toString() !== "MZ") ||
      (availableSetup.includes(setupNames[2]) &&
        readFileSync(localFile(installers, setupNames[2]))
          .subarray(0, 4)
          .toString() !== "xar!")
    )
      throw new Error("invalid native setup package");
    const summaryDirectory = mkdtempSync(
      join(tmpdir(), "hieronymus-qualification-"),
    );
    const notes = join(summaryDirectory, "release-notes.md");
    const evidenceUrl = `https://github.com/InkyQuill/hieronymus/actions/runs/${candidateRun}`;
    writeFileSync(
      notes,
      availableInstallerNotes(
        readFileSync("docs/desktop-release-notes.md", "utf8"),
        availableSetup,
        releases.map((r) => r.target),
      ).replaceAll("@@EVIDENCE_URL@@", evidenceUrl) +
        `\n\nAvailable binary targets: ${releases.map((r) => r.target).join(", ")}.\n` +
        `Missing targets: ${TARGETS.filter((t) => !releases.some((r) => r.target === t)).join(", ") || "none"}.\n` +
        `Missing installers: ${setupNames.filter((n) => !availableSetup.includes(n)).join(", ") || "none"}.\n` +
        "Native installation checks and evidence are advisory; see the linked workflow and open release-warning issues for failures or untested behavior.\n",
    );
    try {
      gh([
        "release",
        "create",
        ref.replace("refs/tags/", ""),
        "--verify-tag",
        "--title",
        `Hieronymus ${ref.replace("refs/tags/", "")}`,
        "--notes-file",
        notes,
        ...availableSetup.map((n) => join(installers, n)),
        ...names.map((n) => join(directory, n)),
      ]);
    } finally {
      rmSync(summaryDirectory, { recursive: true, force: true });
    }
  } else throw new Error("expected inventory|verify|download|publish");
}
