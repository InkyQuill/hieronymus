#!/bin/sh
set -eu

REPO_URL="${HIERONYMUS_REPO_URL:-https://github.com/InkyQuill/hieronymus.git}"
APP_DIR="${HIERONYMUS_APP_DIR:-${HOME}/.local/share/hieronymus/app}"

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "error: required command not found: $1" >&2
        exit 1
    fi
}

require_command git

if ! command -v uv >/dev/null 2>&1; then
    require_command curl
    require_command mktemp
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
fi

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

if [ -n "$LATEST_TAG" ]; then
    git -C "$APP_DIR" fetch --tags origin
    git -C "$APP_DIR" checkout --detach "$LATEST_TAG"
    SELECTED_REF="$LATEST_TAG"
else
    git -C "$APP_DIR" fetch origin main
    git -C "$APP_DIR" checkout --detach FETCH_HEAD
    SELECTED_REF="main"
fi

uv tool install --force "$APP_DIR"

echo "Hieronymus installed successfully from ${SELECTED_REF}."
echo "Managed checkout: ${APP_DIR}"

if ! command -v hiero >/dev/null 2>&1; then
    echo "If 'hiero' is not on PATH, add ${HOME}/.local/bin to PATH."
fi
