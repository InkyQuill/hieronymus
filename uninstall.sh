#!/bin/sh
set -eu

APP_DIR="${HIERONYMUS_APP_DIR:-${HOME}/.local/share/hieronymus/app}"
DATA_DIR="${HIERONYMUS_DATA_ROOT:-${HOME}/.config/hieronymus}"
DATA_MODE="prompt"

strip_trailing_slashes() {
    path=$1
    while [ "$path" != "/" ]; do
        trimmed=${path%/}
        if [ "$trimmed" = "$path" ]; then
            break
        fi
        path=$trimmed
    done
    printf '%s\n' "$path"
}

validate_hieronymus_path() {
    label=$1
    path=$(strip_trailing_slashes "$2")

    case "$path" in
        "" | "/" | "." | "$HOME")
            echo "error: refusing unsafe path for ${label}: ${path}" >&2
            exit 1
            ;;
        ".." | "../"* | *"/.." | *"/../"*)
            echo "error: refusing unsafe path for ${label}: ${path}" >&2
            exit 1
            ;;
        /*)
            ;;
        *)
            echo "error: refusing unsafe path for ${label}: ${path}" >&2
            exit 1
            ;;
    esac

    case "$path" in
        "$HOME/.local/share/hieronymus" | "$HOME/.local/share/hieronymus/"* | \
            "$HOME/.config/hieronymus" | "$HOME/.config/hieronymus/"* | \
            */hieronymus | */hieronymus/*)
            ;;
        *)
            echo "error: refusing unsafe path for ${label}: ${path}" >&2
            exit 1
            ;;
    esac
}

usage() {
    echo "usage: $0 [--keep-data|--purge-data]" >&2
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --keep-data)
            DATA_MODE="keep"
            ;;
        --purge-data)
            DATA_MODE="purge"
            ;;
        *)
            echo "error: unknown option: $1" >&2
            usage
            exit 1
            ;;
    esac
    shift
done

validate_hieronymus_path "APP_DIR" "$APP_DIR"
validate_hieronymus_path "DATA_DIR" "$DATA_DIR"

if command -v uv >/dev/null 2>&1; then
    uv tool uninstall hieronymus >/dev/null 2>&1 || true
fi

rm -rf "$APP_DIR"

if [ "$DATA_MODE" = "keep" ]; then
    echo "Kept Hieronymus settings and data: ${DATA_DIR}"
elif [ "$DATA_MODE" = "purge" ]; then
    rm -rf "$DATA_DIR"
    echo "Removed Hieronymus settings and data: ${DATA_DIR}"
elif [ -t 0 ]; then
    printf "Remove Hieronymus settings and data at %s? [y/N] " "$DATA_DIR"
    read -r REPLY
    case "$REPLY" in
        y | Y | yes | YES)
            rm -rf "$DATA_DIR"
            echo "Removed Hieronymus settings and data: ${DATA_DIR}"
            ;;
        *)
            echo "Kept Hieronymus settings and data: ${DATA_DIR}"
            ;;
    esac
else
    echo "Kept Hieronymus settings and data: ${DATA_DIR}"
fi

echo "Hieronymus uninstalled."
