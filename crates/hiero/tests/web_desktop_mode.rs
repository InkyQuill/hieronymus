//! Default local browser access and explicitly enabled session authentication.
mod common;
use common::{same_origin, send_request, start_daemon};

#[test]
fn default_browser_access_needs_no_cookie_but_preserves_origin_and_mcp_guards() {
    let root = tempfile::tempdir().unwrap();
    let daemon = start_daemon(root.path());
    let port = daemon.local_addr().port();
    assert_eq!(
        send_request(port, "GET", "/api/providers", &[], b"").status,
        200
    );
    assert_eq!(
        send_request(
            port,
            "GET",
            "/api/providers",
            &[("Origin".into(), "https://foreign.invalid".into())],
            b""
        )
        .status,
        403
    );
    assert_eq!(
        send_request(port, "POST", "/api/providers", &[], b"{}").status,
        403
    );
    let origin = vec![("Origin".into(), same_origin(port))];
    // Valid origin reaches payload validation without a browser cookie.
    assert_eq!(
        send_request(port, "POST", "/api/providers", &origin, b"{}").status,
        400
    );
    assert_eq!(
        send_request(port, "GET", "/ws/admin", &origin, b"").status,
        400
    );
    assert_eq!(send_request(port, "GET", "/status", &[], b"").status, 401);
    assert_eq!(
        send_request(port, "POST", "/auth/launch-grant", &[], b"{}").status,
        401
    );
    daemon.shutdown().unwrap();
}

#[test]
fn connection_setup_prepares_both_mcp_and_skills_without_claiming_installation() {
    let root = tempfile::tempdir().unwrap();
    let daemon = start_daemon(root.path());
    let port = daemon.local_addr().port();
    let response = send_request(
        port,
        "POST",
        "/api/agents/prepare",
        &[("Origin".into(), same_origin(port))],
        b"{}",
    );
    assert_eq!(response.status, 200);
    let payload: serde_json::Value = serde_json::from_slice(&response.raw_body).unwrap();
    let agents = payload["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 3);
    for agent in agents {
        let instructions = agent["instructions"].as_str().unwrap();
        assert!(instructions.contains("add its MCP connection"));
        assert!(instructions.contains("install its skills"));
        assert!(instructions.contains("Verify both setup steps separately"));
        assert!(instructions.contains("report any incomplete step"));
    }
    daemon.shutdown().unwrap();
}

#[test]
fn explicit_browser_authentication_guards_rest_and_websocket() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("web.conf"),
        "authentication_required = true\n",
    )
    .unwrap();
    let daemon = start_daemon(root.path());
    let port = daemon.local_addr().port();
    for path in ["/api/providers", "/ws/admin"] {
        assert_eq!(
            send_request(
                port,
                "GET",
                path,
                &[("Origin".into(), same_origin(port))],
                b""
            )
            .status,
            401
        );
    }
    daemon.shutdown().unwrap();
}

#[test]
fn malformed_authentication_configuration_refuses_server_startup() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("web.conf"),
        "authentication_required = 'false'\n",
    )
    .unwrap();
    let result = hiero::daemon::Daemon::start(&hiero::daemon::DaemonOptions {
        data_root: Some(root.path().into()),
        port: 0,
        assets: hiero::daemon::Assets::default(),
    });
    assert!(matches!(
        result,
        Err(hiero::daemon::DaemonError::WebConfig(_))
    ));
    assert!(!root.path().join("daemon.json").exists());
}
