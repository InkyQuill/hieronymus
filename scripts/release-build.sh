#!/usr/bin/env bash
# Release build orchestration (distribution spec §Build Ownership):
#
#   1. check the pinned Bun version (1.4.0);
#   2. build the production console bundle (`frontend/dist`) from the
#      repository lockfile — an explicit step BEFORE compiling the binary;
#   3. verify the embedded-asset tests against the real bundle;
#   4. build the release binary for x86_64-unknown-linux-gnu with the
#      `console-embed` feature (a missing bundle fails the compile in
#      `crates/hiero/build.rs`);
#   5. package the archive (binary + argv[0] command links) and its
#      SHA-256 checksums;
#   6. verify the archive round-trip (checksum, extraction, version probes).
#
# Modes:
#   (default)        full pipeline
#   --assets-only    steps 1-2 plus the console-embed compile check
#   --dry-run        print the plan without building anything
#
# Ordinary developer tests never invoke Bun; only this explicit
# orchestration step (and CI, which calls it) builds the frontend.

set -euo pipefail

TARGET="x86_64-unknown-linux-gnu"
BUN_PIN="1.4.0"
ARCHIVE_DIR="target/release-dist"

mode="full"
case "${1:-}" in
  "") ;;
  --assets-only) mode="assets-only" ;;
  --dry-run) mode="dry-run" ;;
  *)
    echo "usage: $0 [--assets-only|--dry-run]" >&2
    exit 2
    ;;
esac

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

version="$(awk '/^\[workspace\.package\]/{inws=1; next} /^\[/{inws=0} inws && /^version *=/ {gsub(/"/, "", $3); print $3; exit}' Cargo.toml)"
if [ -z "$version" ]; then
  echo "error: cannot read the workspace version from Cargo.toml" >&2
  exit 1
fi

archive_name="hieronymus-${version}-${TARGET}.tar.gz"

step() { printf '\n== %s\n' "$*"; }

step "release build plan (${mode})"
echo "   version:      $version"
echo "   target:       $TARGET"
echo "   bun pin:      $BUN_PIN"
echo "   embed feature: console-embed"
echo "   archive:      $ARCHIVE_DIR/$archive_name"

if [ "$mode" = "dry-run" ]; then
  echo
  echo "dry run: no build performed"
  exit 0
fi

step "checking Bun pin ($BUN_PIN)"
if ! command -v bun >/dev/null 2>&1; then
  echo "error: bun is not installed; the release build needs bun $BUN_PIN" >&2
  exit 1
fi
bun_version="$(bun --version)"
if [ "$bun_version" != "$BUN_PIN" ]; then
  echo "error: bun $bun_version found, but the pinned version is $BUN_PIN" >&2
  exit 1
fi

step "building the production console bundle (frontend/dist)"
bun install --cwd frontend --frozen-lockfile
bun run --cwd frontend build
if [ ! -f frontend/dist/index.html ]; then
  echo "error: frontend/dist/index.html is missing after the console build" >&2
  exit 1
fi
echo "console bundle: $(find frontend/dist -type f | wc -l) files"

if [ "$mode" = "assets-only" ]; then
  step "console-embed compile check (developer frontend-free path stays intact)"
  cargo check -p hiero --features console-embed
  echo
  echo "assets-only build complete"
  exit 0
fi

step "embedded-asset tests (real bundle: index fallback, MIME, hashed assets, no source maps)"
cargo test -p hiero --features console-embed --test console_embed

step "building the release binary (console-embed, $TARGET)"
cargo build --release --locked -p hiero --features console-embed --target "$TARGET"
binary="target/$TARGET/release/hiero"
if [ ! -f "$binary" ]; then
  echo "error: release binary not found at $binary" >&2
  exit 1
fi

step "packaging archive"
out_dir="$repo_root/$ARCHIVE_DIR"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/payload"
cp "$binary" "$stage/payload/hiero"
# argv[0] command links (distribution spec §Installer): the single binary
# routes these names itself, the links only carry them.
ln -s hiero "$stage/payload/hieronymus"
ln -s hiero "$stage/payload/hieronymus-agent-hook"
ln -s hiero "$stage/payload/hieronymus-mcp"
mkdir -p "$out_dir"
tar -czf "$out_dir/$archive_name" -C "$stage/payload" hiero hieronymus hieronymus-agent-hook hieronymus-mcp

step "SHA-256 checksums"
(
  cd "$out_dir"
  sha256sum "$archive_name" >"$archive_name.sha256"
  cat "$archive_name.sha256"
)

step "verifying the archive round-trip"
(
  cd "$out_dir"
  sha256sum -c "$archive_name.sha256"
)
extract="$stage/extract"
mkdir -p "$extract"
tar -xzf "$out_dir/$archive_name" -C "$extract"
if [ ! "$(readlink "$extract/hieronymus")" = "hiero" ] ||
  [ ! "$(readlink "$extract/hieronymus-agent-hook")" = "hiero" ] ||
  [ ! "$(readlink "$extract/hieronymus-mcp")" = "hiero" ]; then
  echo "error: command links did not survive the archive round-trip" >&2
  exit 1
fi
embedded_version="$("$extract/hiero" version --json | sed -n 's/.*"version": "\([^"]*\)".*/\1/p')"
if [ "$embedded_version" != "$version" ]; then
  echo "error: binary reports version '$embedded_version', expected '$version'" >&2
  exit 1
fi
"$extract/hieronymus" version >/dev/null
echo "round-trip: checksum, links, and version probes all pass"

step "release artifacts"
echo "   $ARCHIVE_DIR/$archive_name"
echo "   $ARCHIVE_DIR/$archive_name.sha256"
