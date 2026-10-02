#!/usr/bin/env bun
/** Release orchestration: explicit dispatch avoids GITHUB_TOKEN event suppression. */
import { readFileSync, writeFileSync } from "node:fs";

export function synchronizeVersion(version: string, manifest: string, lock: string) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error("Expected stable SemVer");
  let changed = false;
  const cargo = manifest.replace(/(\[workspace\.package\][\s\S]*?\nversion = ")[^"]+("\n)/, (_, a, b) => {changed = true; return a + version + b;});
  if (!changed) throw new Error("Workspace version missing");
  const names = new Set(["hiero", "hieronymus", "hiero-desktop"]);
  const cargoLock = lock.replace(/(\[\[package\]\]\nname = "([^"]+)"\nversion = ")[^"]+("\n)/g, (all, a, name, b) => {
    if (!names.delete(name)) return all;
    return a + version + b;
  });
  if (names.size) throw new Error("Workspace lock entries missing");
  return {cargo, cargoLock};
}

export function synchronizeReadme(version: string, readme: string): string {
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error("Expected stable SemVer");
  const start = readme.indexOf("## Install\n");
  const end = readme.indexOf("\n## ", start + 1);
  if (start < 0 || end < 0) throw new Error("README install section missing");
  let count = 0;
  const install = readme.slice(start, end).replace(
    /https:\/\/github\.com\/InkyQuill\/hieronymus\/releases\/download\/v\d+\.\d+\.\d+\/(?:Hieronymus-\d+\.\d+\.\d+-(Setup\.exe)|Hieronymus-\d+\.\d+\.\d+(\.pkg)|(install-hieronymus\.sh))/g,
    (_, exe, pkg, shell) => {
      count++;
      const asset = shell ?? (exe ? `Hieronymus-${version}-Setup.exe` : `Hieronymus-${version}.pkg`);
      return `https://github.com/InkyQuill/hieronymus/releases/download/v${version}/${asset}`;
    },
  );
  if (count !== 3) throw new Error("Expected three versioned installer links");
  return readme.slice(0, start) + install + readme.slice(end);
}

export function selectedRun(runs: any[], sha: string, request: string): any | undefined {
  return runs.find(run => run.head_sha === sha && run.event === "workflow_dispatch" && run.display_title === `desktop-candidate / ${request}` && run.path === ".github/workflows/desktop-candidate.yml");
}
function gh(args: string[]) {
  const run = Bun.spawnSync(["gh", ...args], {stdout:"pipe",stderr:"pipe"});
  if (run.exitCode) throw new Error(run.stderr.toString());
  return run.stdout.toString();
}
export async function buildAndPromote(repo: string, tag: string, sha: string, invoke = gh, sleep: (ms:number)=>Promise<unknown> = Bun.sleep, attempts = 660) {
  if (!/^[\w.-]+\/[\w.-]+$/.test(repo) || !/^v\d+\.\d+\.\d+$/.test(tag) || !/^[a-f0-9]{40}$/.test(sha)) throw new Error("Invalid release identity");
  const request = crypto.randomUUID();
  invoke(["api", "--method", "POST", `repos/${repo}/actions/workflows/desktop-candidate.yml/dispatches`, "-f", `ref=${tag}`, "-f", `inputs[orchestration_id]=${request}`]);
  for (let attempt=0;attempt<attempts;attempt++) {
    await sleep(30000);
    const runs = JSON.parse(invoke(["api", `repos/${repo}/actions/workflows/desktop-candidate.yml/runs?event=workflow_dispatch&head_sha=${sha}&per_page=100`]));
    const run = selectedRun(runs.workflow_runs,sha,request);
    if (!run || run.status !== "completed") continue;
    if (run.conclusion !== "success") throw new Error(`Candidate failed: ${run.html_url}`);
    invoke(["api","--method","POST",`repos/${repo}/actions/workflows/release-rust.yml/dispatches`,"-f",`ref=${tag}`,"-f",`inputs[release_tag]=${tag}`,"-f",`inputs[candidate_run]=${run.id}`]);
    console.log(`Promoting retained candidate ${run.id} for ${tag}`);
    return run.id;
  }
  throw new Error("Candidate wait exceeded 330 minutes; resume publication with the existing candidate run, do not dispatch another build");
}
if (import.meta.main) {
  if (Bun.argv[2] === "sync") {
    const version = readFileSync("version.txt","utf8").trim();
    const readme = synchronizeReadme(version, readFileSync("README.md","utf8"));
    const next = synchronizeVersion(version,readFileSync("Cargo.toml","utf8"),readFileSync("Cargo.lock","utf8"));
    writeFileSync("README.md",readme);
    writeFileSync("Cargo.toml",next.cargo);writeFileSync("Cargo.lock",next.cargoLock);
  } else if (Bun.argv[2] === "promote") await buildAndPromote(process.env.GITHUB_REPOSITORY!,Bun.argv[3],Bun.argv[4]);
  else throw new Error("Expected sync|promote");
}
