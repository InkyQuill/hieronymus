/** Public installation payloads; candidate/test evidence stays in maintainer storage. */
import { readReleaseV2, TARGETS } from "./desktop-targets";
import { join } from "node:path";
export function publicPayloads(directory: string): string[] {
  const names = new Set<string>();
  for (const target of TARGETS) {
    const metadata = `release-${target}.json`;
    const release = readReleaseV2(join(directory, metadata), target);
    names.add(metadata);
    names.add(release.platform.archive);
    names.add(release.model.archive);
  }
  return [...names].sort();
}
export function nativeInstallerNames(version: string) {
  if (!/^\d+\.\d+\.\d+$/.test(version))
    throw Error("stable release version required");
  return [
    "install-hieronymus.sh",
    `Hieronymus-${version}-Setup.exe`,
    `Hieronymus-${version}.pkg`,
  ];
}
