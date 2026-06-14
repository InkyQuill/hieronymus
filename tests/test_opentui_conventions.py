from __future__ import annotations

import re
from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.tui_bridge.admin_api import AdminBridge
from hieronymus.tui_bridge.config_api import ConfigBridge
from hieronymus.tui_bridge.server import _handlers

ROOT = Path(__file__).resolve().parents[1]
FRONTEND_SRC = ROOT / "frontend" / "src"
PYTHON_SRC = ROOT / "src"
BRIDGE_CLIENT = Path("frontend/src/rpc/client.ts")

IMPORT_SPECIFIER_RE = re.compile(
    r"""
    (?:
        \bimport\s+(?:type\s+)?(?:[^'"]+\s+from\s+)?|
        \bexport\s+(?:type\s+)?(?:[^'"]+\s+from\s+)?
    )
    (?P<quote>["'])(?P<specifier>[^"']+)(?P=quote)
    """,
    re.VERBOSE,
)
MODULE_CALL_RE = re.compile(
    r"""
    (?:
        \bimport\s*\(|
        \brequire\s*\(
    )
    \s*(?P<quote>["'])(?P<specifier>[^"']+)(?P=quote)
    """,
    re.VERBOSE,
)
RPC_METHOD_RE = re.compile(
    r'\bmethod:\s*["\'](?P<method>(?:admin|config)\.[^"\']+)["\']'
)


def _frontend_sources() -> list[Path]:
    return sorted(FRONTEND_SRC.rglob("*.ts")) + sorted(FRONTEND_SRC.rglob("*.tsx"))


def _relative(path: Path) -> str:
    return path.relative_to(ROOT).as_posix()


def _import_specifiers(text: str) -> list[str]:
    static_imports = [
        match.group("specifier") for match in IMPORT_SPECIFIER_RE.finditer(text)
    ]
    module_calls = [match.group("specifier") for match in MODULE_CALL_RE.finditer(text)]
    return static_imports + module_calls


def _frontend_rpc_methods() -> set[str]:
    methods: set[str] = set()
    for path in _frontend_sources():
        text = path.read_text(encoding="utf-8")
        methods.update(match.group("method") for match in RPC_METHOD_RE.finditer(text))
    return methods


def _resolves_inside_python_src(path: Path, specifier: str) -> bool:
    if not specifier.startswith("."):
        return False
    target = (path.parent / specifier).resolve()
    return target == PYTHON_SRC or PYTHON_SRC in target.parents


def test_frontend_does_not_import_sqlite_or_backend_python_modules() -> None:
    forbidden_import_fragments = (
        "sqlite",
        "better-sqlite",
        "hieronymus.",
        "hieronymus/",
    )
    offenders: list[str] = []

    for path in _frontend_sources():
        text = path.read_text(encoding="utf-8")
        for specifier in _import_specifiers(text):
            if _resolves_inside_python_src(path, specifier) or any(
                fragment in specifier for fragment in forbidden_import_fragments
            ):
                offenders.append(f"{_relative(path)} imports {specifier!r}")

    assert offenders == []


def test_frontend_does_not_parse_human_hiero_cli_output() -> None:
    offenders: list[str] = []

    for path in _frontend_sources():
        text = path.read_text(encoding="utf-8")
        relative = Path(_relative(path))
        if "tui-bridge" in text and relative != BRIDGE_CLIENT:
            offenders.append(f"{relative.as_posix()} references tui-bridge directly")
        if relative != BRIDGE_CLIENT:
            for specifier in _import_specifiers(text):
                if specifier in {"node:child_process", "child_process"}:
                    offenders.append(f"{relative.as_posix()} spawns a CLI process")

    bridge_text = (ROOT / BRIDGE_CLIENT).read_text(encoding="utf-8")
    assert '"tui-bridge"' in bridge_text
    assert "JSON.stringify({ id, method, params })" in bridge_text
    assert "JSON.parse(line)" in bridge_text

    assert offenders == []


def test_tui_mutation_methods_are_registered_on_python_bridges(
    tmp_path: Path,
) -> None:
    handlers = _handlers(HieronymusConfig(data_root=tmp_path / "hieronymus"))
    frontend_methods = _frontend_rpc_methods()

    mutation_methods = {
        method
        for method in frontend_methods
        if method.startswith("config.")
        or method
        in {
            "admin.add_crystal",
            "admin.edit_crystal",
            "admin.merge_crystals",
            "admin.split_crystal",
            "admin.reinforce_crystal",
            "admin.decay_crystal",
            "admin.delete_crystal",
            "admin.approve_proposal",
            "admin.reject_proposal",
            "admin.run_manual_dreaming",
            "admin.rename_concept",
            "admin.merge_concepts",
            "admin.archive_concept",
            "admin.remove_short_term_memory",
        }
    }

    assert mutation_methods <= handlers.keys()
    for method in mutation_methods:
        owner = handlers[method].__self__
        if method.startswith("admin."):
            assert isinstance(owner, AdminBridge), method
        else:
            assert isinstance(owner, ConfigBridge), method
