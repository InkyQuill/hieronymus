from __future__ import annotations

import os
import subprocess
from pathlib import Path
from textwrap import dedent

ROOT = Path(__file__).resolve().parents[1]


def script_text(name: str) -> str:
    return (ROOT / name).read_text(encoding="utf-8")


def write_executable(path: Path, text: str) -> None:
    path.write_text(dedent(text).lstrip(), encoding="utf-8")
    path.chmod(0o755)


def script_env(tmp_path: Path, *, home: Path | None = None) -> dict[str, str]:
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir(exist_ok=True)
    env = os.environ.copy()
    env["HOME"] = str(home or tmp_path / "home")
    env["PATH"] = f"{fake_bin}:{os.environ['PATH']}"
    env["MISE_DISABLE"] = "1"
    return env


def test_install_script_uses_managed_github_checkout() -> None:
    text = script_text("install.sh")

    assert "https://github.com/InkyQuill/hieronymus.git" in text
    assert "${HOME}/.local/share/hieronymus/app" in text
    assert "uv tool install --force" in text
    assert "git ls-remote --tags" in text
    assert "remote get-url origin" in text
    assert "require_command mktemp" in text
    assert 'mktemp "${TMPDIR:-/tmp}/hieronymus-uv-install.XXXXXX"' in text
    assert '-o "$UV_INSTALLER"' in text
    assert "uv installation completed but uv was not found on PATH" in text
    assert "bun install --frozen-lockfile" in text
    assert "bun run build" in text
    assert 'uv tool install --force "$APP_DIR"' in text
    assert "Bun >= 1.3" in text
    assert "HIERONYMUS_INSTALL_YES" in text
    assert "HIERONYMUS_INSTALL_CHANNEL" in text
    assert 'channel = "$channel"' in text


def test_install_script_builds_frontend_before_tool_install_and_writes_stable_channel(
    tmp_path: Path,
) -> None:
    home = tmp_path / "home"
    app_dir = home / ".local" / "share" / "hieronymus" / "app"
    frontend_dir = app_dir / "frontend"
    frontend_dir.mkdir(parents=True)
    (app_dir / ".git").mkdir()
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir()
    command_log = tmp_path / "commands.log"
    write_executable(
        fake_bin / "git",
        f"""
        #!/bin/sh
        echo "git:$PWD:$@" >> "{command_log}"
        if [ "$1" = "-C" ] && [ "$3" = "remote" ] && [ "$4" = "get-url" ]; then
            echo "https://github.com/InkyQuill/hieronymus.git"
            exit 0
        fi
        if [ "$1" = "ls-remote" ]; then
            echo "0000000000000000000000000000000000000000 refs/tags/v1.2.3"
            exit 0
        fi
        exit 0
        """,
    )
    write_executable(
        fake_bin / "uv",
        f"""
        #!/bin/sh
        echo "uv:$PWD:$@" >> "{command_log}"
        exit 0
        """,
    )
    write_executable(
        fake_bin / "python3",
        """
        #!/bin/sh
        if [ "$1" = "-c" ]; then
            if echo "$2" | grep -q "print"; then
                echo "3.12.0"
            fi
            exit 0
        fi
        exit 0
        """,
    )
    write_executable(
        fake_bin / "bun",
        f"""
        #!/bin/sh
        echo "bun:$PWD:$@" >> "{command_log}"
        if [ "$1" = "-e" ]; then
            exit 0
        fi
        if [ "$1" = "--version" ]; then
            echo "1.3.14"
            exit 0
        fi
        exit 0
        """,
    )
    env = script_env(tmp_path, home=home)
    env["HIERONYMUS_INSTALL_YES"] = "1"

    result = subprocess.run(
        ["sh", str(ROOT / "install.sh")],
        check=False,
        env=env,
        capture_output=True,
        text=True,
    )

    assert result.returncode == 0, result.stderr
    commands = command_log.read_text(encoding="utf-8").splitlines()
    assert f"bun:{frontend_dir}:install --frozen-lockfile" in commands
    assert f"bun:{frontend_dir}:run build" in commands
    assert commands.index(f"bun:{frontend_dir}:run build") < commands.index(
        f"uv:{ROOT}:tool install --force {app_dir}"
    )
    assert (home / ".config" / "hieronymus" / "release.conf").read_text(
        encoding="utf-8"
    ) == '[updates]\nchannel = "stable"\n'
    assert "Hieronymus installed successfully from v1.2.3 (stable)." in result.stdout


def test_install_script_dev_channel_checks_out_main_and_writes_release_conf(
    tmp_path: Path,
) -> None:
    home = tmp_path / "home"
    app_dir = home / ".local" / "share" / "hieronymus" / "app"
    frontend_dir = app_dir / "frontend"
    frontend_dir.mkdir(parents=True)
    (app_dir / ".git").mkdir()
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir()
    command_log = tmp_path / "commands.log"
    write_executable(
        fake_bin / "git",
        f"""
        #!/bin/sh
        echo "git:$PWD:$@" >> "{command_log}"
        if [ "$1" = "-C" ] && [ "$3" = "remote" ] && [ "$4" = "get-url" ]; then
            echo "https://github.com/InkyQuill/hieronymus.git"
            exit 0
        fi
        if [ "$1" = "ls-remote" ]; then
            echo "unexpected ls-remote for dev channel" >&2
            exit 2
        fi
        exit 0
        """,
    )
    write_executable(
        fake_bin / "uv",
        f"""
        #!/bin/sh
        echo "uv:$PWD:$@" >> "{command_log}"
        exit 0
        """,
    )
    write_executable(
        fake_bin / "python3",
        """
        #!/bin/sh
        if [ "$1" = "-c" ]; then
            if echo "$2" | grep -q "print"; then
                echo "3.12.0"
            fi
            exit 0
        fi
        exit 0
        """,
    )
    write_executable(
        fake_bin / "bun",
        f"""
        #!/bin/sh
        echo "bun:$PWD:$@" >> "{command_log}"
        if [ "$1" = "--version" ]; then
            echo "1.3.14"
            exit 0
        fi
        exit 0
        """,
    )
    env = script_env(tmp_path, home=home)
    env["HIERONYMUS_INSTALL_YES"] = "1"
    env["HIERONYMUS_INSTALL_CHANNEL"] = "dev"

    result = subprocess.run(
        ["sh", str(ROOT / "install.sh")],
        check=False,
        env=env,
        capture_output=True,
        text=True,
    )

    assert result.returncode == 0, result.stderr
    commands = command_log.read_text(encoding="utf-8").splitlines()
    assert f"git:{ROOT}:-C {app_dir} fetch origin main" in commands
    assert f"git:{ROOT}:-C {app_dir} checkout --detach FETCH_HEAD" in commands
    assert f"bun:{frontend_dir}:run build" in commands
    assert (home / ".config" / "hieronymus" / "release.conf").read_text(
        encoding="utf-8"
    ) == '[updates]\nchannel = "dev"\n'
    assert "Hieronymus installed successfully from main (dev)." in result.stdout


def test_uninstall_script_removes_tool_and_supports_data_modes() -> None:
    text = script_text("uninstall.sh")

    assert "uv tool uninstall hieronymus" in text
    assert "--keep-data" in text
    assert "--purge-data" in text
    assert "Remove Hieronymus settings and data" in text


def test_release_scripts_are_valid_shell() -> None:
    for script in ("install.sh", "uninstall.sh"):
        subprocess.run(["sh", "-n", str(ROOT / script)], check=True)


def test_uninstall_refuses_home_app_dir_and_preserves_files(tmp_path: Path) -> None:
    home = tmp_path / "home"
    home.mkdir()
    sentinel = home / "sentinel.txt"
    sentinel.write_text("keep\n", encoding="utf-8")
    env = script_env(tmp_path, home=home)
    env["HIERONYMUS_APP_DIR"] = str(home)

    result = subprocess.run(
        ["sh", str(ROOT / "uninstall.sh"), "--keep-data"],
        check=False,
        env=env,
        capture_output=True,
        text=True,
    )

    assert result.returncode != 0
    assert "refusing unsafe path" in result.stderr
    assert sentinel.read_text(encoding="utf-8") == "keep\n"


def test_uninstall_refuses_root_data_dir_and_preserves_app(tmp_path: Path) -> None:
    home = tmp_path / "home"
    app_dir = home / ".local" / "share" / "hieronymus" / "app"
    app_dir.mkdir(parents=True)
    sentinel = app_dir / "sentinel.txt"
    sentinel.write_text("keep\n", encoding="utf-8")
    env = script_env(tmp_path, home=home)
    env["HIERONYMUS_DATA_ROOT"] = "/"

    result = subprocess.run(
        ["sh", str(ROOT / "uninstall.sh"), "--purge-data"],
        check=False,
        env=env,
        capture_output=True,
        text=True,
    )

    assert result.returncode != 0
    assert "refusing unsafe path" in result.stderr
    assert sentinel.read_text(encoding="utf-8") == "keep\n"


def test_uninstall_refuses_app_dir_parent_traversal_and_preserves_sibling(
    tmp_path: Path,
) -> None:
    home = tmp_path / "home"
    sibling = home / ".local" / "share" / "hieronymus-keep"
    sibling.mkdir(parents=True)
    sentinel = sibling / "sentinel.txt"
    sentinel.write_text("keep\n", encoding="utf-8")
    env = script_env(tmp_path, home=home)
    env["HIERONYMUS_APP_DIR"] = str(
        home / ".local" / "share" / "hieronymus" / ".." / "hieronymus-keep"
    )

    result = subprocess.run(
        ["sh", str(ROOT / "uninstall.sh"), "--keep-data"],
        check=False,
        env=env,
        capture_output=True,
        text=True,
    )

    assert result.returncode != 0
    assert "refusing unsafe path" in result.stderr
    assert sentinel.read_text(encoding="utf-8") == "keep\n"


def test_uninstall_refuses_data_root_parent_traversal_and_preserves_sibling(
    tmp_path: Path,
) -> None:
    home = tmp_path / "home"
    sibling = home / ".config" / "hieronymus-keep"
    sibling.mkdir(parents=True)
    sentinel = sibling / "sentinel.txt"
    sentinel.write_text("keep\n", encoding="utf-8")
    env = script_env(tmp_path, home=home)
    env["HIERONYMUS_DATA_ROOT"] = str(home / ".config" / "hieronymus" / ".." / "hieronymus-keep")

    result = subprocess.run(
        ["sh", str(ROOT / "uninstall.sh"), "--purge-data"],
        check=False,
        env=env,
        capture_output=True,
        text=True,
    )

    assert result.returncode != 0
    assert "refusing unsafe path" in result.stderr
    assert sentinel.read_text(encoding="utf-8") == "keep\n"


def test_uninstall_keep_data_removes_safe_app_and_keeps_safe_data(tmp_path: Path) -> None:
    home = tmp_path / "home"
    app_dir = home / ".local" / "share" / "hieronymus" / "app"
    data_dir = home / ".config" / "hieronymus"
    app_dir.mkdir(parents=True)
    data_dir.mkdir(parents=True)
    app_file = app_dir / "app.txt"
    data_file = data_dir / "data.txt"
    app_file.write_text("remove\n", encoding="utf-8")
    data_file.write_text("keep\n", encoding="utf-8")
    env = script_env(tmp_path, home=home)

    result = subprocess.run(
        ["sh", str(ROOT / "uninstall.sh"), "--keep-data"],
        check=False,
        env=env,
        capture_output=True,
        text=True,
    )

    assert result.returncode == 0
    assert not app_dir.exists()
    assert data_file.read_text(encoding="utf-8") == "keep\n"


def test_install_refuses_existing_checkout_with_wrong_origin_before_fetch(
    tmp_path: Path,
) -> None:
    home = tmp_path / "home"
    app_dir = home / ".local" / "share" / "hieronymus" / "app"
    app_dir.mkdir(parents=True)
    (app_dir / ".git").mkdir()
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir()
    git_log = tmp_path / "git.log"
    write_executable(
        fake_bin / "git",
        f"""
        #!/bin/sh
        echo "$@" >> "{git_log}"
        if [ "$1" = "-C" ] && [ "$3" = "remote" ] && [ "$4" = "get-url" ]; then
            echo "https://example.invalid/other.git"
            exit 0
        fi
        if [ "$1" = "-C" ] && [ "$3" = "fetch" ]; then
            echo "fetch should not run" >&2
            exit 7
        fi
        exit 0
        """,
    )
    write_executable(
        fake_bin / "uv",
        """
        #!/bin/sh
        exit 0
        """,
    )
    write_executable(
        fake_bin / "python3",
        """
        #!/bin/sh
        if [ "$1" = "-c" ]; then
            if echo "$2" | grep -q "print"; then
                echo "3.12.0"
            fi
            exit 0
        fi
        exit 0
        """,
    )
    write_executable(
        fake_bin / "bun",
        """
        #!/bin/sh
        if [ "$1" = "-e" ]; then
            exit 0
        fi
        echo "1.3.14"
        exit 0
        """,
    )
    env = script_env(tmp_path, home=home)

    result = subprocess.run(
        ["sh", str(ROOT / "install.sh")],
        check=False,
        env=env,
        capture_output=True,
        text=True,
    )

    assert result.returncode != 0
    assert "origin remote" in result.stderr
    assert "fetch" not in git_log.read_text(encoding="utf-8")
