#!/usr/bin/env bash
# Offline v2 desktop bootstrap. Only checksum-authenticated CLI bytes execute.
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
release=""; app=""; data=""; unit=""; no_activate=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --release-dir) release="${2:?--release-dir needs a path}"; shift 2;;
    --app-dir) app="${2:?--app-dir needs a path}"; shift 2;;
    --data-root) data="${2:?--data-root needs a path}"; shift 2;;
    --unit-dir) unit="${2:?--unit-dir needs a path}"; shift 2;;
    --no-activate) no_activate=1; shift;;
    *) echo 'usage: install-desktop.sh --release-dir DIR [--app-dir DIR --data-root DIR --unit-dir DIR --no-activate]' >&2; exit 2;;
  esac
done
[ -n "$release" ] || { echo '--release-dir is required; no default channel URL is configured' >&2; exit 2; }
case "$(uname -s)/$(uname -m)" in
  Linux/x86_64) target=x86_64-unknown-linux-gnu; app="${app:-$HOME/.local/share/hieronymus/app}"; data="${data:-$HOME/.config/hieronymus}";;
  Darwin/arm64) target=aarch64-apple-darwin; app="${app:-$HOME/Library/Application Support/Hieronymus/app}"; data="${data:-$HOME/Library/Application Support/Hieronymus}";;
  Darwin/x86_64) target=x86_64-apple-darwin; app="${app:-$HOME/Library/Application Support/Hieronymus/app}"; data="${data:-$HOME/Library/Application Support/Hieronymus}";;
  *) echo 'Unsupported desktop OS/architecture' >&2; exit 2;;
esac
absolute(){ case "$1" in /*) printf '%s\n' "$1";; *) printf '%s/%s\n' "$PWD" "$1";; esac; }
release="$(absolute "$release")"; app="$(absolute "$app")"; data="$(absolute "$data")"
metadata="$release/release-$target.json"
if [ ! -e "$metadata" ] && [ "$target" = x86_64-unknown-linux-gnu ] && [ -f "$release/release.json" ]; then
  legacy=(--desktop --release-dir "$release" --app-dir "$app" --data-root "$data")
  if [ -n "$unit" ]; then legacy+=(--unit-dir "$unit"); fi
  if [ "$no_activate" = 1 ]; then legacy+=(--no-activate); fi
  exec "$script_dir/install.sh" "${legacy[@]}"
fi
[ -f "$metadata" ] && [ ! -L "$metadata" ] || { echo "Missing regular exact-target metadata: $metadata" >&2; exit 2; }
work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
head -c 65537 "$metadata" > "$work/release-$target.json"
[ "$(wc -c < "$work/release-$target.json")" -le 65536 ] || exit 2
LC_ALL=C awk -v target="$target" -f "$script_dir/desktop-metadata.awk" "$work/release-$target.json" > "$work/fields"
{ read -r version; read -r platform; read -r platform_hash; read -r model; read -r model_hash; } < "$work/fields"
hash(){ if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi; }
verify(){ [ -f "$1" ] && [ ! -L "$1" ] && [ "$(wc -c < "$1")" -le 1073741824 ] && [ "$(hash "$1")" = "$2" ] || { echo "Missing, oversized or mismatched release artifact: $1" >&2; exit 2; }; }
# Copy to private bootstrap snapshots before hashing/extracting. Rust subsequently verifies every member.
verify "$release/$platform" "$platform_hash"
head -c 1073741825 "$release/$platform" > "$work/$platform"; verify "$work/$platform" "$platform_hash"
cache="$app/cache/models/$model_hash.tar.gz"
if [ -e "$cache" ]; then verify "$cache" "$model_hash"; fi
if [ -e "$release/$model" ]; then verify "$release/$model" "$model_hash"; model_source="$release/$model"; else model_source="$cache"; fi
verify "$model_source" "$model_hash"; head -c 1073741825 "$model_source" > "$work/$model"; verify "$work/$model" "$model_hash"
# Bound decompression and inspect the one bootstrap executable before native execution.
gzip -dc "$work/$platform" | head -c 3221225473 > "$work/platform.tar"
[ "$(wc -c < "$work/platform.tar")" -le 3221225472 ] || { echo 'Expanded archive exceeds bound' >&2; exit 2; }
LC_ALL=C tar -tvf "$work/platform.tar" | head -c 1048577 > "$work/list"
[ "$(wc -c < "$work/list")" -le 1048576 ] || exit 2
awk '$NF=="hiero" {n++;if(substr($1,1,1)!="-")bad=1}END{exit (bad||n!=1)}' "$work/list" || { echo 'Invalid bootstrap executable entry' >&2; exit 2; }
tar -xOf "$work/platform.tar" hiero | head -c 536870913 > "$work/hiero"
[ "$(wc -c < "$work/hiero")" -le 536870912 ] || exit 2
chmod 700 "$work/hiero"
"$work/hiero" release-verify --release-dir "$work"
args=(desktop-bootstrap --release-dir "$work" --app-dir "$app" --data-root "$data")
if [ -n "$unit" ]; then args+=(--unit-dir "$(absolute "$unit")"); fi
if [ "$no_activate" = 1 ]; then args+=(--no-activate); fi
"$work/hiero" "${args[@]}"
printf 'Installed Hieronymus %s. Commands: %s/bin. Signing: unsigned.\n' "$version" "$app"
