#!/bin/sh
set -eu

REPO_URL="${HIERONYMUS_REPO_URL:-https://github.com/InkyQuill/hieronymus.git}"
APP_DIR="${HIERONYMUS_APP_DIR:-${HOME}/.local/share/hieronymus/app}"
DATA_ROOT="${HIERONYMUS_DATA_ROOT:-${HOME}/.config/hieronymus}"
case "$DATA_ROOT" in
    "~/"*) DATA_ROOT="${HOME}/${DATA_ROOT#~/}" ;;
esac
RELEASE_CONF="${DATA_ROOT}/release.conf"

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "error: required command not found: $1" >&2
        exit 1
    fi
}

has_tty() {
    [ -r /dev/tty ] && [ -w /dev/tty ] && ( : </dev/tty ) 2>/dev/null && ( : >/dev/tty ) 2>/dev/null
}

confirm() {
    prompt="$1"
    case "${HIERONYMUS_INSTALL_YES:-}" in
        1|true|TRUE|yes|YES) return 0 ;;
        0|false|FALSE|no|NO) return 1 ;;
    esac
    if has_tty; then
        printf "%s [Y/n] " "$prompt" >/dev/tty
        IFS= read -r answer </dev/tty || answer=
        case "$answer" in
            ""|y|Y|yes|YES) return 0 ;;
            *) return 1 ;;
        esac
    fi
    echo "info: no interactive terminal; proceeding automatically. Set HIERONYMUS_INSTALL_YES=0 to abort instead." >&2
    return 0
}

select_install_channel() {
    case "${HIERONYMUS_INSTALL_CHANNEL:-}" in
        stable|STABLE) printf '%s\n' "stable"; return 0 ;;
        dev|DEV) printf '%s\n' "dev"; return 0 ;;
        "")
            ;;
        *)
            echo "error: HIERONYMUS_INSTALL_CHANNEL must be stable or dev" >&2
            exit 1
            ;;
    esac

    if has_tty; then
        echo "Choose Hieronymus install channel:" >/dev/tty
        echo "  1) stable - latest tagged alpha release (recommended)" >/dev/tty
        echo "  2) dev    - latest main commit" >/dev/tty
        printf "Install channel [stable/dev] (stable): " >/dev/tty
        IFS= read -r answer </dev/tty || answer=
        case "$answer" in
            ""|1|stable|STABLE) printf '%s\n' "stable"; return 0 ;;
            2|dev|DEV) printf '%s\n' "dev"; return 0 ;;
            *)
                echo "error: install channel must be stable or dev" >&2
                exit 1
                ;;
        esac
    fi

    echo "info: no interactive terminal; installing stable channel. Set HIERONYMUS_INSTALL_CHANNEL=dev for main." >&2
    printf '%s\n' "stable"
}

write_release_config() {
    channel="$1"
    mkdir -p "$DATA_ROOT"
    cat > "$RELEASE_CONF" <<EOF
[updates]
channel = "$channel"
EOF
}

install_uv() {
    require_command curl
    require_command mktemp
    if ! confirm "uv is missing. Install uv now?"; then
        echo "error: uv is required to install Hieronymus" >&2
        exit 1
    fi
    UV_INSTALLER=$(mktemp "${TMPDIR:-/tmp}/hieronymus-uv-install.XXXXXX")
    trap 'rm -f "$UV_INSTALLER"' EXIT HUP INT TERM
    curl -LsSf https://astral.sh/uv/install.sh -o "$UV_INSTALLER"
    sh "$UV_INSTALLER"
    PATH="${HOME}/.local/bin:${PATH}"
    export PATH
    if ! command -v uv >/dev/null 2>&1; then
        echo "error: uv installation completed but uv was not found on PATH" >&2
        exit 1
    fi
}

install_bun() {
    require_command curl
    require_command bash
    if ! confirm "Bun is missing. Install Bun >= 1.3 now?"; then
        echo "error: Bun >= 1.3 is required to build the Hieronymus web console" >&2
        exit 1
    fi
    curl -fsSL https://bun.sh/install | bash
    PATH="${HOME}/.bun/bin:${PATH}"
    export PATH
    if ! command -v bun >/dev/null 2>&1; then
        echo "error: Bun installation completed but bun was not found on PATH" >&2
        exit 1
    fi
}

ensure_bun() {
    # Verify Bun version >= 1.3
    if ! command -v bun >/dev/null 2>&1; then
        install_bun
    fi

    BUN_VER=$(bun --version)
    BUN_VER_BASE=${BUN_VER%%-*}
    BUN_MAJOR=${BUN_VER_BASE%%.*}
    BUN_REST=${BUN_VER_BASE#*.}
    BUN_MINOR=${BUN_REST%%.*}
    case "$BUN_MAJOR" in ""|*[!0-9]*) BUN_MAJOR=0; BUN_MINOR=0 ;; esac
    case "$BUN_MINOR" in ""|*[!0-9]*) BUN_MAJOR=0; BUN_MINOR=0 ;; esac
    if [ "$BUN_MAJOR" -lt 1 ] || { [ "$BUN_MAJOR" -eq 1 ] && [ "$BUN_MINOR" -lt 3 ]; }; then
        if ! confirm "Bun version is ${BUN_VER}. Upgrade Bun to >= 1.3 now?"; then
            echo "error: Bun >= 1.3 is required to build the Hieronymus web console" >&2
            exit 1
        fi
        bun upgrade
        BUN_VER=$(bun --version)
        BUN_VER_BASE=${BUN_VER%%-*}
        BUN_MAJOR=${BUN_VER_BASE%%.*}
        BUN_REST=${BUN_VER_BASE#*.}
        BUN_MINOR=${BUN_REST%%.*}
        case "$BUN_MAJOR" in ""|*[!0-9]*) BUN_MAJOR=0; BUN_MINOR=0 ;; esac
        case "$BUN_MINOR" in ""|*[!0-9]*) BUN_MAJOR=0; BUN_MINOR=0 ;; esac
        if [ "$BUN_MAJOR" -lt 1 ] || { [ "$BUN_MAJOR" -eq 1 ] && [ "$BUN_MINOR" -lt 3 ]; }; then
            echo "error: Bun version is still ${BUN_VER}; Bun >= 1.3 is required" >&2
            exit 1
        fi
    fi
}

require_command git

# Verify Python version >= 3.12
if ! command -v python3 >/dev/null 2>&1; then
    echo "warning: python3 is not installed. Python >= 3.12 is required to run Hieronymus." >&2
    echo "         Please install Python 3.12 or newer using your system's package manager." >&2
else
    if ! python3 -c "import sys; sys.exit(0 if sys.version_info >= (3, 12) else 1)" >/dev/null 2>&1; then
        PY_VER=$(python3 -c "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}')")
        echo "warning: Python version is ${PY_VER}. Python >= 3.12 is required to run Hieronymus." >&2
        echo "         Please upgrade Python to 3.12 or newer." >&2
    fi
fi

ensure_bun

if ! command -v uv >/dev/null 2>&1; then
    install_uv
fi

INSTALL_CHANNEL=$(select_install_channel)

if [ -d "${APP_DIR}/.git" ]; then
    ORIGIN_URL=$(git -C "$APP_DIR" remote get-url origin)
    if [ "$ORIGIN_URL" != "$REPO_URL" ]; then
        echo "error: existing checkout origin remote does not match ${REPO_URL}" >&2
        exit 1
    fi
    git -C "$APP_DIR" fetch --tags origin
elif [ -e "$APP_DIR" ]; then
    echo "error: ${APP_DIR} exists but is not a git checkout" >&2
    exit 1
else
    mkdir -p "$(dirname "$APP_DIR")"
    git clone "$REPO_URL" "$APP_DIR"
fi

if [ "$INSTALL_CHANNEL" = "stable" ]; then
    REMOTE_TAGS=$(git ls-remote --tags "$REPO_URL")
    LATEST_TAG=$(
        printf '%s\n' "$REMOTE_TAGS" \
            | awk '{
                ref = $2
                sub("^refs/tags/", "", ref)
                sub("\\^\\{\\}$", "", ref)
                if (ref ~ /^v[0-9]+\.[0-9]+\.[0-9]+$/) {
                    print ref
                }
            }' \
            | sort -u \
            | awk '{
                split(substr($0, 2), parts, ".")
                printf "%09d.%09d.%09d %s\n", parts[1], parts[2], parts[3], $0
            }' \
            | sort \
            | tail -n 1 \
            | awk '{ print $2 }'
    )
    if [ -z "$LATEST_TAG" ]; then
        echo "error: stable install requested, but no release tags were found" >&2
        echo "       Set HIERONYMUS_INSTALL_CHANNEL=dev to install latest main." >&2
        exit 1
    fi
    git -C "$APP_DIR" fetch --tags origin
    git -C "$APP_DIR" checkout --detach "$LATEST_TAG"
    SELECTED_REF="$LATEST_TAG"
else
    git -C "$APP_DIR" fetch origin main
    git -C "$APP_DIR" checkout --detach FETCH_HEAD
    SELECTED_REF="main"
fi
write_release_config "$INSTALL_CHANNEL"

uv tool install --force --reinstall "$APP_DIR"

echo "Hieronymus installed successfully from ${SELECTED_REF} (${INSTALL_CHANNEL})."
echo "Managed checkout: ${APP_DIR}"
echo "Update channel config: ${RELEASE_CONF}"

if ! command -v hiero >/dev/null 2>&1; then
    echo "If 'hiero' is not on PATH, add ${HOME}/.local/bin to PATH."
fi
