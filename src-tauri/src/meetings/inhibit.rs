//! Idle sleep only; forced suspend and display sleep remain under OS control.
pub struct Inhibitor {
    #[cfg(target_os = "linux")]
    _fd: zbus::zvariant::OwnedFd,
}
impl Inhibitor {
    pub fn acquire() -> Result<Self, String> {
        #[cfg(target_os = "linux")]
        {
            tauri::async_runtime::block_on(async {
                let conn = zbus::Connection::system()
                    .await
                    .map_err(|e| e.to_string())?;
                let proxy = zbus::Proxy::new(
                    &conn,
                    "org.freedesktop.login1",
                    "/org/freedesktop/login1",
                    "org.freedesktop.login1.Manager",
                )
                .await
                .map_err(|e| e.to_string())?;
                let fd: zbus::zvariant::OwnedFd = proxy
                    .call(
                        "Inhibit",
                        &("idle", "Reflow", "Recording a meeting", "block"),
                    )
                    .await
                    .map_err(|e| format!("Cannot inhibit idle sleep: {e}"))?;
                Ok(Self { _fd: fd })
            })
        }
        #[cfg(target_os = "windows")]
        unsafe {
            use windows::Win32::System::Power::*;
            if SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED).0 == 0 {
                return Err("Cannot inhibit idle sleep".into());
            }
        }
        #[cfg(not(target_os = "linux"))]
        Ok(Self {})
    }
}
impl Drop for Inhibitor {
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        unsafe {
            use windows::Win32::System::Power::*;
            SetThreadExecutionState(ES_CONTINUOUS);
        }
    }
}
