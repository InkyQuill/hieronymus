/** CI/offline preparation uses the same checksum-bound cache as normal reinstalls. */
import { copyFileSync, mkdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { readReleaseV2, type DesktopTarget } from "./desktop-targets";
import { digest } from "./check-rust-release";
const target: DesktopTarget =
  process.platform === "win32"
    ? "x86_64-pc-windows-msvc"
    : process.platform === "darwin"
      ? process.arch === "arm64"
        ? "aarch64-apple-darwin"
        : "x86_64-apple-darwin"
      : "x86_64-unknown-linux-gnu";
const [source, app] = Bun.argv.slice(2);
const metadata = readReleaseV2(join(source, `release-${target}.json`), target);
const cache = join(resolve(app), "cache/downloads");
mkdirSync(cache, { recursive: true, mode: 0o700 });
for (const file of [metadata.platform, metadata.model]) {
  if ((await digest(join(source, file.archive))) !== file.sha256)
    throw Error("cache input checksum mismatch");
  copyFileSync(join(source, file.archive), join(cache, file.archive));
}
