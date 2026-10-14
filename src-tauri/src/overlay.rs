use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use parking_lot::Mutex;
use tauri::{Emitter, LogicalPosition, Manager};

use crate::commands::AppContext;
use crate::state::AppStateEnum;

static OVERLAY_GEN: AtomicU64 = AtomicU64::new(0);
static RESPONSE_FOCUS: Mutex<(isize, isize)> = Mutex::new((0, 0));

// The webview already leaves four logical pixels below the pill for its shadow.
pub(crate) const BOTTOM_MARGIN: i32 = 0;
const HIDE_TRANSITION_MS: u64 = 180;

fn response_target(foreground: isize, overlay: isize, previous: isize) -> isize {
    if overlay != 0 && foreground == overlay && previous != 0 {
        previous
    } else {
        foreground
    }
}

pub fn dictation_target(foreground: isize) -> isize {
    let (overlay, previous) = *RESPONSE_FOCUS.lock();
    response_target(foreground, overlay, previous)
}

pub fn restore_dictation_focus() -> Result<(), String> {
    let foreground = crate::platform::foreground_hwnd();
    let target = dictation_target(foreground);
    if target != foreground
        && !crate::platform::focus_hwnd_and_confirm(target, Duration::from_millis(300))
    {
        return Err("Focus the application you want to dictate into, then try again.".into());
    }
    Ok(())
}

#[derive(Clone)]
struct OverlayGeom {
    position: String,
    kind: String,
    size: String,
}

fn overlay_geom() -> &'static Mutex<OverlayGeom> {
    static SLOT: OnceLock<Mutex<OverlayGeom>> = OnceLock::new();
    SLOT.get_or_init(|| {
        Mutex::new(OverlayGeom {
            position: "bottom_center".into(),
            kind: "listening".into(),
            size: "standard".into(),
        })
    })
}

/// The capsule (compact 172 × 34, standard 200 × 40, large 240 × 48) with four
/// logical pixels around it for the shadow. The settled result uses the same
/// size, so completion never moves the HUD. Must match `.hud-scale-*` in CSS.
fn overlay_dims(kind: &str, size: &str) -> (f64, f64) {
    if kind == "response" {
        return (320.0, 180.0);
    }
    match size {
        "compact" => (180.0, 42.0),
        "large" => (248.0, 56.0),
        _ => (208.0, 48.0),
    }
}

fn position_overlay_sized(app: &tauri::AppHandle) {
    let OverlayGeom {
        position,
        kind,
        size,
    } = overlay_geom().lock().clone();
    let Some(window) = app.get_webview_window("overlay") else {
        return;
    };
    let (win_w, win_h) = overlay_dims(&kind, &size);

    #[cfg(target_os = "linux")]
    if crate::overlay_layer::is_active() {
        crate::overlay_layer::anchor(app, &position, win_w as i32, win_h as i32);
        return;
    }

    let Ok(Some(monitor)) = window.primary_monitor() else {
        return;
    };

    let scale = monitor.scale_factor();
    let screen = monitor.size();
    let origin = monitor.position();
    let screen_w = screen.width as f64 / scale;
    let origin_x = origin.x as f64 / scale;
    let origin_y = origin.y as f64 / scale;
    let work_area = monitor.work_area();
    let bottom = (work_area.position.y as f64 + work_area.size.height as f64) / scale
        - win_h
        - f64::from(BOTTOM_MARGIN);

    let left = origin_x + 24.0;
    let center = origin_x + (screen_w - win_w) / 2.0;
    let right = origin_x + screen_w - win_w - 24.0;
    let top = origin_y + 40.0;
    let (x, y) = match position.as_str() {
        "top_center" => (center, top),
        "top_left" => (left, top),
        "top_right" => (right, top),
        "bottom_left" => (left, bottom),
        "bottom_right" => (right, bottom),
        _ => (center, bottom),
    };

    let _ = window.set_size(tauri::LogicalSize::new(win_w, win_h));
    let _ = window.set_position(LogicalPosition::new(x, y));
}

/// Applies the user's anchor and capsule size from settings.
pub fn position_overlay(app: &tauri::AppHandle, position: &str, size: &str) {
    {
        let mut geom = overlay_geom().lock();
        geom.position = position.to_string();
        geom.size = size.to_string();
    }
    position_overlay_sized(app);
}

pub fn resize_overlay(app: &tauri::AppHandle, kind: &str) {
    overlay_geom().lock().kind = kind.to_string();
    position_overlay_sized(app);
}

pub fn show_overlay(app: &tauri::AppHandle, position: &str, size: &str) {
    OVERLAY_GEN.fetch_add(1, Ordering::SeqCst);
    {
        let mut geom = overlay_geom().lock();
        geom.position = position.to_string();
        geom.size = size.to_string();
        geom.kind = "listening".into();
    }
    position_overlay_sized(app);
    if let Some(window) = app.get_webview_window("overlay") {
        let _ = window.set_focusable(false);
        let _ = window.set_always_on_top(true);
        // GTK has no native window until the overlay is first shown, and tao
        // aborts the process when click-through is applied before that.
        #[cfg(not(target_os = "linux"))]
        let _ = window.set_ignore_cursor_events(true);
        #[cfg(target_os = "linux")]
        crate::overlay_layer::set_interactive(app, false);
        show_without_activating(&window);
        #[cfg(target_os = "linux")]
        let _ = window.set_ignore_cursor_events(true);
        let _ = window.emit("window:visibility", true);
    }
}

fn show_without_activating(window: &tauri::WebviewWindow) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, HWND_TOPMOST,
            SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, WINDOW_EX_STYLE,
            WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
        };
        if let Ok(raw) = window.hwnd() {
            let hwnd = HWND(raw.0);
            unsafe {
                let mut ex = WINDOW_EX_STYLE(GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32);
                ex |= WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex.0 as isize);
                let _ = SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
            return;
        }
    }
    let _ = window.show();
}

pub fn show_response(app: &tauri::AppHandle) {
    OVERLAY_GEN.fetch_add(1, Ordering::SeqCst);
    resize_overlay(app, "response");
    if let Some(window) = app.get_webview_window("overlay") {
        let _ = window.set_focusable(true);
        let _ = window.set_ignore_cursor_events(false);
        #[cfg(windows)]
        if let Ok(raw) = window.hwnd() {
            use windows::Win32::Foundation::HWND;
            use windows::Win32::UI::WindowsAndMessaging::{
                GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE,
            };
            unsafe {
                let hwnd = HWND(raw.0);
                let foreground = crate::platform::foreground_hwnd();
                let mut focus = RESPONSE_FOCUS.lock();
                if foreground != raw.0 as isize {
                    *focus = (raw.0 as isize, foreground);
                }
                let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style & !(WS_EX_NOACTIVATE.0 as isize));
            }
        }
        #[cfg(target_os = "linux")]
        crate::overlay_layer::set_interactive(app, true);
        let _ = window.show();
        let _ = window.emit("window:visibility", true);
        let _ = window.set_focus();
    }
}

pub fn hide_overlay(app: &tauri::AppHandle) {
    let foreground = crate::platform::foreground_hwnd();
    let target = dictation_target(foreground);
    if target != foreground {
        crate::platform::focus_hwnd(target);
    }
    if let Some(window) = app.get_webview_window("overlay") {
        hide_native(&window);
        let _ = window.hide();
        let _ = window.emit("window:visibility", false);
    }
}

fn hide_native(window: &tauri::WebviewWindow) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
        if let Ok(raw) = window.hwnd() {
            let hwnd = HWND(raw.0);
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }
    let _ = window;
}

pub fn hide_overlay_later(app: tauri::AppHandle, delay_ms: u64) {
    if overlay_geom().lock().kind == "response" {
        return;
    }
    resize_overlay(&app, "preview");
    // A newer result or recording supersedes both the hold and the fade.
    let gen = OVERLAY_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    if let Some(window) = app.get_webview_window("overlay") {
        let _ = window.emit("hud:hide", false);
    }
    tauri::async_runtime::spawn(async move {
        let transition_ms = delay_ms.min(HIDE_TRANSITION_MS);
        tokio::time::sleep(Duration::from_millis(delay_ms - transition_ms)).await;
        if OVERLAY_GEN.load(Ordering::SeqCst) != gen {
            return;
        }
        let ctx = app.state::<AppContext>();
        let current = *ctx.state_enum.read();
        if !matches!(
            current,
            AppStateEnum::Recording | AppStateEnum::Processing | AppStateEnum::Injecting
        ) {
            if let Some(window) = app.get_webview_window("overlay") {
                let _ = window.emit("hud:hide", true);
            }
            tokio::time::sleep(Duration::from_millis(transition_ms)).await;
            if OVERLAY_GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            let current = *ctx.state_enum.read();
            if matches!(
                current,
                AppStateEnum::Recording | AppStateEnum::Processing | AppStateEnum::Injecting
            ) {
                return;
            }
            hide_overlay(&app);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::overlay_dims;

    #[test]
    fn a_focused_assistant_preserves_the_external_dictation_target() {
        assert_eq!(super::response_target(22, 22, 11), 11);
        assert_eq!(super::response_target(33, 22, 11), 33);
        assert_eq!(super::response_target(0, 0, 11), 0);
        assert_eq!(super::response_target(22, 22, 0), 22);
    }

    #[test]
    fn capsule_geometry_is_stable_and_matches_the_initial_window() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let window = config["app"]["windows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|window| window["label"] == "overlay")
            .unwrap();
        let initial = (
            window["width"].as_f64().unwrap(),
            window["height"].as_f64().unwrap(),
        );
        for phase in ["listening", "processing", "polishing", "preview"] {
            assert_eq!(overlay_dims(phase, "standard"), initial);
            assert!(overlay_dims(phase, "compact").0 < initial.0);
            assert!(overlay_dims(phase, "large").0 > initial.0);
        }
        assert!(initial.0 <= 208.0 && initial.1 <= 48.0);
        assert_eq!(window["focusable"], false);
    }
}
