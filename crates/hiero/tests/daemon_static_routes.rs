//! Embedded console static route tests: the frozen `frontend.route.*`
//! targets where they are non-empty, and the spec's index-fallback rule for
//! the client-side routes whose fixture targets are empty (ADR 0014). Static
//! routes are unauthenticated and Host-validated only.

mod common;

use common::{
    FIXTURE_ASSET_JS, FIXTURE_INDEX_HTML, route_target, send_request,
    start_daemon_on_ephemeral_port, start_daemon_with_fixture_assets, substitute_placeholders,
};
use serde_json::json;

const HTML_CONTENT_TYPE: &str = "text/html; charset=utf-8";
const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";

#[test]
fn root_matches_frozen_target() {
    let (_root, daemon) = start_daemon_with_fixture_assets();
    let port = daemon.local_addr().port();
    let target = substitute_placeholders(&route_target("frontend.route.get.root"), port);

    // Frozen failure: a foreign Host is rejected before any asset work.
    let failure = &target["failures"][0];
    let host = failure["request"]["headers"]["Host"].as_str().unwrap();
    let response = send_request(
        port,
        "GET",
        "/",
        &[("Host".to_string(), host.to_string())],
        b"",
    );
    assert_eq!(response.status, 400);
    assert_eq!(response.content_type, JSON_CONTENT_TYPE);
    assert_eq!(response.body(), json!({"error": "invalid_host"}));

    // Frozen success: the embedded index with the exact body and type.
    let expected_body = target["success"]["response"]["body"].as_str().unwrap();
    let response = send_request(port, "GET", "/", &[], b"");
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, HTML_CONTENT_TYPE);
    assert_eq!(response.raw_body, expected_body.as_bytes());
}

#[test]
fn assets_route_matches_frozen_target() {
    let (_root, daemon) = start_daemon_with_fixture_assets();
    let port = daemon.local_addr().port();

    let response = send_request(port, "GET", "/assets/fixture", &[], b"");
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "application/javascript");
    assert_eq!(response.raw_body, FIXTURE_ASSET_JS.as_bytes());

    // Missing actual assets are 404, not an index fallback.
    let missing = send_request(port, "GET", "/assets/does-not-exist.js", &[], b"");
    assert_eq!(missing.status, 404);
    assert_eq!(missing.content_type, JSON_CONTENT_TYPE);
    assert_eq!(missing.body(), json!({"error": "not_found"}));
}

#[test]
fn client_side_routes_fall_back_to_the_index() {
    let (_root, daemon) = start_daemon_with_fixture_assets();
    let port = daemon.local_addr().port();
    // The frozen `frontend.route.get.admin(.path)`/`config(.path)` targets are
    // empty; the spec's fallback rule governs: unknown client-side routes
    // serve the embedded index.html.
    for path in [
        "/admin",
        "/admin/fixture",
        "/admin/deep/nested",
        "/config",
        "/config/general",
    ] {
        let response = send_request(port, "GET", path, &[], b"");
        assert_eq!(response.status, 200, "{path} status");
        assert_eq!(
            response.content_type, HTML_CONTENT_TYPE,
            "{path} content type"
        );
        assert_eq!(
            response.raw_body,
            FIXTURE_INDEX_HTML.as_bytes(),
            "{path} body"
        );
    }
}

#[test]
fn static_routes_fail_closed_on_invalid_host() {
    let (_root, daemon) = start_daemon_with_fixture_assets();
    let port = daemon.local_addr().port();
    for path in ["/", "/admin", "/assets/fixture"] {
        let response = send_request(
            port,
            "GET",
            path,
            &[("Host".to_string(), "attacker.invalid".to_string())],
            b"",
        );
        assert_eq!(response.status, 400, "{path} status");
        assert_eq!(response.content_type, JSON_CONTENT_TYPE);
        assert_eq!(
            response.body(),
            json!({"error": "invalid_host"}),
            "{path} body"
        );
    }
}

#[test]
fn static_routes_stay_unauthenticated() {
    let (_root, daemon) = start_daemon_with_fixture_assets();
    let port = daemon.local_addr().port();
    // No cookie, no Origin: the frozen targets record credential <absent>.
    // Even a foreign Origin cannot matter — these routes have no Origin guard.
    for path in ["/", "/admin", "/assets/fixture"] {
        let response = send_request(
            port,
            "GET",
            path,
            &[("Origin".to_string(), "https://attacker.invalid".to_string())],
            b"",
        );
        assert_eq!(response.status, 200, "{path} must serve without a session");
    }
    // Non-GET requests are not static routes (Python served them only on GET).
    let post = send_request(port, "POST", "/", &[], b"");
    assert_eq!(post.status, 404);
    assert_eq!(post.body(), json!({"error": "not_found"}));
}

#[test]
fn console_without_assets_reports_not_built() {
    // The default daemon has an empty embedded set: web routes mirror the
    // Python "web_console_not_built" outcome until the console-build slice.
    let (_root, daemon) = start_daemon_on_ephemeral_port();
    let port = daemon.local_addr().port();
    for path in ["/", "/admin", "/config"] {
        let response = send_request(port, "GET", path, &[], b"");
        assert_eq!(response.status, 404, "{path} status");
        assert_eq!(response.content_type, JSON_CONTENT_TYPE);
        assert_eq!(
            response.body(),
            json!({"error": "web_console_not_built"}),
            "{path}"
        );
    }
    let missing = send_request(port, "GET", "/assets/fixture", &[], b"");
    assert_eq!(missing.status, 404);
    assert_eq!(missing.body(), json!({"error": "not_found"}));
}
