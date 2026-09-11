#[cfg(target_os = "linux")]
mod linux;
use hieronymus::data_root::HieronymusConfig;

pub fn run(config: HieronymusConfig) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        linux::run(config)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = config;
        Err("The native desktop helper is not implemented for this platform".into())
    }
}
