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
fn schema_two_discovery_is_supported_without_promising_direction_context() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "project.md", &MANIFEST.replace("schema-version: 1", "schema-version: 2\nproject-kind: translation\nwork-kind: series\ntranslation-enabled: true"));
    let project = discover(dir.path()).unwrap().unwrap();
    assert_eq!(project.schema_version, 2);
    assert!(project.translation_enabled);
    for direction in [None, Some("ru-main")] {
        assert!(matches!(
            select_binding(&project, direction),
            Err(CwsError::UnsupportedDirectionSelection)
        ));
    }
}
