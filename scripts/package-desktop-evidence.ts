#!/usr/bin/env bun
/** Ingest data from a separate immutable commit; never execute evidence-side code. */
import {
  readdirSync,
  lstatSync,
  readFileSync,
  mkdirSync,
  copyFileSync,
  writeFileSync,
} from "node:fs";
import { join, dirname } from "node:path";
import {
  localFile,
  validateMatrix,
  verifyFile,
} from "./check-desktop-evidence";
import { digest } from "./check-rust-release";
import { numericRun, verifyCandidate } from "./desktop-ci";
function json(root: string, name: string) {
  const file = localFile(root, name);
  if (lstatSync(file).size > 65536)
    throw new Error("JSON evidence exceeds 64KiB");
  return JSON.parse(readFileSync(file, "utf8"));
}
export function evidenceFiles(root: string): string[] {
  const index = json(root, "records.json");
  if (
    !Array.isArray(index) ||
    index.length !== 7 ||
    index.some((n) => typeof n !== "string") ||
    new Set(index).size !== index.length
  )
    throw new Error("seven distinct evidence records required");
  const allowed = new Set<string>(["records.json", ...index]);
  for (const name of index) {
    const record = json(root, name);
    if (!Array.isArray(record.checks)) throw new Error("checks required");
    for (const check of record.checks) allowed.add(check.evidence_path);
  }
  let total = 0;
  for (const name of allowed) {
    const size = lstatSync(localFile(root, name)).size;
    if (size === 0 || size > 50 * 1024 * 1024)
      throw new Error("evidence file size outside 1 byte–50MiB bound");
    total += size;
  }
  if (total > 250 * 1024 * 1024)
    throw new Error("evidence exceeds 250MiB aggregate bound");
  const found: string[] = [];
  function walk(prefix: string) {
    for (const name of readdirSync(join(root, prefix))) {
      const relative = prefix ? `${prefix}/${name}` : name;
      const st = lstatSync(join(root, relative));
      if (st.isSymbolicLink()) throw new Error("evidence symlink refused");
      if (st.isDirectory()) walk(relative);
      else {
        localFile(root, relative);
        found.push(relative);
      }
    }
  }
  walk("");
  const expected = [...allowed].sort();
  const data = found.filter((n) => n !== "provenance.json").sort();
  if (data.join("\n") !== expected.join("\n"))
    throw new Error("unknown evidence attachment");
  return expected;
}
export async function packageEvidence(
  release: string,
  source: string,
  out: string,
  commit: string,
  candidateRun: string,
  dataCommit: string,
) {
  numericRun(candidateRun);
  if (!/^[a-f0-9]{40}$/.test(dataCommit))
    throw new Error("full immutable evidence data commit required");
  if (readdirSync(source).includes("provenance.json"))
    throw new Error("evidence data cannot supply workflow provenance");
  const files = evidenceFiles(source);
  await verifyCandidate(release, commit);
  await validateMatrix(release, source, commit);
  mkdirSync(out, { recursive: true });
  if (readdirSync(out).length) throw new Error("evidence output must be empty");
  const hashes: Record<string, string> = {};
  for (const name of files) {
    const src = localFile(source, name);
    const dest = join(out, name);
    mkdirSync(dirname(dest), { recursive: true });
    copyFileSync(src, dest);
    hashes[name] = await digest(dest);
  }
  writeFileSync(
    join(out, "provenance.json"),
    JSON.stringify(
      {
        candidate_commit: commit,
        candidate_run: candidateRun,
        evidence_data_commit: dataCommit,
        files: hashes,
      },
      null,
      2,
    ) + "\n",
    { flag: "wx" },
  );
  await verifyEvidence(release, out, commit, candidateRun);
}
export async function verifyEvidence(
  release: string,
  source: string,
  commit: string,
  candidateRun: string,
) {
  numericRun(candidateRun);
  const provenance = json(source, "provenance.json");
  if (
    provenance.candidate_commit !== commit ||
    provenance.candidate_run !== candidateRun ||
    !/^[a-f0-9]{40}$/.test(provenance.evidence_data_commit)
  )
    throw new Error("evidence provenance mismatch");
  const files = evidenceFiles(source);
  if (Object.keys(provenance.files).sort().join("\n") !== files.join("\n"))
    throw new Error("evidence provenance file mismatch");
  for (const name of files)
    await verifyFile(source, name, provenance.files[name]);
  await verifyCandidate(release, commit);
  await validateMatrix(release, source, commit);
}
if (import.meta.main) {
  const args = Bun.argv.slice(2);
  if (args[0] === "--verify")
    await verifyEvidence(args[1], args[2], args[3], args[4]);
  else
    await packageEvidence(args[0], args[1], args[2], args[3], args[4], args[5]);
}
