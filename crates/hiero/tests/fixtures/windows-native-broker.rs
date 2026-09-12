//! Transport-only child; never contacts Task Scheduler, a browser or a daemon.
#![windows_subsystem = "windows"]
#[path = "../../src/platform/native_gate.rs"]
mod native_gate;
#[path = "../../src/platform/native_protocol.rs"]
mod native_protocol;
use std::io::{BufRead, Write};
fn main() {
    let exe = std::env::current_exe().unwrap();
    let directory = exe.parent().unwrap();
    let before = exe.file_stem().unwrap() == "before";
    let browser = exe.file_stem().unwrap() == "browser";
    let delayed_exit = exe.file_stem().unwrap() == "delayed-exit";
    let mut input = std::io::stdin().lock();
    let mut request = String::new();
    input.read_line(&mut request).unwrap();
    if before {
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
    let root = native_gate::acquire(
        &directory.join("root"),
        if browser {
            native_gate::BROWSER_GATE
        } else {
            native_gate::MANAGER_GATE
        },
    )
    .unwrap();
    let registration = if browser {
        None
    } else {
        Some(native_gate::acquire(&directory.join("units"), native_gate::MANAGER_GATE).unwrap())
    };
    let result = native_protocol::committed(
        &mut input,
        &mut std::io::stdout(),
        (root, registration),
        || {
            std::fs::write(directory.join("committed"), "yes").unwrap();
            if !delayed_exit {
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
            Ok(())
        },
    );
    if result.is_ok() {
        let _ = std::io::stdout().write_all(b"{\"Ok\":null}\n");
        std::io::stdout().flush().unwrap();
        if delayed_exit {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
}
