use std::process::ExitCode;
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("hiero-desktop: {error}");
            ExitCode::from(2)
        }
    }
}
fn run() -> Result<(), String> {
    let mut arguments = std::env::args_os().skip(1);
    let root = match arguments.next().as_deref() {
        Some(flag) if flag == "--data-root" => {
            Some(arguments.next().ok_or("--data-root requires a path")?)
        }
        None => None,
        _ => return Err("usage: hiero-desktop [--data-root <path>]".into()),
    };
    if arguments.next().is_some() {
        return Err("usage: hiero-desktop [--data-root <path>]".into());
    }
    let config = hieronymus::data_root::load_config(root.as_deref().map(std::path::Path::new));
    let root = std::path::absolute(config.data_root())
        .map_err(|_| "Could not resolve the desktop data root")?;
    hiero_desktop::platform::run(hieronymus::data_root::HieronymusConfig::new(root))
}
