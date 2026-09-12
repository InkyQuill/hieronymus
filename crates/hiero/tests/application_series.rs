//! M1 regression: the application dispatcher performs real series/session
//! domain work (the registry previously reported these tools as not ported).
//!
//! Coverage: argument decoding exactly per the frozen `input_schema`
//! (defaults, nulls, missing and wrong-typed fields), duplicate-slug upserts,
//! unknown sessions, persisted session state, and one end-to-end
//! series/session workflow served over real `POST /mcp` plus the `hiero mcp`
//! stdio adapter.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use common::{mcp_headers, send_request};
use hiero::application::{AppError, Application};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::workspace::WorkspaceStore;
use serde_json::{Value, json};

const ACTOR: &str = "local-user";

fn test_application() -> (tempfile::TempDir, Application) {
    let root = tempfile::tempdir().unwrap();
    let application = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    (root, application)
}

fn expect_invalid(error: AppError, needle: &str) {
    match error {
        AppError::Invalid(message) => assert!(
            message.contains(needle),
            "expected {needle:?} in invalid-argument diagnostic: {message}"
        ),
        other => panic!("expected an invalid-argument error, got: {other}"),
    }
}

fn expect_domain(error: AppError, needle: &str) {
    match error {
        AppError::Domain(message) => assert!(
            message.contains(needle),
            "expected {needle:?} in domain diagnostic: {message}"
        ),
        other => panic!("expected a domain error, got: {other}"),
    }
}

// ------------------------------------------------------------ step 1 regression

#[test]
fn series_create_is_visible_to_list() {
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    app.call(
        "hieronymus_series_create",
        &json!({"slug":"book","title":"Book"}),
        "local-user",
    )
    .unwrap();
    let rows = app
        .call("hieronymus_series_list", &json!({}), "local-user")
        .unwrap();
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .any(|row| row["slug"] == "book")
    );
}

// --------------------------------------------------------- argument decoding

#[test]
fn series_create_applies_the_schema_defaults() {
    let (_root, app) = test_application();
    let payload = app
        .call(
            "hieronymus_series_create",
            &json!({"slug": "book", "title": "Book"}),
            ACTOR,
        )
        .unwrap();

    assert_eq!(payload["slug"], json!("book"));
    assert_eq!(payload["title"], json!("Book"));
    // Defaults: empty languages, tags seeded from them (empty), persisted id.
    assert_eq!(payload["source_language"], json!(""));
    assert_eq!(payload["target_language"], json!(""));
    assert_eq!(payload["language_tags"], json!([]));
    assert!(payload["id"].is_i64());
}

#[test]
fn series_create_follows_the_null_rules_for_language_tags() {
    let (_root, app) = test_application();
    let create = |slug: &str, tags: Value| {
        app.call(
            "hieronymus_series_create",
            &json!({"slug": slug, "title": "T", "source_language": "ja", "target_language": "en",
                    "language_tags": tags}),
            ACTOR,
        )
        .unwrap()
    };

    // Explicit null behaves like the omitted default: tags seed from the
    // default directions.
    let seeded = create("demo-a", json!(null));
    assert_eq!(seeded["language_tags"], json!(["en", "ja"]));
    // Explicit tags are normalized (trim, lowercase, sort, dedup).
    let explicit = create("demo-b", json!([" Ja", " en ", "JA"]));
    assert_eq!(explicit["language_tags"], json!(["en", "ja"]));
    // An explicit empty array clears the tags instead of reseeding.
    let cleared = create("demo-c", json!([]));
    assert_eq!(cleared["language_tags"], json!([]));
}

#[test]
fn missing_or_mistyped_arguments_fail_without_writes() {
    let (_root, app) = test_application();

    let error = app
        .call("hieronymus_series_create", &json!({"title": "Book"}), ACTOR)
        .unwrap_err();
    expect_invalid(error, "missing field `slug`");

    let error = app
        .call(
            "hieronymus_series_create",
            &json!({"slug": "book", "title": 3}),
            ACTOR,
        )
        .unwrap_err();
    expect_invalid(error, "invalid type");

    // Neither failure wrote anything.
    let rows = app
        .call("hieronymus_series_list", &json!({}), ACTOR)
        .unwrap();
    assert!(rows.as_array().unwrap().is_empty());
}

// ---------------------------------------------------------------- series work

#[test]
fn duplicate_slug_upserts_instead_of_duplicating() {
    let (_root, app) = test_application();
    app.call(
        "hieronymus_series_create",
        &json!({"slug": "book", "title": "Book"}),
        ACTOR,
    )
    .unwrap();
    let updated = app
        .call(
            "hieronymus_series_create",
            &json!({"slug": "book", "title": "Book, Volume 1",
                    "source_language": "ja", "target_language": "en"}),
            ACTOR,
        )
        .unwrap();

    assert_eq!(updated["title"], json!("Book, Volume 1"));
    let rows = app
        .call("hieronymus_series_list", &json!({}), ACTOR)
        .unwrap();
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 1, "duplicate slugs must upsert");
    assert_eq!(rows[0]["title"], json!("Book, Volume 1"));
    assert_eq!(rows[0]["source_language"], json!("ja"));
}

#[test]
fn invalid_slugs_are_domain_rejections() {
    let (_root, app) = test_application();
    let error = app
        .call(
            "hieronymus_series_create",
            &json!({"slug": "Book", "title": "Book"}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "invalid series slug");

    let rows = app
        .call("hieronymus_series_list", &json!({}), ACTOR)
        .unwrap();
    assert!(rows.as_array().unwrap().is_empty());
}

#[test]
fn series_init_performs_the_same_registered_setup() {
    // Fresh data root: the compatibility wrapper must still run the real
    // project setup (apply the schema) and persist the series, never alias
    // into a stub.
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();

    let payload = app
        .call(
            "hieronymus_series_init",
            &json!({"slug": "init-demo", "title": "Init Demo",
                    "source_language": "ja", "target_language": "en"}),
            ACTOR,
        )
        .unwrap();

    assert_eq!(payload["slug"], json!("init-demo"));
    assert!(payload["id"].is_i64());
    assert!(
        root.path().join("hieronymus.sqlite").exists(),
        "series_init must initialize the database on a fresh root"
    );
    let rows = app
        .call("hieronymus_series_list", &json!({}), ACTOR)
        .unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
}

#[test]
fn set_language_tags_replaces_tags_without_touching_compat_fields() {
    let (_root, app) = test_application();
    let series = app
        .call(
            "hieronymus_series_create",
            &json!({"slug": "book", "title": "Book",
                    "source_language": "ja", "target_language": "en"}),
            ACTOR,
        )
        .unwrap();
    let series_id = series["id"].as_i64().unwrap();

    let payload = app
        .call(
            "hieronymus_series_set_language_tags",
            &json!({"series_id": series_id, "language_tags": [" fr ", "DE"]}),
            ACTOR,
        )
        .unwrap();

    assert_eq!(payload["language_tags"], json!(["de", "fr"]));
    assert_eq!(payload["source_language"], json!("ja"));
    assert_eq!(payload["target_language"], json!("en"));
    assert_eq!(payload["id"], json!(series_id));

    // Unknown series id: a domain rejection.
    let error = app
        .call(
            "hieronymus_series_set_language_tags",
            &json!({"series_id": 987_654, "language_tags": ["en"]}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "unknown series id");

    // Wrong types (a string, or the null the schema does not allow here):
    // invalid arguments, and the stored tags stay untouched.
    let error = app
        .call(
            "hieronymus_series_set_language_tags",
            &json!({"series_id": series_id, "language_tags": "de"}),
            ACTOR,
        )
        .unwrap_err();
    expect_invalid(error, "invalid type");
    let error = app
        .call(
            "hieronymus_series_set_language_tags",
            &json!({"series_id": series_id, "language_tags": null}),
            ACTOR,
        )
        .unwrap_err();
    expect_invalid(error, "invalid type");
    let rows = app
        .call("hieronymus_series_list", &json!({}), ACTOR)
        .unwrap();
    assert_eq!(rows[0]["language_tags"], json!(["de", "fr"]));
}

// --------------------------------------------------------------- session work

#[test]
fn session_start_persists_an_active_session() {
    let (root, app) = test_application();
    app.call(
        "hieronymus_series_create",
        &json!({"slug": "book", "title": "Book",
                "source_language": "ja", "target_language": "en"}),
        ACTOR,
    )
    .unwrap();

    let started = app
        .call(
            "hieronymus_session_start",
            &json!({"series_slug": "book", "volume": "1", "chapter": "2"}),
            ACTOR,
        )
        .unwrap();
    let session_id = started["session_id"].as_i64().unwrap();

    // Real persisted work, verified through the domain store.
    let store = WorkspaceStore::open(&HieronymusConfig::new(root.path())).unwrap();
    let session = store.get_session(session_id).unwrap();
    assert_eq!(session.status, "active");
    assert_eq!(session.context.series_slug, "book");
    assert_eq!(session.context.source_language, "ja");
    assert_eq!(session.context.target_language, "en");
    assert_eq!(session.context.task_type, "translation");
    assert_eq!(session.context.volume, "1");
    assert_eq!(session.context.chapter, "2");
}

#[test]
fn session_start_rejects_unknown_series_and_empty_languages() {
    let (_root, app) = test_application();

    let error = app
        .call(
            "hieronymus_session_start",
            &json!({"series_slug": "ghost"}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "unknown series");

    app.call(
        "hieronymus_series_create",
        &json!({"slug": "book", "title": "Book",
                "source_language": "ja", "target_language": "en"}),
        ACTOR,
    )
    .unwrap();

    let error = app
        .call(
            "hieronymus_session_start",
            &json!({"series_slug": "book", "source_language": " "}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "must not be empty");

    let error = app
        .call(
            "hieronymus_session_start",
            &json!({"series_slug": "book", "target_language": " "}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "must not be empty");

    // Overrides equal to the defaults are accepted.
    app.call(
        "hieronymus_session_start",
        &json!({"series_slug": "book", "source_language": "ja", "target_language": "en"}),
        ACTOR,
    )
    .unwrap();
}

#[test]
fn session_complete_marks_the_session_completed() {
    let (root, app) = test_application();
    app.call(
        "hieronymus_series_create",
        &json!({"slug": "book", "title": "Book",
                "source_language": "ja", "target_language": "en"}),
        ACTOR,
    )
    .unwrap();
    let started = app
        .call(
            "hieronymus_session_start",
            &json!({"series_slug": "book"}),
            ACTOR,
        )
        .unwrap();
    let session_id = started["session_id"].as_i64().unwrap();

    let completed = app
        .call(
            "hieronymus_session_complete",
            &json!({"session_id": session_id}),
            ACTOR,
        )
        .unwrap();
    assert_eq!(
        completed,
        json!({"session_id": session_id, "completed": true})
    );

    let store = WorkspaceStore::open(&HieronymusConfig::new(root.path())).unwrap();
    assert_eq!(store.get_session(session_id).unwrap().status, "completed");

    // Unknown sessions are domain errors.
    let error = app
        .call(
            "hieronymus_session_complete",
            &json!({"session_id": 424_242}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "unknown session");

    // Completing again reports the same success (the Python wrapper ignores
    // the already-completed case).
    app.call(
        "hieronymus_session_complete",
        &json!({"session_id": session_id}),
        ACTOR,
    )
    .unwrap();
    assert_eq!(
        store.get_session(session_id).unwrap().status,
        "completed",
        "repeated completion must not resurrect the session"
    );
}

#[test]
fn unclaimed_tools_report_not_implemented() {
    let (_root, app) = test_application();
    // Every advertised tool has a concrete handler since M5 (see the
    // tool_completeness regression); a name no family claims is the honest
    // leftover.
    let error = app
        .call("hieronymus_nonexistent", &json!({}), ACTOR)
        .unwrap_err();
    match error {
        AppError::NotImplemented(name) => assert!(name.contains("hieronymus_nonexistent")),
        other => panic!("expected NotImplemented, got: {other}"),
    }
}

#[test]
fn dream_dispatch_serves_only_through_the_daemon_controller() {
    let (_root, app) = test_application();
    // D5: `hieronymus_dream` runs through the daemon's dream controller. A
    // bare application (no daemon) fails closed instead of constructing a
    // provider itself.
    let error = app.call("hieronymus_dream", &json!({}), ACTOR).unwrap_err();
    match error {
        AppError::Domain(message) => {
            assert!(message.contains("dream controller"), "{message}");
        }
        other => panic!("expected Domain, got: {other}"),
    }
    // A named provider cannot select a lane: providers are configured, and
    // the fail-closed pin rejects anything the configured lanes do not run.
    let error = app
        .call("hieronymus_dream", &json!({"provider": "openai"}), ACTOR)
        .unwrap_err();
    assert!(error.to_string().contains("not available"), "{error}");
}

// ------------------------------------------------- real HTTP + stdio workflow

/// A stateless `tools/call` request carrying the exact `_meta` the protocol
/// layer validates.
fn tools_call(id: i64, name: &str, arguments: Value) -> Value {
    json!({
        "id": id,
        "jsonrpc": "2.0",
        "method": "tools/call",
        "params": {
            "_meta": {
                "io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/clientInfo": {
                    "name": "application-series",
                    "version": "1.0.0"
                },
                "io.modelcontextprotocol/protocolVersion": common::PROTOCOL_REVISION
            },
            "arguments": arguments,
            "name": name
        }
    })
}

fn call_tool_over_http(
    daemon: &hiero::daemon::Daemon,
    id: i64,
    name: &str,
    arguments: Value,
) -> common::RawResponse {
    send_request(
        daemon.local_addr().port(),
        "POST",
        "/mcp",
        &mcp_headers(daemon, &[("Mcp-Method", "tools/call"), ("Mcp-Name", name)]),
        &serde_json::to_vec(&tools_call(id, name, arguments)).unwrap(),
    )
}

fn spawn_adapter(root: &Path) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args(["mcp", "--data-root", root.to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn stdio_call(adapter: &mut std::process::Child, id: i64, name: &str, arguments: Value) -> Value {
    let line = serde_json::to_string(&tools_call(id, name, arguments)).unwrap();
    let stdin = adapter.stdin.as_mut().unwrap();
    stdin.write_all(line.as_bytes()).unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
    let mut response_line = String::new();
    BufReader::new(adapter.stdout.as_mut().unwrap())
        .read_line(&mut response_line)
        .unwrap();
    serde_json::from_str(response_line.trim_end())
        .expect("the adapter must answer with a JSON-RPC line")
}

#[test]
fn daemon_serves_one_series_session_workflow_over_http_and_stdio() {
    let root = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon(root.path());

    // 1. Create the series over real POST /mcp.
    let create = call_tool_over_http(
        &daemon,
        1,
        "hieronymus_series_create",
        json!({"slug": "book", "title": "Book",
               "source_language": "ja", "target_language": "en"}),
    );
    assert_eq!(create.status, 200, "{:?}", create.body());
    let result = &create.body()["result"];
    assert_eq!(result["isError"], json!(false));
    assert_eq!(result["resultType"], json!("complete"));
    assert_eq!(result["structuredContent"]["slug"], json!("book"));
    let rendered: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(rendered, result["structuredContent"]);

    // 2. List it over the stdio adapter (proxied to the same daemon).
    let mut adapter = spawn_adapter(root.path());
    let listed = stdio_call(&mut adapter, 2, "hieronymus_series_list", json!({}));
    let rows = listed["result"]["structuredContent"].as_array().unwrap();
    assert!(rows.iter().any(|row| row["slug"] == "book"), "{rows:?}");

    // 3. Start a session over HTTP.
    let started = call_tool_over_http(
        &daemon,
        3,
        "hieronymus_session_start",
        json!({"series_slug": "book", "volume": "1", "chapter": "2"}),
    );
    assert_eq!(started.status, 200, "{:?}", started.body());
    let session_id = started.body()["result"]["structuredContent"]["session_id"]
        .as_i64()
        .expect("session_start must return a session_id");

    // 4. Complete the session over stdio.
    let completed = stdio_call(
        &mut adapter,
        4,
        "hieronymus_session_complete",
        json!({"session_id": session_id}),
    );
    assert_eq!(
        completed["result"]["structuredContent"],
        json!({"session_id": session_id, "completed": true})
    );
    adapter.kill().unwrap();
    let _ = adapter.wait();

    // 5. The workflow really persisted.
    let store = WorkspaceStore::open(&HieronymusConfig::new(root.path())).unwrap();
    let session = store.get_session(session_id).unwrap();
    assert_eq!(session.status, "completed");
    assert_eq!(session.context.series_slug, "book");
    assert_eq!(session.context.volume, "1");

    daemon.shutdown().unwrap();
}

#[test]
fn daemon_maps_argument_and_domain_failures_per_protocol() {
    let root = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon(root.path());

    // Malformed arguments -> JSON-RPC invalid-params.
    let response = call_tool_over_http(
        &daemon,
        10,
        "hieronymus_series_create",
        json!({"title": "Book"}),
    );
    assert_eq!(response.status, 400, "{:?}", response.body());
    assert_eq!(response.body()["error"]["code"], json!(-32602));
    assert!(
        response.body()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("slug")
    );

    // Domain failures -> tool error results (a 200 envelope with isError).
    let response = call_tool_over_http(
        &daemon,
        11,
        "hieronymus_session_start",
        json!({"series_slug": "ghost"}),
    );
    assert_eq!(response.status, 200, "{:?}", response.body());
    let result = &response.body()["result"];
    assert_eq!(result["isError"], json!(true));
    assert_eq!(result["resultType"], json!("complete"));
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unknown series")
    );
    assert!(result.get("structuredContent").is_none());

    daemon.shutdown().unwrap();
}
