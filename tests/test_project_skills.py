from pathlib import Path

import pytest

from hieronymus.project_skills import (
    install_project_skills,
    skill_assets,
    uninstall_project_skills,
)


def test_install_writes_every_skill_to_both_targets(tmp_path: Path) -> None:
    install_project_skills(tmp_path, ("agents", "claude"))

    for target in (".agents/skills", ".claude/skills"):
        for relative_path, text in skill_assets().items():
            installed_path = tmp_path / target / relative_path
            assert installed_path.read_text(encoding="utf-8") == text


def test_overwrite_owned_skill_preserves_custom_skill(tmp_path: Path) -> None:
    owned = tmp_path / ".agents/skills/hieronymus-read/SKILL.md"
    custom = tmp_path / ".agents/skills/custom/SKILL.md"
    owned.parent.mkdir(parents=True)
    custom.parent.mkdir(parents=True)
    owned.write_text("old", encoding="utf-8")
    custom.write_text("custom", encoding="utf-8")

    install_project_skills(tmp_path, ("agents",))

    assert owned.read_text(encoding="utf-8") != "old"
    assert custom.read_text(encoding="utf-8") == "custom"


def test_uninstall_removes_only_owned_skill_directories(tmp_path: Path) -> None:
    install_project_skills(tmp_path, ("agents",))
    custom = tmp_path / ".agents/skills/custom/SKILL.md"
    custom.parent.mkdir(parents=True)
    custom.write_text("custom", encoding="utf-8")

    uninstall_project_skills(tmp_path, ("agents",))

    assert not (tmp_path / ".agents/skills/hieronymus-read").exists()
    assert custom.read_text(encoding="utf-8") == "custom"


def test_dry_run_does_not_modify_workspace(tmp_path: Path) -> None:
    plan = install_project_skills(tmp_path, ("agents",), dry_run=True)

    assert plan.dry_run is True
    assert not (tmp_path / ".agents").exists()


def test_duplicate_targets_are_installed_once_in_canonical_order(tmp_path: Path) -> None:
    plan = install_project_skills(tmp_path, ("claude", "agents", "claude"))

    assert plan.targets == ("agents", "claude")
    assert (tmp_path / ".agents/skills/hieronymus-read/SKILL.md").is_file()
    assert (tmp_path / ".claude/skills/hieronymus-read/SKILL.md").is_file()


def test_rejects_unknown_target(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="unsupported project skill target"):
        install_project_skills(tmp_path, ("codex",))


def test_rejects_symlinked_workspace(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    linked_workspace = tmp_path / "linked-workspace"
    linked_workspace.symlink_to(workspace, target_is_directory=True)

    with pytest.raises(ValueError, match="workspace must not be a symlink"):
        install_project_skills(linked_workspace, ("agents",))


def test_rejects_symlinked_target_root(tmp_path: Path) -> None:
    target = tmp_path / ".agents/skills"
    target.parent.mkdir()
    target.symlink_to(tmp_path / "outside", target_is_directory=True)

    with pytest.raises(ValueError, match="target root must not be a symlink"):
        install_project_skills(tmp_path, ("agents",))


@pytest.mark.parametrize("symlinked_destination", ["directory", "skill_file"])
def test_install_rejects_symlinked_owned_destinations_before_writing(
    tmp_path: Path, symlinked_destination: str
) -> None:
    skills_root = tmp_path / ".agents/skills"
    already_owned = skills_root / "hieronymus-recall/SKILL.md"
    already_owned.parent.mkdir(parents=True)
    already_owned.write_text("old bundled content", encoding="utf-8")
    outside = tmp_path / "outside"
    outside.mkdir()
    outside_skill = outside / "SKILL.md"
    outside_skill.write_text("outside content", encoding="utf-8")

    malicious_directory = skills_root / "hieronymus-read"
    if symlinked_destination == "directory":
        malicious_directory.symlink_to(outside, target_is_directory=True)
    else:
        malicious_directory.mkdir()
        (malicious_directory / "SKILL.md").symlink_to(outside_skill)

    with pytest.raises(ValueError, match="must not contain a symlink"):
        install_project_skills(tmp_path, ("agents",))

    assert already_owned.read_text(encoding="utf-8") == "old bundled content"
    assert outside_skill.read_text(encoding="utf-8") == "outside content"
