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
CHANNEL="${HIERONYMUS_RELEASE_CHANNEL:-stable}"
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
    --channel)
      [ $# -ge 2 ] || usage
      CHANNEL="$2"
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
case "$CHANNEL" in stable|dev) ;; *) echo "error: channel must be stable or dev" >&2; exit 1;; esac
if [ -n "$RELEASE_URL" ]; then
  if [[ "$RELEASE_URL" =~ [[:space:]@\\?#] ]]; then
    echo "error: invalid HTTPS release URL (userinfo, whitespace, query or fragment)" >&2
    exit 1
  fi
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
# Bootstrap accepts the flat, unescaped metadata emitted by release-build.sh.
# Reject unsupported JSON representations instead of interpreting them differently
# from the typed verifier. This trusted parser runs before any archive executable.
read_metadata() {
  [ "$(wc -c < "$1")" -le 65536 ] || fatal "release metadata exceeds size limit"
  LC_ALL=C awk -v target="$TARGET" -v channel="$CHANNEL" -v remote="$2" '
    function fail() { print "invalid release metadata (including signature policy)" > "/dev/stderr"; exit 1 }
    function space() { sub(/^[ \t\r\n]*/, "", input) }
    function take(c) { space(); if (substr(input,1,1) != c) fail(); input=substr(input,2) }
    function string( value) {
      space()
      if (!match(input, /^"[A-Za-z0-9_.+-]+"/)) fail()
      value=substr(input,2,RLENGTH-2); input=substr(input,RLENGTH+1)
      return value
    }
    { input=input $0 "\n" }
    END {
      take("{"); space()
      while (substr(input,1,1) != "}") {
        key=string(); if (key !~ /^(version|archive|sha256|signature|channel|target)$/ || seen[key]++) fail()
        take(":"); space()
        if (key == "signature") {
          if (substr(input,1,4) != "null") fail()
          input=substr(input,5)
        } else value[key]=string()
        space()
        if (substr(input,1,1) == "}") break
        take(","); space(); if (substr(input,1,1) == "}") fail()
      }
      take("}"); space(); if (input != "") fail()
      if (value["version"] !~ /^[A-Za-z0-9][A-Za-z0-9.+-]*$/) fail()
      if (value["archive"] != "hieronymus-" value["version"] "-" target ".tar.gz") fail()
      if (length(value["sha256"]) != 64 || value["sha256"] ~ /[^A-Fa-f0-9]/) fail()
      if (seen["target"] && value["target"] != target) fail()
      if (seen["channel"] && value["channel"] !~ /^(stable|dev)$/) fail()
      if (remote && value["channel"] != channel) fail()
      print value["version"]; print value["archive"]; print value["sha256"]
    }
  ' "$1" > "$work/metadata-fields" || fatal "unsupported or invalid release metadata"
  { read -r declared_version; read -r archive_name; read -r expected_sha; } < "$work/metadata-fields"
}

archive_name=""
expected_sha=""
declared_version=""
if [ -n "$RELEASE_DIR" ]; then
  [ -d "$RELEASE_DIR" ] || fatal "release directory does not exist: $RELEASE_DIR"
  if [ -f "$RELEASE_DIR/release.json" ]; then
    info "metadata:    $RELEASE_DIR/release.json"
    read_metadata "$RELEASE_DIR/release.json" 0
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
  RELEASE_URL="${RELEASE_URL%/}/$CHANNEL"
  curl --proto =https --proto-redir =https --max-time 600 --max-filesize 65536 -fsS "$RELEASE_URL/release.json" -o "$work/release.json" ||
    fatal "could not download $RELEASE_URL/release.json"
  read_metadata "$work/release.json" 1
  curl --proto =https --proto-redir =https --max-time 600 --max-filesize 1073741824 -fsS "$RELEASE_URL/$archive_name" -o "$work/$archive_name" ||
    fatal "could not download $RELEASE_URL/$archive_name"
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
actual_sha="$(sha256sum "$archive_path" | awk '{print $1}')"
expected_sha="$(printf '%s' "$expected_sha" | tr 'A-Z' 'a-z')"
[ "$actual_sha" = "$expected_sha" ] ||
  fatal "checksum mismatch for $archive_name: expected $expected_sha, got $actual_sha"
info "sha256 verified: $actual_sha"

# ---------------------------------------------------------------------------
step "4. install into a versioned application directory"
# ---------------------------------------------------------------------------
# Inspect the bootstrap entry before executing any downloaded bytes. It must
# occur exactly once as a regular, bounded file with the literal name hiero.
# The full typed verifier subsequently rejects every other unsafe entry.
# Cap expansion before tar can scan an attacker-controlled stream. Keep one
# bounded uncompressed copy so listing and extraction never decompress again.
# Limits match release_source.rs (3 GiB expanded, 512 MiB executable).
expanded_limit=3221225472
if ! gzip -dc -- "$archive_path" | head -c "$((expanded_limit + 1))" > "$work/payload.tar"; then
  fatal "archive decompression failed or expanded size limit exceeded"
fi
[ "$(wc -c < "$work/payload.tar")" -le "$expanded_limit" ] || fatal "archive expanded size limit exceeded"
# A release has a fixed small entry set. Bound even a many-entry or long-name
# listing before storing it; pipefail also rejects producer errors/SIGPIPE.
if ! LC_ALL=C tar --list --verbose --numeric-owner --quoting-style=escape -f "$work/payload.tar" | head -c 1048577 > "$work/listing"; then
  fatal "archive listing failed or size limit exceeded"
fi
[ "$(wc -c < "$work/listing")" -le 1048576 ] || fatal "archive listing size limit exceeded"
awk '$6 == "hiero" { if (NF != 6 || substr($1,1,1) != "-" || $3 > 536870912) bad=1; count++ } END { exit (bad || count != 1) }' "$work/listing" || fatal "invalid bootstrap executable entry"
if ! tar -xOf "$work/payload.tar" hiero | head -c 536870913 > "$work/hiero"; then
  fatal "bootstrap extraction failed or size limit exceeded"
fi
[ "$(wc -c < "$work/hiero")" -le 536870912 ] || fatal "bootstrap executable exceeds size limit"
chmod 700 "$work/hiero"
if [ -z "$RELEASE_DIR" ]; then RELEASE_DIR="$work"; fi
"$work/hiero" release-verify --release-dir "$RELEASE_DIR" --output "$work/verified" || fatal "release verification failed"
"$work/verified/hiero" release-assets --output "$work/verified" > "$work/assets.json" || fatal "required semantic assets failed native verification"
cmp "$work/verified/assets.json" "$work/assets.json" || fatal "bundled asset metadata mismatch"
version="$("$work/hiero" version --json | sed -n 's/.*"version": "\([^"]*\)".*/\1/p')"
update_args=(update --release-dir "$RELEASE_DIR" --app-dir "$APP_DIR" --data-root "$DATA_ROOT")
if [ -n "$UNIT_DIR" ]; then update_args+=(--unit-dir "$UNIT_DIR"); fi
# A no-activation bootstrap cannot take over a running managed installation.
if [ "$NO_ACTIVATE" = "1" ] && [ -z "$UNIT_DIR" ]; then
  update_args+=(--unit-dir "$work/offline-units")
fi
env -u HIERONYMUS_RELEASE_URL -u HIERONYMUS_RELEASE_DIR "$work/hiero" "${update_args[@]}" || fatal "verified update activation failed"
install_dir="$APP_DIR/versions/$version"
info "version: $version"

# ---------------------------------------------------------------------------
step "5. update the stable command links"
# ---------------------------------------------------------------------------
info "stable links installed by the ownership-checked updater"

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
        if ! "$APP_DIR/bin/hiero" release-ready --data-root "$DATA_ROOT"; then
          systemctl --user stop hieronymus.service || true
          fatal "installed daemon did not reach authenticated semantic readiness; service stopped"
        fi
        info "daemon started and semantic readiness verified through the systemd user service"
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
