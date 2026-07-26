#![cfg(unix)]

use std::{process::Stdio, time::Duration};

use assert_cmd::cargo::cargo_bin;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

async fn random_port() -> u16 {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    listener.local_addr().unwrap().port()
}

fn daemon_command(root: &TempDir, port: u16, explicit_start: bool) -> Command {
    let mut command = Command::new(cargo_bin!("hiero"));
    command
        .arg("--data-root")
        .arg(root.path().join("data"))
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("HIERONYMUS_PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if explicit_start {
        command.arg("start").arg("--port").arg(port.to_string());
    }
    command
}

async fn spawn_daemon(root: &TempDir, port: u16) -> Child {
    let child = daemon_command(root, port, true).spawn().unwrap();
    let token_path = root
        .path()
        .join("config")
        .join("hieronymus")
        .join("auth-token");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let (Ok(token), Ok(mut stream)) = (
                std::fs::read_to_string(&token_path),
                tokio::net::TcpStream::connect(("127.0.0.1", port)).await,
            ) {
                let request = format!(
                    "GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
                     X-Hieronymus-Token: {}\r\nConnection: close\r\n\r\n",
                    token.trim()
                );
                let mut response = [0_u8; 1_024];
                if stream.write_all(request.as_bytes()).await.is_ok()
                    && stream
                        .read(&mut response)
                        .await
                        .is_ok_and(|read| response[..read].starts_with(b"HTTP/1.1 200"))
                {
                    break;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("daemon should start");
    child
}

async fn assert_signal_stops_daemon(signal: &str) {
    let root = TempDir::new().unwrap();
    let port = random_port().await;
    let mut child = spawn_daemon(&root, port).await;
    let pid = child.id().unwrap();
    let status = Command::new("kill")
        .arg(signal)
        .arg(pid.to_string())
        .status()
        .await
        .unwrap();
    assert!(status.success());
    let exit = tokio::time::timeout(Duration::from_secs(8), child.wait())
        .await
        .expect("signal should stop the daemon")
        .unwrap();
    assert!(exit.success(), "daemon exit: {exit}");
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn ctrl_c_stops_listener_and_workers_cleanly() {
    assert_signal_stops_daemon("-INT").await;
}

#[tokio::test]
async fn sigterm_stops_listener_and_workers_cleanly() {
    assert_signal_stops_daemon("-TERM").await;
}

#[tokio::test]
async fn no_subcommand_prints_status_when_daemon_is_already_running() {
    let root = TempDir::new().unwrap();
    let port = random_port().await;
    let mut daemon = spawn_daemon(&root, port).await;
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        daemon_command(&root, port, false).output(),
    )
    .await
    .expect("no-subcommand status should not block")
    .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Hieronymus daemon is running\n"
    );
    assert!(daemon.try_wait().unwrap().is_none());
    Command::new("kill")
        .arg("-TERM")
        .arg(daemon.id().unwrap().to_string())
        .status()
        .await
        .unwrap();
    daemon.wait().await.unwrap();
}
