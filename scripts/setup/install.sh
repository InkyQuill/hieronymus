#!/usr/bin/env bash
# Generated standalone installer. No source checkout or language runtime needed.
set -euo pipefail
main() {
  local app='' data='' unit='' source='' no_activate=0 no_open=0
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --app-dir) app="${2:?Missing application directory}"; shift 2;;
      --data-root) data="${2:?Missing data directory}"; shift 2;;
      --unit-dir) unit="${2:?Missing service directory}"; shift 2;;
      --release-dir) source="${2:?Missing offline directory}"; shift 2;;
      --no-activate) no_activate=1; no_open=1; shift;;
      --no-open) no_open=1; shift;;
      --help) printf 'Install Hieronymus @@VERSION@@ for this computer. No arguments needed.\nAdvanced: --app-dir DIR --data-root DIR --unit-dir DIR --release-dir DIR --no-activate --no-open\n'; return;;
      *) printf 'Unknown option: %s\nRun with --help for options.\n' "$1" >&2; return 2;;
    esac
  done
  local target
  case "$(uname -s)/$(uname -m)" in
    Linux/x86_64) target=x86_64-unknown-linux-gnu; app="${app:-$HOME/.local/share/hieronymus/app}"; data="${data:-$HOME/.config/hieronymus}";;
    Darwin/arm64) target=aarch64-apple-darwin; app="${app:-$HOME/Library/Application Support/Hieronymus/app}"; data="${data:-$HOME/Library/Application Support/Hieronymus}";;
    Darwin/x86_64)
      # A Terminal running under Rosetta still needs the Apple Silicon build.
      if [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ]; then target=aarch64-apple-darwin; else target=x86_64-apple-darwin; fi
      app="${app:-$HOME/Library/Application Support/Hieronymus/app}"; data="${data:-$HOME/Library/Application Support/Hieronymus}";;
    *) printf 'This computer is not supported. Hieronymus needs Linux x86_64 or an Apple Silicon/Intel Mac.\n' >&2; return 2;;
  esac
  case "$app" in /*) ;; *) app="$PWD/$app";; esac
  case "$data" in /*) ;; *) data="$PWD/$data";; esac
  local command
  for command in bash awk tar gzip head wc mktemp; do
    command -v "$command" >/dev/null || { printf 'Please install %s and try again.\n' "$command" >&2; return 2; }
  done
  if ! command -v sha256sum >/dev/null && ! command -v shasum >/dev/null; then printf 'A SHA-256 checksum tool is required.\n' >&2; return 2; fi
  if [ -z "$source" ]; then command -v curl >/dev/null || { printf 'Please install curl and try again.\n' >&2; return 2; }; fi
  # Keep work in the enclosing subshell so the EXIT trap can see it after return.
  work="$(mktemp -d)"
  # Run cleanup in this subshell, including when a download or install fails.
  trap 'rm -rf "$work"' EXIT
  local platform platform_hash model model_hash
@@PAYLOAD@@
  printf 'Installing Hieronymus @@VERSION@@…\n'
  if [ "$target" = x86_64-apple-darwin ]; then printf 'Intel Mac build: desktop testing is incomplete.\n'; fi
  local name expected actual cached
  for name in "$platform" "$model"; do
    if [ "$name" = "$platform" ]; then expected="$platform_hash"; printf 'Downloading the app…\n'; else expected="$model_hash"; printf 'Downloading the memory model (this can take a few minutes)…\n'; fi
    cached=''
    if [ -n "$source" ]; then cached="$source/$name"
    elif [ -f "$app/cache/downloads/$name" ]; then cached="$app/cache/downloads/$name"
    elif [ "$name" = "$model" ] && [ -f "$app/cache/models/$model_hash.tar.gz" ]; then cached="$app/cache/models/$model_hash.tar.gz"
    fi
    if [ -n "$cached" ]; then
      [ -f "$cached" ] && [ ! -L "$cached" ] || { printf 'Invalid cached download: %s\n' "$name" >&2; return 1; }
      head -c 1073741825 "$cached" > "$work/$name"
    else
      curl --proto '=https' --proto-redir '=https' --location --fail --silent --show-error --retry 3 --connect-timeout 20 --max-time 900 --max-filesize 1073741824 "@@RELEASE_URL@@/$name" --output "$work/$name" || { printf 'Download failed. Please check your connection and run the installer again.\n' >&2; return 1; }
    fi
    [ "$(wc -c < "$work/$name")" -le 1073741824 ] || { printf 'Download is larger than expected.\n' >&2; return 1; }
    if command -v sha256sum >/dev/null; then actual="$(sha256sum "$work/$name" | awk '{print $1}')"; else actual="$(shasum -a 256 "$work/$name" | awk '{print $1}')"; fi
    [ "$actual" = "$expected" ] || { printf 'The download could not be verified. Nothing was installed. Please try again.\n' >&2; return 1; }
  done
  local args=(--release-dir "$work" --app-dir "$app" --data-root "$data")
  if [ -n "$unit" ]; then args+=(--unit-dir "$unit"); fi
  if [ "$no_activate" = 1 ]; then args+=(--no-activate); fi
  printf 'Verifying and installing…\n'
  if ! bash "$work/offline.sh" "${args[@]}" > "$work/install.log" 2>&1; then
    printf 'Installation needs attention:\n' >&2; cat "$work/install.log" >&2; return 1
  fi
  printf 'Hieronymus is installed.\n'
  if [ "$no_open" = 0 ]; then
    printf 'Opening Hieronymus. Choose “Connect your agent” to finish setup.\n'
    if ! "$app/bin/hiero" admin --data-root "$data" > "$work/open.log" 2>&1; then
      printf 'Could not open the web interface automatically. Use the Hieronymus tray icon to open it.\n' >&2
      cat "$work/install.log" "$work/open.log" >&2
    fi
  fi
}
# Parsing the complete script before starting makes curl | bash safe from stdin reads.
( main "$@" ) < /dev/null
