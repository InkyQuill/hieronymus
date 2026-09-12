use std::{fs, path::Path};

use hieronymus::{
    cws_binding::select_binding,
    cws_project::{CwsError, DocumentRole, classify, discover},
};
use serde_json::{Value, json};

const MANIFEST: &str = "---\nschema-version: 1\ntitle: Test\nlanguage: ru\nstatus: drafting\n---\n";

fn write(root: &Path, name: &str, text: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn binding(root: &Path, value: Value) {
    write(root, ".hieronymus.json", &value.to_string());
}

fn valid_binding() -> Value {
    json!({"series_slug":"work", "cws":{
        "binding_version":1,"project_contract_version":1,"directions":{}
    }})
}

#[test]
fn pinned_public_fixtures_are_executable_without_cws() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../compatibility/rust/cws-project-v1.json"
    ))
    .unwrap();
    assert_eq!(fixture["contract_version"], 1);
    for case in fixture["cases"].as_array().unwrap() {
        let dir = tempfile::tempdir().unwrap();
        for (path, contents) in case["files"].as_object().unwrap() {
            write(dir.path(), path, contents.as_str().unwrap());
        }
        let expected = &case["expect"];
        if expected["technical_failure"] == "unsupported_schema" {
            assert!(matches!(
                discover(dir.path()),
                Err(CwsError::UnsupportedSchema(99))
            ));
            continue;
        }
        let root = dir.path().join(expected["root"].as_str().unwrap());
        let project = discover(&root).unwrap().unwrap();
        assert_eq!(project.root, root.canonicalize().unwrap());
        assert_eq!(
            project.schema_version,
            expected["schema_version"].as_u64().unwrap()
        );
        for (path, role) in expected["roles"].as_object().unwrap() {
            let actual = classify(&project, Path::new(path)).unwrap();
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                *role,
                "{}: {path}",
                case["name"]
            );
        }
    }
}

#[test]
fn discovery_uses_nearest_manifest_and_never_parses_the_body() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "project.md", MANIFEST);
    write(
        dir.path(),
        "inner/project.md",
        &MANIFEST.replace("language: ru", "language: en"),
    );
    write(
        dir.path(),
        "inner/story/chapters/01.md",
        "[body: malformed yaml",
    );
    let project = discover(&dir.path().join("inner/story/chapters/01.md"))
        .unwrap()
        .unwrap();
    assert_eq!(project.language, "en");
    assert_eq!(project.project_kind, "authoring");
    assert_eq!(project.work_kind, "book");
    assert!(!project.translation_enabled);
    write(
        dir.path(),
        "inner/project.md",
        "---\nschema-version: [1]\n---\n",
    );
    assert!(matches!(
        discover(&dir.path().join("inner")),
        Err(CwsError::InvalidManifest)
    ));
}

#[test]
fn nested_project_does_not_inherit_outer_binding() {
    let dir = tempfile::tempdir().unwrap();
    let inner = dir.path().join("inner");
    fs::create_dir(&inner).unwrap();
    fs::write(dir.path().join("project.md"), MANIFEST).unwrap();
    fs::write(inner.join("project.md"), MANIFEST).unwrap();
    fs::write(dir.path().join(".hieronymus.json"),
        r#"{"series_slug":"outer","cws":{"binding_version":1,"project_contract_version":1,"directions":{}}}"#).unwrap();
    let project = discover(&inner).unwrap().unwrap();
    assert!(select_binding(&project, None).unwrap().is_none());
}

#[test]
fn cws_scalar_and_list_subset_accepts_producer_spellings() {
    // Pinned to cwcli.documents.parse_document, not YAML scalar coercion.
    for extra in [
        "coverage:\nauxiliary-editions: \"\"\n",
        "evidence:\n  - true\n  - -3\n  - user: keep calm\n  - \"日本語\"\n",
        "note: null\nfloat: 1.2\nleading-zero: 01\nplus: +1\n",
        "note: 'an ''author'' note'\ncomment: keep # literally\n# comment\n",
        "note: \"line\\n\\u65e5\"\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let text = MANIFEST.replace("status: drafting\n", &format!("status: drafting\n{extra}"));
        for newline in ["\n", "\r\n", "\r"] {
            let text = format!(
                "\u{feff}{}[not: YAML in the body",
                text.replace('\n', newline)
            );
            write(dir.path(), "project.md", &text);
            assert!(discover(dir.path()).unwrap().is_some(), "{extra:?}");
        }
    }
}

#[test]
fn required_manifest_strings_reject_cws_control_whitespace() {
    let dir = tempfile::tempdir().unwrap();
    for field in ["title: Test", "language: ru"] {
        let key = field.split_once(':').unwrap().0;
        for code in 0x1c..=0x1f {
            let replacement = format!("{key}: \"\\u00{code:02x}\"");
            write(
                dir.path(),
                "project.md",
                &MANIFEST.replace(field, &replacement),
            );
            assert!(
                matches!(discover(dir.path()), Err(CwsError::InvalidManifest)),
                "{replacement} must be rejected as empty by CWS whitespace rules"
            );
        }
    }
}

#[test]
fn unsupported_frontmatter_and_wrong_manifest_types_are_rejected() {
    let invalid = [
        "title: Duplicate\n",
        "nested:\n  child: x\n",
        "flow: [x]\n",
        "map: {x: y}\n",
        "anchor: &x foo\n",
        "alias: *x\n",
        "tag: !str x\n",
        "block: |\n",
        "block: >-2\n",
        "note: \"bad\\q\"\n",
        "note: 'unterminated\n",
        "list:\n    - x\n",
        "list:\n- x\n",
        "list: \"\"\n  - x\n",
        " \n",
        "  # indented comment\n",
        "list:\n  - [x]\n",
        "orphan\n",
    ];
    let dir = tempfile::tempdir().unwrap();
    for extra in invalid {
        write(
            dir.path(),
            "project.md",
            &MANIFEST.replace("status: drafting\n", &format!("status: drafting\n{extra}")),
        );
        assert!(
            matches!(discover(dir.path()), Err(CwsError::InvalidManifest)),
            "{extra:?}"
        );
    }
    for (from, to) in [
        ("schema-version: 1", "schema-version: \"1\""),
        ("schema-version: 1", "schema-version: true"),
        ("title: Test", "title:"),
        ("language: ru", "language: false"),
        ("status: drafting", "status: unknown"),
        ("title: Test", "title: 123"),
    ] {
        write(dir.path(), "project.md", &MANIFEST.replace(from, to));
        assert!(
            matches!(discover(dir.path()), Err(CwsError::InvalidManifest)),
            "{to}"
        );
    }
    for text in [
        "---\nschema-version: 1",
        "--- \nschema-version: 1\n---\n",
        "no frontmatter",
    ] {
        write(dir.path(), "project.md", text);
        assert!(matches!(
            discover(dir.path()),
            Err(CwsError::InvalidManifest)
        ));
    }
    fs::write(dir.path().join("project.md"), [0xff, 0xfe]).unwrap();
    assert!(matches!(
        discover(dir.path()),
        Err(CwsError::InvalidManifest)
    ));
}

#[test]
fn structural_classification_stays_exact_and_respects_layout() {
    let dir = tempfile::tempdir().unwrap();
    let v2 = MANIFEST.replace(
        "schema-version: 1",
        "schema-version: 2\nproject-kind: authoring\nwork-kind: book\ntranslation-enabled: true",
    );
    write(dir.path(), "project.md", &v2);
    let project = discover(dir.path()).unwrap().unwrap();
    for path in [
        "notes/_index.md",
        "kb/mystery/_index.md",
        "work/notes/a.md",
        "story/chapters/nested/a.md",
        "kb/world/nested/a.md",
        "story/chapters/a.txt",
        "translations/originals/translation.md",
        "sources/ja/originals/a.md",
        "translations/ru/memory/mystery/a.md",
        "translations/ru/memory/terms/deep/a.md",
        "inspiration/old.md",
        "sources/ja/volumes/v1/text/a.md",
        "translations/ru/volumes/v1/drafts/a.md",
    ] {
        write(dir.path(), path, "opaque");
        assert_eq!(
            classify(&project, Path::new(path)).unwrap(),
            DocumentRole::Opaque,
            "{path}"
        );
    }
    for (path, expected) in [
        ("work/archive/a.md", DocumentRole::Work),
        ("kb/continuity/questions.md", DocumentRole::Knowledge),
        ("kb/continuity/_index.md", DocumentRole::Derived),
        (
            "translations/ru/memory/voices/a.md",
            DocumentRole::TranslationMemory,
        ),
        (
            "translations/ru/memory/decisions/a.md",
            DocumentRole::TranslationMemory,
        ),
        (".creative-writing/state.json", DocumentRole::PrivateState),
    ] {
        write(dir.path(), path, "text");
        assert_eq!(
            classify(&project, Path::new(path)).unwrap(),
            expected,
            "{path}"
        );
    }
}

#[test]
fn paths_cannot_escape_or_enter_nested_projects() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "project.md", MANIFEST);
    write(dir.path(), "story/chapters/nested/project.md", MANIFEST);
    write(dir.path(), "story/chapters/nested/a.md", "text");
    let project = discover(dir.path()).unwrap().unwrap();
    for path in [
        "../project.md",
        "/tmp/project.md",
        "story/../project.md",
        "C:/project.md",
        "story\\chapters\\a.md",
        "story/chapters/nested/a.md",
    ] {
        assert!(
            matches!(
                classify(&project, Path::new(path)),
                Err(CwsError::UnsafePath)
            ),
            "{path}"
        );
    }
    assert!(matches!(
        discover(&dir.path().join("story/../")),
        Err(CwsError::UnsafePath)
    ));
}

#[cfg(unix)]
#[test]
fn actual_symlinks_are_not_followed() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(dir.path(), "project.md", MANIFEST);
    write(outside.path(), "chapter.md", "<hidden>Secret</hidden>");
    symlink(outside.path(), dir.path().join("story")).unwrap();
    let project = discover(dir.path()).unwrap().unwrap();
    assert!(matches!(
        classify(&project, Path::new("story/chapter.md")),
        Err(CwsError::UnsafePath)
    ));
    assert!(matches!(
        discover(&dir.path().join("story/chapter.md")),
        Err(CwsError::UnsafePath)
    ));
    symlink(
        outside.path().join("chapter.md"),
        dir.path().join(".hieronymus.json"),
    )
    .unwrap();
    assert!(matches!(
        select_binding(&project, None),
        Err(CwsError::UnsafePath)
    ));
    fs::remove_file(dir.path().join("project.md")).unwrap();
    symlink(
        outside.path().join("chapter.md"),
        dir.path().join("project.md"),
    )
    .unwrap();
    assert!(discover(dir.path()).unwrap().is_none());
}

#[test]
fn bindings_are_explicit_validated_and_independent_of_legacy_defaults() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "project.md", MANIFEST);
    let project = discover(dir.path()).unwrap().unwrap();
    assert!(select_binding(&project, None).unwrap().is_none());
    binding(dir.path(), json!({"series_slug":"legacy"}));
    assert!(select_binding(&project, None).unwrap().is_none());
    binding(dir.path(), valid_binding());
    assert_eq!(
        select_binding(&project, None).unwrap().unwrap().series_slug,
        "work"
    );
    let mut no_common = valid_binding();
    no_common.as_object_mut().unwrap().remove("series_slug");
    binding(dir.path(), no_common);
    assert!(select_binding(&project, None).unwrap().is_none());
    for value in [json!({"cws":null}), json!({"cws":{}}), json!([])] {
        binding(dir.path(), value);
        assert!(matches!(
            select_binding(&project, None),
            Err(CwsError::InvalidBinding)
        ));
    }
    for (key, value) in [("series_slug", json!(" ")), ("series_slug", json!(42))] {
        let mut raw = valid_binding();
        raw[key] = value;
        binding(dir.path(), raw);
        assert!(matches!(
            select_binding(&project, None),
            Err(CwsError::InvalidBinding)
        ));
    }
    for (key, value) in [
        ("trust", json!("primary")),
        ("directions", json!([])),
        ("directions", json!({"ru":{"series_slug":""}})),
        ("directions", json!({"../ru":{"series_slug":"work"}})),
        (
            "directions",
            json!({"ru":{"series_slug":"work", "authority":true}}),
        ),
    ] {
        let mut raw = valid_binding();
        raw["cws"][key] = value;
        binding(dir.path(), raw);
        assert!(
            matches!(
                select_binding(&project, None),
                Err(CwsError::InvalidBinding)
            ),
            "{key}"
        );
    }
    let mut raw = valid_binding();
    raw["cws"]["binding_version"] = json!(2);
    binding(dir.path(), raw);
    assert!(matches!(
        select_binding(&project, None),
        Err(CwsError::UnsupportedBinding(2))
    ));
    let mut raw = valid_binding();
    raw["cws"]["project_contract_version"] = json!(2);
    binding(dir.path(), raw);
    assert!(matches!(
        select_binding(&project, None),
        Err(CwsError::UnsupportedContractVersion(2))
    ));
    // A future version may have a different shape; do not validate it as v1.
    binding(
        dir.path(),
        json!({"cws":{"binding_version":2,"new_field":true}}),
    );
    assert!(matches!(
        select_binding(&project, None),
        Err(CwsError::UnsupportedBinding(2))
    ));
    binding(
        dir.path(),
        json!({"cws":{"binding_version":1,"project_contract_version":2,"new_field":true}}),
    );
    assert!(matches!(
        select_binding(&project, None),
        Err(CwsError::UnsupportedContractVersion(2))
    ));
}

#[test]
fn schema_two_discovery_and_binding_are_independent_of_direction_selection() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "project.md", &MANIFEST.replace("schema-version: 1", "schema-version: 2\nproject-kind: translation\nwork-kind: series\ntranslation-enabled: true"));
    let project = discover(dir.path()).unwrap().unwrap();
    assert_eq!(project.schema_version, 2);
    assert!(project.translation_enabled);
    for direction in [None, Some("ru-main")] {
        assert_eq!(select_binding(&project, direction).unwrap(), None);
    }
}

#[test]
fn actionable_selection_retains_editions_sources_and_producer_hashes() {
    use hieronymus::cws_project::select_direction;
    use sha2::{Digest, Sha256};
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../../compatibility/rust/cws-project-v1.json"
    ))
    .unwrap();
    for case in fixtures["cases"].as_array().unwrap() {
        let Some(selections) = case["expect"]["selections"].as_array() else {
            continue;
        };
        let root = tempfile::tempdir().unwrap();
        for (path, contents) in case["files"].as_object().unwrap() {
            write(root.path(), path, contents.as_str().unwrap());
        }
        let project = discover(root.path()).unwrap().unwrap();
        for expected in selections
            .iter()
            .filter(|s| s["status"] == "ready" || s["status"] == "unbound")
        {
            let selected = select_direction(
                &project,
                &root.path().join(expected["cwd"].as_str().unwrap()),
                expected["direction_id"].as_str(),
            )
            .unwrap()
            .unwrap();
            assert_eq!(selected.work_kind, "series");
            assert_eq!(
                serde_json::to_value(&selected.volume_id).unwrap(),
                expected["volume_id"]
            );
            assert_eq!(
                selected.primary_edition.edition_id,
                expected["primary_edition"]
            );
            assert_eq!(selected.primary_edition.revision_label, "first");
            assert_eq!(selected.source_language, expected["source_language"]);
            assert_eq!(selected.target_language, expected["target_language"]);
            assert_eq!(
                serde_json::to_value(
                    selected
                        .auxiliary_editions
                        .iter()
                        .map(|e| &e.edition_id)
                        .collect::<Vec<_>>()
                )
                .unwrap(),
                expected["auxiliary_editions"]
            );
            let refs: Vec<_> = selected
                .source_units
                .iter()
                .map(|u| u.reference.clone())
                .collect();
            assert_eq!(
                serde_json::to_value(refs).unwrap(),
                expected.get("source_units").cloned().unwrap_or(json!([]))
            );
            for unit in selected.source_units {
                assert_eq!(
                    unit.sha256,
                    format!("{:x}", Sha256::digest(fs::read(&unit.path).unwrap()))
                );
                assert!(unit.original_sha256.is_some());
                assert!(
                    unit.original_path
                        .unwrap()
                        .starts_with(project.root.join("sources"))
                );
            }
        }
    }
}

fn translation_fixture(name: &str) -> tempfile::TempDir {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../../compatibility/rust/cws-project-v1.json"
    ))
    .unwrap();
    let case = fixtures["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    for (path, contents) in case["files"].as_object().unwrap() {
        write(root.path(), path, contents.as_str().unwrap());
    }
    root
}

#[test]
fn book_selection_follows_declared_source_units_and_never_filename_alignment() {
    use hieronymus::cws_project::select_direction;
    let root = translation_fixture("translation-book");
    let project = discover(root.path()).unwrap().unwrap();
    let selected = select_direction(
        &project,
        Path::new("translations/ru-main/drafts/u001.md"),
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(selected.work_kind, "book");
    assert_eq!(selected.volume_id, None);
    assert_eq!(selected.source_language, "ja");
    assert_eq!(selected.target_language, "ru");
    assert_eq!(selected.source_units[0].reference, "ja:u001");
    assert!(matches!(
        select_direction(&project, Path::new("sources/ja/text/u001.md"), None),
        Err(CwsError::AmbiguousDirection)
    ));
    let dotted = select_direction(
        &project,
        Path::new("./sources/ja/text/u001.md"),
        Some("ru-main"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(dotted.source_units[0].reference, "ja:u001");
    let unknown = select_direction(
        &project,
        Path::new("translations/ru-main/drafts/another.md"),
        None,
    )
    .unwrap()
    .unwrap();
    assert!(unknown.source_units.is_empty());
    let path = root.path().join("translations/ru-main/drafts/u001.md");
    let contents = fs::read_to_string(&path)
        .unwrap()
        .replace("ja:u001", "u001");
    fs::write(&path, contents).unwrap();
    assert!(matches!(
        select_direction(&project, &path, None),
        Err(CwsError::InvalidManifest)
    ));
}

#[test]
fn series_documents_resolve_volume_without_document_volume_metadata() {
    use hieronymus::cws_project::select_direction;
    let root = translation_fixture("translation-series");
    // Recorded from the canonical CWS draft -> reviewed -> accepted producer.
    let accepted = "---\ndirection-id: ru-main\ndraft-id: u002\nsource-units:\n  - ja:u002\npacket-transaction: 210549e157b84af0b55a9db1b2eca549\nbase-revision: absent\nstatus: accepted\nreview-hash: 23216bae071f0aa39799ab476268d46f780eefb1f4454df208899cbc44fdc197\n---\nПеревод второго тома.\n\n";
    write(
        root.path(),
        "translations/ru-main/volumes/v002/accepted/u002.md",
        accepted,
    );
    let project = discover(root.path()).unwrap().unwrap();
    for kind in ["drafts", "accepted"] {
        let relative = format!("translations/ru-main/volumes/v002/{kind}/u002.md");
        let path = root.path().join(&relative);
        let original = fs::read_to_string(&path).unwrap();
        for volume_field in ["", "volume-id: v002\n"] {
            fs::write(
                &path,
                original.replacen("direction-id:", &format!("{volume_field}direction-id:"), 1),
            )
            .unwrap();
            let selected = select_direction(&project, Path::new(&relative), None)
                .unwrap_or_else(|error| panic!("{relative}: {error:?}"))
                .unwrap();
            assert_eq!(selected.direction_id, "ru-main");
            assert_eq!(selected.work_kind, "series");
            assert_eq!(selected.volume_id.as_deref(), Some("v002"));
            assert_eq!(selected.source_language, "ja");
            assert_eq!(selected.target_language, "ru");
            assert_eq!(selected.primary_edition.edition_id, "ja");
            assert!(selected.auxiliary_editions.is_empty());
            assert_eq!(selected.source_units.len(), 1);
            assert_eq!(selected.source_units[0].reference, "ja:u002");
            assert_eq!(selected.source_units[0].volume_id.as_deref(), Some("v002"));
            assert_eq!(
                selected.source_units[0].path,
                project.root.join("sources/ja/volumes/v002/text/u002.md")
            );
        }
        for invalid in [
            original.replacen("direction-id:", "volume-id: v001\ndirection-id:", 1),
            original.replacen("direction-id:", "volume-id: \ndirection-id:", 1),
            original.replace("ja:u002", "ja:u001"),
        ] {
            fs::write(&path, invalid).unwrap();
            assert!(
                matches!(
                    select_direction(&project, Path::new(&relative), None),
                    Err(CwsError::InvalidManifest)
                ),
                "{relative} accepted a conflicting volume"
            );
        }
    }
}

#[test]
fn selection_rejects_unsafe_overrides_mismatched_identity_and_layout() {
    use hieronymus::cws_project::select_direction;
    let root = translation_fixture("direction-context-series");
    let project = discover(root.path()).unwrap().unwrap();
    for content in [
        "---\ndirection-id: another\nvolume-id: v002\n---\n",
        "---\ndirection-id: ru-main\nvolume-id: v002\nprimary-edition: ../ja\n---\n",
        "---\ndirection-id: ru-main\nvolume-id: v002\nlanguage: fr\n---\n",
    ] {
        write(
            root.path(),
            "translations/ru-main/volumes/v002/settings.md",
            content,
        );
        assert!(matches!(
            select_direction(
                &project,
                Path::new("translations/ru-main/volumes/v002"),
                None
            ),
            Err(CwsError::InvalidManifest)
        ));
    }
    let book = translation_fixture("translation-book");
    let project = discover(book.path()).unwrap().unwrap();
    assert!(matches!(
        select_direction(
            &project,
            Path::new("translations/ru-main/volumes/v001"),
            None
        ),
        Err(CwsError::InvalidManifest)
    ));
    let source = book.path().join("sources/ja/text/u001.md");
    write(book.path(), "translations/ru-main/project.md", MANIFEST);
    assert!(matches!(
        select_direction(&project, &source, Some("ru-main")),
        Err(CwsError::UnsafePath)
    ));
}

#[test]
fn direction_binding_never_falls_back_to_common_work_or_legacy_languages() {
    let root = translation_fixture("direction-explicit-unbound");
    let project = discover(root.path()).unwrap().unwrap();
    assert_eq!(
        select_binding(&project, None).unwrap().unwrap().series_slug,
        "work"
    );
    assert_eq!(select_binding(&project, Some("ru-main")).unwrap(), None);
    assert_eq!(
        select_binding(&project, Some("en-main"))
            .unwrap()
            .unwrap()
            .series_slug,
        "work"
    );
}

#[test]
fn source_path_filters_direction_coverage_before_resolving_volume_context() {
    use hieronymus::cws_project::select_direction;
    let root = translation_fixture("direction-context-series");
    let project = discover(root.path()).unwrap().unwrap();
    assert!(matches!(
        select_direction(&project, Path::new("sources/ja/edition.md"), None),
        Err(CwsError::AmbiguousDirection)
    ));
    let direction = root.path().join("translations/en-main/translation.md");
    let source = fs::read_to_string(&direction)
        .unwrap()
        .replace("  - v002\n", "");
    fs::write(direction, source).unwrap();
    // The English direction covers only volume one; it cannot make the selected
    // volume-two source invalid, and the matching literary direction is unique.
    let selected = select_direction(
        &project,
        Path::new("sources/ja/volumes/v002/text/u002.md"),
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(selected.direction_id, "ru-literary");
}

#[cfg(unix)]
#[test]
fn linked_ancestors_allow_discovery_and_selection_but_project_links_are_rejected() {
    use hieronymus::cws_project::select_direction;
    use std::os::unix::fs::symlink;
    let root = translation_fixture("translation-book");
    let aliases = tempfile::tempdir().unwrap();
    symlink(root.path().parent().unwrap(), aliases.path().join("tmp")).unwrap();
    let alias = aliases
        .path()
        .join("tmp")
        .join(root.path().file_name().unwrap());
    let selected = alias.join("translations/ru-main/drafts/u001.md");
    let project = discover(&selected).unwrap().unwrap();
    assert_eq!(project.root, root.path().canonicalize().unwrap());
    assert_eq!(
        select_direction(&project, &selected, None)
            .unwrap()
            .unwrap()
            .direction_id,
        "ru-main"
    );
    symlink(root.path().join("translations"), root.path().join("linked")).unwrap();
    assert!(matches!(
        discover(&alias.join("linked/ru-main/drafts/u001.md")),
        Err(CwsError::UnsafePath)
    ));
    assert!(matches!(
        select_direction(&project, &alias.join("linked/ru-main/drafts/u001.md"), None),
        Err(CwsError::UnsafePath)
    ));
}

#[test]
fn uncovered_editions_do_not_block_other_source_direction_candidates() {
    use hieronymus::cws_project::select_direction;
    let root = translation_fixture("direction-context-series");
    let project = discover(root.path()).unwrap().unwrap();
    let path = root.path().join("translations/en-main/translation.md");
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("primary-edition: ja", "primary-edition: en");
    fs::write(&path, &text).unwrap();
    let selected = select_direction(
        &project,
        Path::new("sources/ja/volumes/v002/text/u002.md"),
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(selected.direction_id, "ru-literary");
    fs::write(
        &path,
        text.replace("primary-edition: en", "primary-edition: ../bad"),
    )
    .unwrap();
    assert!(matches!(
        select_direction(
            &project,
            Path::new("sources/ja/volumes/v002/text/u002.md"),
            None
        ),
        Err(CwsError::InvalidManifest)
    ));
}

#[cfg(unix)]
#[test]
fn internal_link_to_project_root_cannot_disappear_during_path_normalization() {
    use hieronymus::cws_project::select_direction;
    let root = translation_fixture("translation-book");
    let project = discover(root.path()).unwrap().unwrap();
    std::os::unix::fs::symlink(root.path(), root.path().join("loop")).unwrap();
    let selected = root.path().join("loop/translations/ru-main/drafts/u001.md");
    assert!(matches!(
        select_direction(&project, &selected, None),
        Err(CwsError::UnsafePath)
    ));
}

#[test]
fn uncovered_primary_does_not_hide_invalid_auxiliary_metadata() {
    use hieronymus::cws_project::select_direction;
    let root = translation_fixture("direction-context-series");
    let project = discover(root.path()).unwrap().unwrap();
    let path = root.path().join("translations/en-main/translation.md");
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("primary-edition: ja", "primary-edition: en")
        .replace("auxiliary-editions:\n", "auxiliary-editions:\n  - ../bad\n");
    fs::write(path, text).unwrap();
    assert!(matches!(
        select_direction(
            &project,
            Path::new("sources/ja/volumes/v002/text/u002.md"),
            None
        ),
        Err(CwsError::InvalidManifest)
    ));
}
