/** Consume retained, reviewed source-build bytes; never derive new release pins. */
import { mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { desktopTarget, type OfficialRuntime } from "./desktop-targets";
import { localFile } from "./check-desktop-evidence";
import { acquireFile, verify } from "./release-assets";
import { INTEL_SOURCE } from "./build-intel-runtime";
import { spawnSync } from "node:child_process";

export function downloadRetainedArtifact(command: string[], timeout = 300_000) {
  const result = spawnSync(command[0], command.slice(1), {
    timeout,
    killSignal: "SIGKILL",
    stdio: "pipe",
    maxBuffer: 1024 ** 2,
  });
  if (result.error || result.status !== 0)
    throw new Error("Could not download the retained reviewed Intel runtime", {
      cause: result.error,
    });
}

export function reviewedSource(runtime: OfficialRuntime, target: string) {
  const source = runtime.source;
  if (
    target !== "x86_64-apple-darwin" ||
    runtime.origin !== "source-reviewed" ||
    !source ||
    source.repository !== "https://github.com/microsoft/onnxruntime.git" ||
    source.revision !== INTEL_SOURCE ||
    !/^[1-9][0-9]{0,19}$/.test(source.run) ||
    !/^[a-f0-9]{64}$/.test(source.receipt_sha256) ||
    runtime.member !== "lib/libonnxruntime.dylib" ||
    runtime.top !== "onnxruntime-osx-x86_64-1.28.0-source" ||
    !/^[A-Za-z0-9_.-]+\.tgz$/.test(runtime.archive) ||
    !runtime.url.startsWith(
      "https://github.com/InkyQuill/hieronymus/releases/download/",
    ) ||
    !runtime.url.endsWith(`/${runtime.archive}`)
  )
    throw new Error("invalid reviewed Intel runtime provenance");
  return source;
}

export function validateSourceReceipt(receipt: any, runtime: OfficialRuntime) {
  const source = reviewedSource(runtime, "x86_64-apple-darwin");
  if (
    receipt.format_version !== 1 ||
    receipt.target !== "x86_64-apple-darwin" ||
    receipt.architecture !== "x86_64" ||
    receipt.runtime_version !== "1.28.0" ||
    receipt.source_repository !== source.repository ||
    receipt.source_revision !== source.revision ||
    receipt.archive !== runtime.archive ||
    receipt.sha256 !== runtime.sha256 ||
    !receipt.members ||
    Object.keys(receipt.members).sort().join("\n") !==
      Object.keys(runtime.members).sort().join("\n")
  )
    throw new Error("reviewed runtime receipt does not match compiled pins");
  for (const [name, pin] of Object.entries(runtime.members))
    if (
      receipt.members[name]?.sha256 !== pin.sha256 ||
      receipt.members[name]?.size !== pin.size
    )
      throw new Error(`reviewed runtime member mismatch: ${name}`);
}

export async function acquireReviewedRuntime(root: string, target: string) {
  const runtime = desktopTarget(target).runtime;
  if (runtime.origin !== "source-reviewed") return undefined;
  const source = reviewedSource(runtime, target);
  const parent = join(root, "target");
  mkdirSync(parent, { recursive: true });
  let directory = mkdtempSync(join(parent, "reviewed-intel-"));
  // Once published, release attachments outlive temporary CI artifact retention.
  // Before first publication the same pinned bytes come from their retained run.
  try {
    const hosts = new Set([
      "github.com",
      "release-assets.githubusercontent.com",
      "objects.githubusercontent.com",
    ]);
    const signal = AbortSignal.timeout(300_000);
    await acquireFile(
      join(directory, runtime.archive),
      runtime.sha256,
      runtime.url,
      hosts,
      1024 ** 3,
      signal,
    );
    await acquireFile(
      join(directory, "source-build-receipt.json"),
      source.receipt_sha256,
      new URL("onnxruntime-source-build-receipt.json", runtime.url).href,
      hosts,
      1024 ** 2,
      signal,
    );
  } catch {
    // A public archive may have downloaded before its receipt failed. Keep the
    // CI extraction empty so gh neither collides with nor trusts partial data.
    rmSync(directory, { recursive: true, force: true });
    directory = mkdtempSync(join(parent, "reviewed-intel-ci-"));
    downloadRetainedArtifact([
      "gh",
      "run",
      "download",
      source.run,
      "--repo",
      "InkyQuill/hieronymus",
      "--name",
      "intel-runtime-review",
      "--dir",
      directory,
    ]);
  }
  const receipt = localFile(directory, "source-build-receipt.json");
  await verify(receipt, source.receipt_sha256);
  validateSourceReceipt(JSON.parse(readFileSync(receipt, "utf8")), runtime);
  const archive = localFile(directory, runtime.archive);
  await verify(archive, runtime.sha256, runtime.size);
  return { archive, receipt };
}
