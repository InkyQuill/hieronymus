#!/usr/bin/env bash
# Bootstrap installer (distribution spec §Installer, as amended 2026-09-03):
#
#   1. resolve a supported OS/architecture WITHOUT executing downloaded
#      content;
#   2. fetch the release metadata and the matching archive (a local release
#      directory or an HTTPS base URL; no external network is contacted in
#      tests);
#   3. verify the SHA-256 checksum (and refuse any signature the waived
#      first release line cannot verify);
#   4. install atomically into a versioned application directory;
#   5. update the stable command links;
#   6. install/update the per-user daemon service;
#   7. run the non-mutating doctor checks;
#   8. start the daemon only when the existing schema is already compatible.
#
# If a database upgrade is required, installation completes but the daemon
# stays STOPPED until the user runs or confirms the separately reported
# migration — this installer never performs a hidden destructive schema
# upgrade. No Python, Node, or Bun is required on the target. The layout is
# shared with `hiero update` (crates/hiero/src/app.rs):
#
#   <app>/versions/<version>/{hiero,hieronymus,hieronymus-agent-hook,hieronymus-mcp}
#   <app>/bin/<name>          stable links into versions/<version>
#   ~/.local/bin/<name>       PATH links into <app>/bin
#
# Usage:
#   scripts/install.sh --release-dir <dir> | --release-url <base-url>
#                      [--app-dir <dir>] [--data-root <dir>] [--unit-dir <dir>]
#                      [--no-activate]

set -euo pipefail

TARGET="x86_64-unknown-linux-gnu"
APP_DIR="${HIERONYMUS_APP_DIR:-${HOME}/.local/share/hieronymus/app}"
DATA_ROOT="${HIERONYMUS_DATA_ROOT:-${HOME}/.config/hieronymus}"
case "$DATA_ROOT" in
  "~/"*) DATA_ROOT="${HOME}/${DATA_ROOT#~/}" ;;
esac
RELEASE_DIR="${HIERONYMUS_RELEASE_DIR:-}"
RELEASE_URL="${HIERONYMUS_RELEASE_URL:-}"
UNIT_DIR="${HIERONYMUS_UNIT_DIR:-}"
NO_ACTIVATE=0

usage() {
  echo "usage: $0 (--release-dir <dir> | --release-url <base-url>) [--app-dir <dir>] [--data-root <dir>] [--unit-dir <dir>] [--no-activate]" >&2
  exit 2
}

while [ $# -gt 0 ]; do
  case "$1" in
    --release-dir)
      [ $# -ge 2 ] || usage
      RELEASE_DIR="$2"
      shift 2
      ;;
    --release-url)
      [ $# -ge 2 ] || usage
      RELEASE_URL="$2"
      shift 2
      ;;
    --app-dir)
      [ $# -ge 2 ] || usage
      APP_DIR="$2"
      shift 2
      ;;
    --data-root)
      [ $# -ge 2 ] || usage
      DATA_ROOT="$2"
      shift 2
      ;;
    --unit-dir)
      [ $# -ge 2 ] || usage
      UNIT_DIR="$2"
      shift 2
      ;;
    --no-activate)
      NO_ACTIVATE=1
      shift
      ;;
    *) usage ;;
  esac
done

if [ -z "$RELEASE_DIR" ] && [ -z "$RELEASE_URL" ]; then
  echo "error: one of --release-dir or --release-url is required" >&2
  usage
fi
if [ -n "$RELEASE_DIR" ] && [ -n "$RELEASE_URL" ]; then
  echo "error: --release-dir and --release-url are mutually exclusive" >&2
  usage
fi
if [ -n "$RELEASE_URL" ]; then
  case "$RELEASE_URL" in
    https://*) : ;;
    *)
      echo "error: --release-url must be an https:// base URL; over http:// a network attacker could replace the binary together with the release.json sha256 that vouches for it. Use --release-dir for local or test installs." >&2
      exit 1
      ;;
  esac
fi

# The managed layout needs absolute paths: the stable and PATH links carry
# them, and the service unit execs them. Relative arguments are resolved
# against the caller's working directory here, once.
abspath() {
  case "$1" in
    /*) printf '%s\n' "$1" ;;
    *) printf '%s\n' "$(pwd)/$1" ;;
  esac
}
APP_DIR="$(abspath "$APP_DIR")"
DATA_ROOT="$(abspath "$DATA_ROOT")"
# `set -e` note: bare `[ -n x ] && cmd` lists abort the script when x is
# empty, so the optional paths use explicit if-statements.
if [ -n "$UNIT_DIR" ]; then
  UNIT_DIR="$(abspath "$UNIT_DIR")"
fi
if [ -n "$RELEASE_DIR" ]; then
  RELEASE_DIR="$(abspath "$RELEASE_DIR")"
fi

step() { printf '\n== %s\n' "$*"; }
info() { echo "   $*"; }
fatal() {
  echo "error: $*" >&2
  exit 1
}

# ---------------------------------------------------------------------------
step "1. resolve the supported platform"
# ---------------------------------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
info "os:          $os"
info "arch:        $arch"
[ "$os" = "Linux" ] || fatal "unsupported OS '$os'; this installer supports Linux only"
[ "$arch" = "x86_64" ] || fatal "unsupported architecture '$arch'; this installer supports x86_64 only"
info "target:      $TARGET"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# ---------------------------------------------------------------------------
step "2. fetch the release metadata and archive"
# ---------------------------------------------------------------------------
json_field() {
  # Extract a top-level string field from release.json without jq.
  grep -o "\"$2\"[[:space:]]*:[[:space:]]*\"[^\"]*\"" "$1" |
    head -n 1 |
    sed 's/.*:[[:space:]]*"//; s/"$//'
}

archive_name=""
expected_sha=""
declared_version=""
if [ -n "$RELEASE_DIR" ]; then
  [ -d "$RELEASE_DIR" ] || fatal "release directory does not exist: $RELEASE_DIR"
  if [ -f "$RELEASE_DIR/release.json" ]; then
    info "metadata:    $RELEASE_DIR/release.json"
    declared_version="$(json_field "$RELEASE_DIR/release.json" version || true)"
    archive_name="$(json_field "$RELEASE_DIR/release.json" archive || true)"
    expected_sha="$(json_field "$RELEASE_DIR/release.json" sha256 || true)"
    [ -n "$archive_name" ] || fatal "release.json does not declare an archive name"
    [ -f "$RELEASE_DIR/$archive_name" ] || fatal "release.json points at $archive_name, which is not in $RELEASE_DIR"
    archive_path="$RELEASE_DIR/$archive_name"
  else
    matches="$(find "$RELEASE_DIR" -maxdepth 1 -name "hieronymus-*-$TARGET.tar.gz" | sort)"
    count="$(printf '%s' "$matches" | grep -c . || true)"
    [ "$count" = "1" ] || fatal "expected exactly one hieronymus-*-$TARGET.tar.gz in $RELEASE_DIR (found $count); add release.json or keep one archive"
    archive_path="$matches"
    archive_name="$(basename "$archive_path")"
  fi
else
  command -v curl >/dev/null 2>&1 || fatal "curl is required for --release-url"
  info "base url:    $RELEASE_URL"
  curl -fsSL "$RELEASE_URL/release.json" -o "$work/release.json" ||
    fatal "could not download $RELEASE_URL/release.json"
  declared_version="$(json_field "$work/release.json" version || true)"
  archive_name="$(json_field "$work/release.json" archive || true)"
  expected_sha="$(json_field "$work/release.json" sha256 || true)"
  [ -n "$archive_name" ] || fatal "release.json does not declare an archive name"
  curl -fsSL "$RELEASE_URL/$archive_name" -o "$work/$archive_name" ||
    fatal "could not download $RELEASE_URL/$archive_name"
  curl -fsSL "$RELEASE_URL/$archive_name.sha256" -o "$work/$archive_name.sha256" ||
    true
  archive_path="$work/$archive_name"
fi
info "archive:     $archive_name"
# Downloaded content is only ever read as data here; nothing from the
# release executes before step 3 verified it.

# ---------------------------------------------------------------------------
step "3. verify the archive checksum"
# ---------------------------------------------------------------------------
if [ -z "$expected_sha" ]; then
  checksum_file="$archive_path.sha256"
  [ -f "$checksum_file" ] || fatal "no release.json sha256 and no $archive_name.sha256 sibling"
  expected_sha="$(awk '{print $1}' "$checksum_file")"
fi
signature=""
if [ -n "$RELEASE_DIR" ] && [ -f "$RELEASE_DIR/release.json" ]; then
  signature="$(json_field "$RELEASE_DIR/release.json" signature || true)"
fi
if [ -f "$work/release.json" ]; then
  url_signature="$(json_field "$work/release.json" signature || true)"
  if [ -n "$url_signature" ]; then
    signature="$url_signature"
  fi
fi
if [ -n "$signature" ]; then
  fatal "release metadata carries a signature, but signature verification is not configured in the waived first release line; refusing to trust it"
fi
actual_sha="$(sha256sum "$archive_path" | awk '{print $1}')"
expected_sha="$(printf '%s' "$expected_sha" | tr 'A-Z' 'a-z')"
[ "$actual_sha" = "$expected_sha" ] ||
  fatal "checksum mismatch for $archive_name: expected $expected_sha, got $actual_sha"
info "sha256 verified: $actual_sha"

# ---------------------------------------------------------------------------
step "4. install into a versioned application directory"
# ---------------------------------------------------------------------------
mkdir -p "$APP_DIR/versions"
stage="$(mktemp -d "$APP_DIR/versions/.stage-XXXXXX")"
tar -xzf "$archive_path" -C "$stage"
[ -x "$stage/hiero" ] || fatal "the archive payload has no executable hiero binary"
for name in hieronymus hieronymus-agent-hook hieronymus-mcp; do
  [ "$(readlink "$stage/$name")" = "hiero" ] ||
    fatal "archive link $name does not point at hiero"
done
version="$("$stage/hiero" version --json | sed -n 's/.*"version": "\([^"]*\)".*/\1/p')"
[ -n "$version" ] || fatal "the archive binary does not report its version"
if [ -n "$declared_version" ] && [ "$declared_version" != "$version" ]; then
  rm -rf "$stage"
  fatal "metadata declares version $declared_version but the binary reports $version"
fi
info "version:     $version"
install_dir="$APP_DIR/versions/$version"
if [ -e "$install_dir" ]; then
  info "replacing existing $install_dir (verified payload; never an in-place overwrite)"
  rm -rf "$install_dir"
fi
mv "$stage" "$install_dir"

# ---------------------------------------------------------------------------
step "5. update the stable command links"
# ---------------------------------------------------------------------------
mkdir -p "$APP_DIR/bin"
for name in hiero hieronymus hieronymus-agent-hook hieronymus-mcp; do
  ln -sfn "../versions/$version/$name" "$APP_DIR/bin/$name"
done
info "stable links: $APP_DIR/bin/{hiero,hieronymus,hieronymus-agent-hook,hieronymus-mcp}"

mkdir -p "$HOME/.local/bin"
for name in hiero hieronymus hieronymus-agent-hook hieronymus-mcp; do
  dest="$HOME/.local/bin/$name"
  if [ -e "$dest" ] && [ "$(readlink "$dest" 2>/dev/null || true)" != "$APP_DIR/bin/$name" ]; then
    info "replacing existing $dest (the hieronymus link names are managed by this install)"
  fi
  ln -sfn "$APP_DIR/bin/$name" "$dest"
done
info "path links:   $HOME/.local/bin/{hiero,hieronymus,hieronymus-agent-hook,hieronymus-mcp}"

# ---------------------------------------------------------------------------
step "6. install the per-user daemon service"
# ---------------------------------------------------------------------------
service_args=(install --data-root "$DATA_ROOT")
if [ -n "$UNIT_DIR" ]; then
  service_args+=(--unit-dir "$UNIT_DIR")
fi
if [ "$NO_ACTIVATE" = "1" ]; then
  service_args+=(--no-activate)
fi
service_ok=1
"$APP_DIR/bin/hiero" service "${service_args[@]}" || service_ok=0
if [ "$service_ok" = "1" ]; then
  info "service definition is in place (the daemon is NOT started yet; step 8 decides)"
else
  info "warning: service install failed; continuing without the daemon service"
fi

# ---------------------------------------------------------------------------
step "7. run the non-mutating doctor checks"
# ---------------------------------------------------------------------------
doctor_rc=0
"$APP_DIR/bin/hiero" doctor --data-root "$DATA_ROOT" || doctor_rc=$?
case "$doctor_rc" in
  0) info "doctor: healthy" ;;
  1) info "doctor: degraded (see the warnings above); the install is usable" ;;
  2) info "warning: doctor reports an UNHEALTHY installation (see above)" ;;
  *) info "warning: doctor could not run (exit $doctor_rc)" ;;
esac

# ---------------------------------------------------------------------------
step "8. start the daemon only when the schema is compatible"
# ---------------------------------------------------------------------------
state="$("$APP_DIR/bin/hiero" classify --json --data-root "$DATA_ROOT" |
  sed -n 's/.*"state": "\([^"]*\)".*/\1/p')"
can_activate=1
[ "$NO_ACTIVATE" = "0" ] || can_activate=0
[ -z "$UNIT_DIR" ] || can_activate=0
command -v systemctl >/dev/null 2>&1 || can_activate=0
daemon_summary="not started (see step 8 above for the manual command)"
case "$state" in
  empty | rust-schema)
    if [ "$can_activate" = "1" ]; then
      if systemctl --user start hieronymus.service; then
        info "daemon started through the systemd user service"
        daemon_summary="started (systemd user service)"
      else
        info "warning: the service could not be started; run it manually with:"
        info "  \"$APP_DIR/bin/hiero\" daemon --data-root \"$DATA_ROOT\""
        daemon_summary="not started (the service start failed; see step 8 above)"
      fi
    else
      info "daemon not started (no activation); start it manually with:"
      info "  \"$APP_DIR/bin/hiero\" daemon --data-root \"$DATA_ROOT\""
      info "or reinstall the service without --no-activate, then:"
      info "  \"$APP_DIR/bin/hiero\" service start"
    fi
    ;;
  python-schema)
    info "DATABASE UPGRADE REQUIRED: installation completed but the daemon stays stopped."
    info "Run the migration and review its report before starting anything:"
    info "  \"$APP_DIR/bin/hiero\" migrate --data-root \"$DATA_ROOT\""
    info "then start the daemon:"
    info "  \"$APP_DIR/bin/hiero\" service start"
    daemon_summary="stopped — a database upgrade is required (see step 8 above)"
    ;;
  *)
    info "warning: daemon NOT started: database state '$state' is not supported by this release."
    info "Inspect the situation with: \"$APP_DIR/bin/hiero\" doctor --data-root \"$DATA_ROOT\""
    daemon_summary="not started (database state '$state' is not supported)"
    ;;
esac

# ---------------------------------------------------------------------------
printf '\n'
echo "Hieronymus $version installed."
echo "  binaries:   $APP_DIR/versions/$version"
echo "  commands:   $APP_DIR/bin (linked from $HOME/.local/bin)"
echo "  data root:  $DATA_ROOT"
echo "  daemon:     $daemon_summary"
if ! command -v hiero >/dev/null 2>&1; then
  echo
  echo "note: 'hiero' is not on PATH; add $HOME/.local/bin to PATH."
fi
