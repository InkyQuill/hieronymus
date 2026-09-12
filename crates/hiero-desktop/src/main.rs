#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use std::process::ExitCode;
fn main() -> ExitCode {
    if std::env::args().skip(1).collect::<Vec<_>>() == ["version", "--json"] {
        println!(
            "{{\"version\":\"{}\",\"target\":\"{}\"}}",
            env!("CARGO_PKG_VERSION"),
            hiero::app::TARGET_TRIPLE
        );
        return ExitCode::SUCCESS;
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("hiero-desktop: {error}");
            ExitCode::from(2)
        }
    }
}
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn run() -> Result<(), String> {
    let mut root = None;
    let mut unit = None;
    let mut binary = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(flag) = args.next() {
        if flag == "--resume" {
            continue;
        }
        let target = match flag.to_str() {
            Some("--data-root") => &mut root,
            Some("--unit-dir") => &mut unit,
            Some("--binary") => &mut binary,
            _ => {
                return Err(
                    "usage: hiero-desktop [--data-root PATH] [--unit-dir DIR] [--binary CLI]"
                        .into(),
                );
            }
        };
        *target = Some(
            std::path::absolute(args.next().ok_or("Desktop option requires a path")?)
                .map_err(|_| "Could not resolve desktop path")?,
        );
    }
    let config = hieronymus::data_root::load_config(root.as_deref());
    let root =
        std::path::absolute(config.data_root()).map_err(|_| "Could not resolve desktop root")?;
    let binary = match binary {
        Some(path) => path,
        None => hiero::desktop::launch::stable_cli(
            &std::env::current_exe().map_err(|_| "Could not locate helper")?,
        )?,
    };
    let options = hiero::service::ServiceOptions {
        data_root: root.clone(),
        unit_dir: unit.unwrap_or_else(hiero::service::default_unit_dir),
        binary,
        use_manager: true,
    };
    hiero_desktop::platform::run_with_service_options(
        hieronymus::data_root::HieronymusConfig::new(root),
        options,
    )
}
