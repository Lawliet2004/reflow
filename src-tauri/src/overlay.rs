use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use parking_lot::Mutex;
use tauri::{LogicalPosition, Manager};

use crate::commands::AppContext;
use crate::state::AppStateEnum;

static OVERLAY_GEN: AtomicU64 = AtomicU64::new(0);

struct OverlayGeom {
    position: String,
    kind: String,
}

fn overlay_geom() -> &'static Mutex<OverlayGeom> {
    static SLOT: OnceLock<Mutex<OverlayGeom>> = OnceLock::new();
    SLOT.get_or_init(|| {
        Mutex::new(OverlayGeom {
            position: "bottom_center".into(),
            kind: "listening".into(),
        })
    })
}

fn overlay_dims(kind: &str) -> (f64, f64) {
    match kind {
        "preview" => (520.0, 120.0),
        _ => (520.0, 56.0),
    }
}

fn position_overlay_sized(app: &tauri::AppHandle, position: &str, kind: &str) {
    let Some(window) = app.get_webview_window("overlay") else {
        return;
    };

    let Ok(Some(monitor)) = window.primary_monitor() else {
        return;
    };

    let scale = monitor.scale_factor();
    let screen = monitor.size();
    let origin = monitor.position();
    let (win_w, win_h) = overlay_dims(kind);
    let screen_w = screen.width as f64 / scale;
    let screen_h = screen.height as f64 / scale;
    let origin_x = origin.x as f64 / scale;
    let origin_y = origin.y as f64 / scale;

    let (x, y) = match position {
        "top_center" => (origin_x + (screen_w - win_w) / 2.0, origin_y + 40.0),
        "top_right" => (origin_x + screen_w - win_w - 24.0, origin_y + 40.0),
        "bottom_right" => (
            origin_x + screen_w - win_w - 24.0,
            origin_y + screen_h - win_h - 40.0,
        ),
        _ => (
            origin_x + (screen_w - win_w) / 2.0,
            origin_y + screen_h - win_h - 48.0,
        ),
    };

    let _ = window.set_size(tauri::LogicalSize::new(win_w, win_h));
    let _ = window.set_position(LogicalPosition::new(x, y));
}

pub fn position_overlay(app: &tauri::AppHandle, position: &str) {
    let kind = {
        let mut geom = overlay_geom().lock();
        geom.position = position.to_string();
        if geom.kind.is_empty() {
            geom.kind = "listening".into();
        }
        geom.kind.clone()
    };
    position_overlay_sized(app, position, &kind);
}

pub fn resize_overlay(app: &tauri::AppHandle, kind: &str) {
    let position = {
        let mut geom = overlay_geom().lock();
        geom.kind = kind.to_string();
        if geom.position.is_empty() {
            geom.position = "bottom_center".into();
        }
        geom.position.clone()
    };
    position_overlay_sized(app, &position, kind);
}

pub fn show_overlay(app: &tauri::AppHandle, position: &str) {
    OVERLAY_GEN.fetch_add(1, Ordering::SeqCst);
    {
        let mut geom = overlay_geom().lock();
        geom.position = position.to_string();
        geom.kind = "listening".into();
    }
    position_overlay_sized(app, position, "listening");
    if let Some(window) = app.get_webview_window("overlay") {
        let _ = window.set_ignore_cursor_events(true);
        let _ = window.set_always_on_top(true);
        show_without_activating(&window);
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

pub fn hide_overlay(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("overlay") {
        hide_native(&window);
        let _ = window.hide();
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
    resize_overlay(&app, "preview");
    let gen = OVERLAY_GEN.load(Ordering::SeqCst);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        if OVERLAY_GEN.load(Ordering::SeqCst) != gen {
            return;
        }
        let ctx = app.state::<AppContext>();
        let current = *ctx.state_enum.read();
        if !matches!(
            current,
            AppStateEnum::Recording | AppStateEnum::Processing | AppStateEnum::Injecting
        ) {
            hide_overlay(&app);
        }
    });
}
