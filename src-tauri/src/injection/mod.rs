pub(crate) mod cancellation;
mod clipboard;
pub mod injector;
mod restore_chain;
#[cfg(target_os = "linux")]
mod wayland_clipboard;

pub use injector::{InjectionOutcome, TextInjector};
