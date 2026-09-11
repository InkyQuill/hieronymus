#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;
use hieronymus::data_root::HieronymusConfig;

pub fn run(config: HieronymusConfig) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        linux::run(config)
    }
    #[cfg(windows)]
    {
        windows::run(config)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = config;
        Err("The native desktop helper is not implemented for this platform".into())
    }
}

#[cfg(windows)]
pub fn run_with_service_options(
    config: HieronymusConfig,
    options: hiero::service::ServiceOptions,
) -> Result<(), String> {
    windows::run_with_service_options(config, options)
}

#[cfg(any(windows, test))]
mod windows_pixels;

#[cfg(any(windows, test))]
mod windows_events;
