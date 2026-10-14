pub(crate) mod dispatch_guard;
pub mod hook;
pub mod manager;

pub use manager::{HotkeyAction, HotkeyManager};
pub mod registry;

#[cfg(target_os = "linux")]
pub mod portal;
