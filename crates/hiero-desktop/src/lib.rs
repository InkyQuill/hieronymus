//! Desktop presentation support kept outside the headless runtime graph.

mod icons;

pub use icons::render_icon;

#[cfg(target_os = "linux")]
mod events;
pub mod menu;
pub mod platform;
