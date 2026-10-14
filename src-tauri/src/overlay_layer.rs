//! Wayland HUD placement through wlr-layer-shell. KWin and wlroots honor anchors
//! and keyboard mode here, where GTK focus flags and absolute positions are ignored.
use std::sync::atomic::{AtomicBool, Ordering};

use gtk::prelude::*;
use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use tauri::{AppHandle, Manager};

use crate::overlay::BOTTOM_MARGIN;

static ACTIVE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Placement {
    pub top: Option<i32>,
    pub bottom: Option<i32>,
    pub left: Option<i32>,
    pub right: Option<i32>,
}

/// Anchored edges carry their margin; `None` leaves the edge unanchored.
pub(crate) fn placement(position: &str) -> Placement {
    match position {
        "top_center" => Placement {
            top: Some(40),
            ..Default::default()
        },
        "top_left" => Placement {
            top: Some(40),
            left: Some(24),
            ..Default::default()
        },
        "top_right" => Placement {
            top: Some(40),
            right: Some(24),
            ..Default::default()
        },
        "bottom_left" => Placement {
            bottom: Some(BOTTOM_MARGIN),
            left: Some(24),
            ..Default::default()
        },
        "bottom_right" => Placement {
            bottom: Some(BOTTOM_MARGIN),
            right: Some(24),
            ..Default::default()
        },
        _ => Placement {
            bottom: Some(BOTTOM_MARGIN),
            ..Default::default()
        },
    }
}

pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

fn apply_placement(window: &tauri::WebviewWindow, placement: Placement) {
    let Ok(gtk_window) = window.gtk_window() else {
        return;
    };
    for (edge, margin) in [
        (Edge::Top, placement.top),
        (Edge::Bottom, placement.bottom),
        (Edge::Left, placement.left),
        (Edge::Right, placement.right),
    ] {
        gtk_window.set_anchor(edge, margin.is_some());
        if let Some(margin) = margin {
            gtk_window.set_layer_shell_margin(edge, margin);
        }
    }
}

/// Must run on the main thread before the overlay is first shown.
pub fn init(app: &AppHandle) {
    if !gtk_layer_shell::is_supported() {
        return;
    }
    let Some(window) = app.get_webview_window("overlay") else {
        return;
    };
    let Ok(gtk_window) = window.gtk_window() else {
        return;
    };
    gtk_window.init_layer_shell();
    if !gtk_window.is_layer_window() {
        log::warn!(
            "Wayland HUD could not become a layer surface; desktop HUD support remains unavailable"
        );
        return;
    }
    gtk_window.set_layer(Layer::Overlay);
    gtk_window.set_keyboard_mode(KeyboardMode::None);
    gtk_window.set_namespace("reflow-hud");
    gtk_window.set_exclusive_zone(0);
    // GTK otherwise retains a larger allocation after an assistant response.
    // Layer-shell owns the geometry, so this does not enable desktop resizing.
    gtk_window.set_resizable(true);
    apply_placement(&window, placement("bottom_center"));
    ACTIVE.store(true, Ordering::Release);
    log::info!("Wayland HUD initialized as a layer surface with keyboard focus disabled");
    crate::platform::desktop::hud(true);
}

pub fn anchor(app: &AppHandle, position: &str, width: i32, height: i32) {
    if !is_active() {
        return;
    }
    let Some(window) = app.get_webview_window("overlay") else {
        return;
    };
    let placement = placement(position);
    let _ = app.run_on_main_thread(move || {
        apply_placement(&window, placement);
        let sized_window = window.clone();
        // WebKit's natural minimum height is 200 logical pixels. Merely
        // resizing the GTK window keeps that minimum, placing the pill at
        // the top of a large transparent surface above the bottom edge.
        let _ = window.with_webview(move |webview| {
            webview.inner().set_size_request(width, height);
            if let Ok(gtk_window) = sized_window.gtk_window() {
                gtk_window.set_size_request(width, height);
                // Layer-shell uses the explicit widget request as its size.
                gtk_window.resize(1, 1);
            }
        });
    });
}

pub fn set_interactive(app: &AppHandle, interactive: bool) {
    if !is_active() {
        return;
    }
    let Some(window) = app.get_webview_window("overlay") else {
        return;
    };
    let mode = if interactive {
        KeyboardMode::OnDemand
    } else {
        KeyboardMode::None
    };
    let _ = app.run_on_main_thread(move || {
        if let Ok(gtk_window) = window.gtk_window() {
            gtk_window.set_keyboard_mode(mode);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{placement, Placement};

    #[test]
    fn bottom_center_is_anchored_to_the_bottom_only() {
        assert_eq!(
            placement("bottom_center"),
            Placement {
                top: None,
                bottom: Some(0),
                left: None,
                right: None,
            }
        );
    }

    #[test]
    fn corners_anchor_one_vertical_and_one_horizontal_edge() {
        assert_eq!(
            placement("bottom_left"),
            Placement {
                top: None,
                bottom: Some(0),
                left: Some(24),
                right: None,
            }
        );
        assert_eq!(
            placement("top_right"),
            Placement {
                top: Some(40),
                bottom: None,
                left: None,
                right: Some(24),
            }
        );
        assert_eq!(
            placement("top_center"),
            Placement {
                top: Some(40),
                bottom: None,
                left: None,
                right: None,
            }
        );
    }

    #[test]
    fn unknown_position_falls_back_to_bottom_center() {
        assert_eq!(placement("nowhere"), placement("bottom_center"));
    }
}
