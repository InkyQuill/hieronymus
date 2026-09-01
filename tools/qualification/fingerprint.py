"""Byte-exact, repository-relative qualification input fingerprints."""

from __future__ import annotations

import hashlib
import os
import stat
from collections.abc import Mapping
from pathlib import Path, PurePosixPath, PureWindowsPath
from types import MappingProxyType

from tools.qualification.model import Risk

COMMON_FINGERPRINT_INPUTS = (
    "qualification/prerequisites.json",
    "qualification/rust-toolchain.toml",
    "tools/qualification/model.py",
    "tools/qualification/fingerprint.py",
    "tools/qualification/redaction.py",
    "tools/qualification/validate.py",
    "tools/qualification/render.py",
    "tools/qualification/acquire.py",
    "tools/qualification/process.py",
    "tools/qualification/clean.py",
)

_MCP_TOOL_NAMES = (
    "hieronymus_concept_archive",
    "hieronymus_concept_create",
    "hieronymus_concept_facet_add",
    "hieronymus_concept_facet_list",
    "hieronymus_concept_facet_set_canonical",
    "hieronymus_concept_facet_update",
    "hieronymus_concept_get",
    "hieronymus_concept_list",
    "hieronymus_concept_merge",
    "hieronymus_concept_proposals_list",
    "hieronymus_concept_rename",
    "hieronymus_concept_semantic_tags_set",
    "hieronymus_concept_update",
    "hieronymus_crystal_link_concept",
    "hieronymus_crystal_semantic_tags_set",
    "hieronymus_crystal_story_scopes_set",
    "hieronymus_dream",
    "hieronymus_feedback",
    "hieronymus_memory_add",
    "hieronymus_memory_search",
    "hieronymus_rag_import",
    "hieronymus_rag_search",
    "hieronymus_recall",
    "hieronymus_rule_crystal_archive",
    "hieronymus_rule_crystal_validate",
    "hieronymus_rule_crystals_list",
    "hieronymus_series_create",
    "hieronymus_series_init",
    "hieronymus_series_list",
    "hieronymus_series_set_language_tags",
    "hieronymus_session_complete",
    "hieronymus_session_start",
    "hieronymus_short_term_add",
    "hieronymus_short_term_add_batch",
    "hieronymus_status",
    "hieronymus_termbase_approve",
    "hieronymus_termbase_contract",
    "hieronymus_termbase_propose",
    "hieronymus_termbase_validate",
)
_MCP_TOOL_FIXTURE_LEAVES = (
    "error.input.json",
    "success.input.json",
    "wire.error.json",
    "wire.success.json",
)
MCP_TOOL_INPUT_WIRE_INPUTS = tuple(
    f"compatibility/fixtures/mcp/{tool}/{leaf}"
    for tool in _MCP_TOOL_NAMES
    for leaf in _MCP_TOOL_FIXTURE_LEAVES
)

_FRONTEND_SOURCE_INPUTS = (
    "frontend/src/web/App.svelte",
    "frontend/src/web/app.css",
    "frontend/src/web/app.test.ts",
    "frontend/src/web/components/AdminDashboard.svelte",
    "frontend/src/web/components/DreamingEditor.svelte",
    "frontend/src/web/components/IngestEditor.svelte",
    "frontend/src/web/components/MemoryViews.svelte",
    "frontend/src/web/components/MemoryViews.test.ts",
    "frontend/src/web/components/ProviderEditor.svelte",
    "frontend/src/web/components/ReleaseEditor.svelte",
    "frontend/src/web/components/Toast.svelte",
    "frontend/src/web/components/editors.test.ts",
    "frontend/src/web/fonts.css",
    "frontend/src/web/fonts/geist.woff2",
    "frontend/src/web/fonts/inconsolatalgc.woff2",
    "frontend/src/web/fonts/literata.woff2",
    "frontend/src/web/lib/admin-events.svelte.ts",
    "frontend/src/web/lib/api.ts",
    "frontend/src/web/lib/theme.svelte.test.ts",
    "frontend/src/web/lib/theme.svelte.ts",
    "frontend/src/web/lib/types.ts",
    "frontend/src/web/main.ts",
    "frontend/src/web/test/setup.ts",
)

RISK_FINGERPRINT_SUFFIXES: Mapping[Risk, tuple[str, ...]] = MappingProxyType(
    {
        "mcp-transport": (
            "tools/qualification/run_mcp.py",
            "qualification/harnesses/mcp-transport/Cargo.toml",
            "qualification/harnesses/mcp-transport/Cargo.lock",
            "qualification/harnesses/mcp-transport/src/main.rs",
            "qualification/harnesses/mcp-transport/src/registry.rs",
            "qualification/harnesses/mcp-transport/src/report.rs",
            "qualification/harnesses/mcp-transport/tests/transport.rs",
            "compatibility/manifest.json",
            "compatibility/authorities/mcp/2026-07-28/schema.json",
            "compatibility/authorities/mcp/2026-07-28/schema.source.json",
            "compatibility/snapshots/mcp.json",
            "compatibility/fixtures/mcp/protocol.json",
            "compatibility/fixtures/http/route-cases.json",
            *MCP_TOOL_INPUT_WIRE_INPUTS,
        ),
        "semantic-native": (
            "tools/qualification/run_semantic.py",
            "qualification/harnesses/semantic-native/Cargo.toml",
            "qualification/harnesses/semantic-native/Cargo.lock",
            "qualification/harnesses/semantic-native/src/lib.rs",
            "qualification/harnesses/semantic-native/src/corpus.rs",
            "qualification/harnesses/semantic-native/src/model.rs",
            "qualification/harnesses/semantic-native/src/index.rs",
            "qualification/harnesses/semantic-native/src/main.rs",
            "qualification/harnesses/semantic-native/src/scenario.rs",
            "qualification/harnesses/semantic-native/src/fts.rs",
            "qualification/harnesses/semantic-native/tests/corpus.rs",
            "qualification/harnesses/semantic-native/tests/index.rs",
            "qualification/harnesses/semantic-native/tests/recovery.rs",
            "qualification/harnesses/semantic-native/tests/fts.rs",
            "qualification/fixtures/semantic-corpus.json",
            "compatibility/fixtures/mcp/hieronymus_rag_search/success.input.json",
            "compatibility/fixtures/mcp/hieronymus_recall/success.input.json",
        ),
        "frontend-embedding": (
            "tools/qualification/run_frontend.py",
            "qualification/harnesses/frontend-embedding/Cargo.toml",
            "qualification/harnesses/frontend-embedding/Cargo.lock",
            "qualification/harnesses/frontend-embedding/build.rs",
            "qualification/harnesses/frontend-embedding/src/main.rs",
            "qualification/harnesses/frontend-embedding/src/assets.rs",
            "qualification/harnesses/frontend-embedding/tests/assets.rs",
            "frontend/index.html",
            "frontend/package.json",
            "frontend/bun.lock",
            "frontend/tsconfig.json",
            "frontend/vite.config.ts",
            *_FRONTEND_SOURCE_INPUTS,
            "compatibility/fixtures/http/route-cases.json",
        ),
        "legacy-database-import": (
            "tools/qualification/run_database.py",
            "qualification/harnesses/legacy-database-import/Cargo.toml",
            "qualification/harnesses/legacy-database-import/Cargo.lock",
            "qualification/harnesses/legacy-database-import/src/main.rs",
            "qualification/harnesses/legacy-database-import/src/classify.rs",
            "qualification/harnesses/legacy-database-import/src/probe_import.rs",
            "qualification/harnesses/legacy-database-import/src/report.rs",
            "qualification/harnesses/legacy-database-import/tests/fixtures.rs",
            "compatibility/manifest.json",
            "compatibility/snapshots/state.json",
            "compatibility/fixtures/database/corrupt.sqlite",
            "compatibility/fixtures/database/empty.sqlite",
            "compatibility/fixtures/database/legacy-python.sqlite",
            "compatibility/fixtures/database/minimal-python.sqlite",
            "compatibility/fixtures/database/partial-python.sqlite",
            "compatibility/fixtures/database/unknown-schema.sqlite",
        ),
    }
)

_DOMAIN = b"hieronymus qualification input fingerprint\x00v1\x00"
_DIRECTORY_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | getattr(os, "O_CLOEXEC", 0)
_FILE_FLAGS = (
    os.O_RDONLY
    | os.O_NOFOLLOW
    | os.O_NONBLOCK
    | getattr(os, "O_CLOEXEC", 0)
    | getattr(os, "O_BINARY", 0)
)


def required_fingerprint_inputs(risk: Risk) -> tuple[str, ...]:
    """Return the one exact ordered common-plus-risk input policy."""
    try:
        suffix = RISK_FINGERPRINT_SUFFIXES[risk]
    except (KeyError, TypeError) as error:
        raise ValueError("unknown qualification fingerprint risk") from error
    return (*COMMON_FINGERPRINT_INPUTS, *suffix)


def fingerprint_inputs(repo_root: Path, paths: tuple[str, ...]) -> str:
    """Hash sorted canonical names and raw bytes through one no-follow root fd."""
    if type(paths) is not tuple:
        raise ValueError("fingerprint paths must be an ordered tuple")
    canonical = tuple(_canonical_relative_path(path) for path in paths)
    if len(canonical) != len(set(canonical)):
        raise ValueError("fingerprint paths must not contain duplicates")

    root_descriptor = _open_repository_root(repo_root)
    seen_inodes: set[tuple[int, int]] = set()
    digest = hashlib.sha256(_DOMAIN)
    try:
        for relative in sorted(canonical):
            content, inode = _read_regular_file(root_descriptor, relative)
            if inode in seen_inodes:
                raise ValueError(
                    f"hard-link alias fingerprint input {relative!r} duplicates another input"
                )
            seen_inodes.add(inode)
            name = relative.encode("utf-8")
            digest.update(len(name).to_bytes(8, "big"))
            digest.update(name)
            digest.update(len(content).to_bytes(16, "big"))
            digest.update(content)
    finally:
        os.close(root_descriptor)
    return digest.hexdigest()


def _canonical_relative_path(value: object) -> str:
    if type(value) is not str or not value:
        raise ValueError("fingerprint paths must be nonempty strings")
    windows = PureWindowsPath(value)
    if windows.is_absolute() or windows.drive or windows.root:
        raise ValueError("fingerprint paths must be relative")
    if "\\" in value:
        raise ValueError("fingerprint paths must use canonical POSIX separators")

    path = PurePosixPath(value)
    if path.is_absolute():
        raise ValueError("fingerprint paths must be relative")
    if path.as_posix() != value or any(part in (".", "..") for part in path.parts):
        raise ValueError("fingerprint paths must be canonical relative paths")
    if "\x00" in value:
        raise ValueError("fingerprint paths must not contain NUL")
    return value


def _open_repository_root(repo_root: Path) -> int:
    """Open every lexical root component without following a symlink alias."""
    try:
        absolute = os.path.abspath(os.fspath(repo_root))
    except (TypeError, ValueError, OSError) as error:
        raise ValueError("fingerprint repository root is invalid") from error
    descriptor: int | None = None
    try:
        descriptor = os.open(os.sep, _DIRECTORY_FLAGS)
        for part in Path(absolute).parts[1:]:
            next_descriptor = os.open(part, _DIRECTORY_FLAGS, dir_fd=descriptor)
            os.close(descriptor)
            descriptor = next_descriptor
        if not stat.S_ISDIR(os.fstat(descriptor).st_mode):
            raise OSError("not a directory")
        return descriptor
    except (OSError, ValueError) as error:
        if descriptor is not None:
            os.close(descriptor)
        raise ValueError("fingerprint repository root is invalid") from error


def _read_regular_file(root_descriptor: int, relative: str) -> tuple[bytes, tuple[int, int]]:
    """Open and read one input through descriptor-relative no-follow operations."""
    directory_descriptor = os.dup(root_descriptor)
    file_descriptor: int | None = None
    try:
        parts = PurePosixPath(relative).parts
        for part in parts[:-1]:
            try:
                next_descriptor = os.open(
                    part,
                    _DIRECTORY_FLAGS,
                    dir_fd=directory_descriptor,
                )
            except OSError as error:
                raise _input_error(relative) from error
            os.close(directory_descriptor)
            directory_descriptor = next_descriptor
        try:
            file_descriptor = os.open(
                parts[-1],
                _FILE_FLAGS,
                dir_fd=directory_descriptor,
            )
        except OSError as error:
            raise _input_error(relative) from error

        before = os.fstat(file_descriptor)
        if not stat.S_ISREG(before.st_mode):
            raise _input_error(relative)
        chunks: list[bytes] = []
        while True:
            try:
                chunk = os.read(file_descriptor, 1024 * 1024)
            except OSError as error:
                raise _input_error(relative) from error
            if not chunk:
                break
            chunks.append(chunk)
        after = os.fstat(file_descriptor)
        if (
            (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino)
            or not stat.S_ISREG(after.st_mode)
            or before.st_size != after.st_size
            or sum(map(len, chunks)) != after.st_size
        ):
            raise ValueError(f"fingerprint input {relative!r} changed while being read")
        return b"".join(chunks), (after.st_dev, after.st_ino)
    except ValueError:
        raise
    except OSError as error:
        raise _input_error(relative) from error
    finally:
        if file_descriptor is not None:
            os.close(file_descriptor)
        os.close(directory_descriptor)


def _input_error(relative: str) -> ValueError:
    return ValueError(f"fingerprint input {relative!r} must be an existing regular file")
