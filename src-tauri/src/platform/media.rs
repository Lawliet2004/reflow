//! Windows session volumes live on one COM thread for capture and restoration.
#[cfg(windows)]
enum Request {
    Duck(std::sync::mpsc::Sender<Result<(), String>>),
    Restore,
}

#[cfg(windows)]
fn worker() -> &'static std::sync::mpsc::Sender<Request> {
    static CHANNEL: std::sync::OnceLock<std::sync::mpsc::Sender<Request>> =
        std::sync::OnceLock::new();
    CHANNEL.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || unsafe {
            use windows::{
                core::Interface,
                Win32::{Media::Audio::*, System::Com::*},
            };
            let initialized = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            let mut saved: Vec<(ISimpleAudioVolume, f32)> = Vec::new();
            let restore = |saved: &mut Vec<(ISimpleAudioVolume, f32)>| {
                for (volume, original) in saved.drain(..) {
                    // Preserve volume changes made by the user during capture.
                    if volume
                        .GetMasterVolume()
                        .is_ok_and(|current| (current - original * 0.5).abs() < 0.001)
                    {
                        if let Err(error) = volume.SetMasterVolume(original, std::ptr::null()) {
                            log::warn!("Could not restore media volume: {error}");
                        }
                    }
                }
            };
            while let Ok(request) = receiver.recv() {
                match request {
                    Request::Restore => restore(&mut saved),
                    Request::Duck(reply) => {
                        let result = (|| -> Result<(), String> {
                            if !initialized {
                                return Err("Media duck COM initialization failed".into());
                            }
                            if !saved.is_empty() {
                                return Ok(());
                            }
                            let enumerator: IMMDeviceEnumerator =
                                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                                    .map_err(|e| e.to_string())?;
                            let endpoints = enumerator
                                .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
                                .map_err(|e| e.to_string())?;
                            for device in 0..endpoints.GetCount().map_err(|e| e.to_string())? {
                                let endpoint = endpoints.Item(device).map_err(|e| e.to_string())?;
                                let manager: IAudioSessionManager2 = endpoint
                                    .Activate(CLSCTX_ALL, None)
                                    .map_err(|e| e.to_string())?;
                                let sessions =
                                    manager.GetSessionEnumerator().map_err(|e| e.to_string())?;
                                for i in 0..sessions.GetCount().map_err(|e| e.to_string())? {
                                    let session =
                                        sessions.GetSession(i).map_err(|e| e.to_string())?;
                                    let control: IAudioSessionControl2 =
                                        session.cast().map_err(|e| e.to_string())?;
                                    if control.GetProcessId().map_err(|e| e.to_string())?
                                        == std::process::id()
                                    {
                                        continue;
                                    }
                                    let volume: ISimpleAudioVolume =
                                        session.cast().map_err(|e| e.to_string())?;
                                    let original =
                                        volume.GetMasterVolume().map_err(|e| e.to_string())?;
                                    volume
                                        .SetMasterVolume(original * 0.5, std::ptr::null())
                                        .map_err(|e| e.to_string())?;
                                    saved.push((volume, original));
                                }
                            }
                            Ok(())
                        })();
                        if result.is_err() {
                            restore(&mut saved);
                        }
                        let _ = reply.send(result);
                    }
                }
            }
            restore(&mut saved);
            if initialized {
                CoUninitialize();
            }
        });
        sender
    })
}
pub fn duck() -> Result<(), String> {
    #[cfg(windows)]
    {
        let (reply, result) = std::sync::mpsc::channel();
        worker()
            .send(Request::Duck(reply))
            .map_err(|e| e.to_string())?;
        result
            .recv_timeout(std::time::Duration::from_millis(500))
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(windows))]
    {
        Ok(())
    }
}
pub fn restore() {
    #[cfg(windows)]
    {
        let _ = worker().send(Request::Restore);
    }
}

pub fn on_battery() -> Option<bool> {
    #[cfg(windows)]
    {
        use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        let mut status = SYSTEM_POWER_STATUS::default();
        unsafe {
            GetSystemPowerStatus(&mut status).ok()?;
        }
        match status.ACLineStatus {
            0 => Some(true),
            1 => Some(false),
            _ => None,
        }
    }
    #[cfg(target_os = "linux")]
    {
        let entries = std::fs::read_dir("/sys/class/power_supply").ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if std::fs::read_to_string(path.join("type")).is_ok_and(|s| s.trim() == "Battery") {
                return std::fs::read_to_string(path.join("status"))
                    .ok()
                    .map(|s| s.trim() == "Discharging");
            }
        }
        None
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}
