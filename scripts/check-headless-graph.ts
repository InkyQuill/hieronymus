#!/usr/bin/env bun
const command = Bun.spawnSync(
  [
    "cargo",
    "tree",
    "--locked",
    "--all-features",
    "--prefix",
    "none",
    "--format",
    "{p}",
  ],
  { stdout: "pipe", stderr: "inherit" },
);
if (command.exitCode !== 0) throw new Error("dependency graph failed");
const forbidden =
  /^(gtk|gtk-sys|gdk|gdk-sys|tray-icon|muda|objc2-app-kit|hiero-desktop) /m;
if (forbidden.test(command.stdout.toString()))
  throw new Error("GUI dependency leaked into default workspace graph");
console.log("Default workspace graph contains no desktop GUI packages");
