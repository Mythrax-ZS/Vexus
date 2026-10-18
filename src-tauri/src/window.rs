//! The main window: creation, show/hide and remembered geometry.
//!
//! Geometry is persisted to Vexus's own state folder (not the OS app-data folder) so portable
//! mode stays truly portable, and a window saved on a monitor that no longer exists is moved
//! back on-screen.

use serde::{Deserialize, Serialize};
use tauri::window::Color;
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};
use vexus_core::settings::Theme;

use crate::state::AppState;

pub const MAIN: &str = "main";
const FILE: &str = "window.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct Geometry {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    maximized: bool,
}

pub fn create_main(app: &AppHandle, state: &AppState) -> tauri::Result<WebviewWindow> {
    // Match the page background so there is no white flash while WebView2 starts.
    let bg = match state.settings().theme {
        Theme::Light => Color(244, 245, 247, 255),
        _ => Color(14, 16, 20, 255),
    };
    let window = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("Vexus")
        .inner_size(1180.0, 780.0)
        .min_inner_size(980.0, 640.0)
        .center()
        .decorations(false)
        .shadow(true)
        .visible(false) // shown by `app_ready` once the UI has painted
        .background_color(bg)
        .data_directory(state.data.webview.clone())
        .build()?;
    restore_geometry(&window, state);

    // Failsafe: if the page never reports ready (a script error, a missing WebView2 piece), show
    // the window anyway so the app can't end up running invisibly.
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(5));
        if let Some(w) = handle.get_webview_window(MAIN)
            && !w.is_visible().unwrap_or(true)
        {
            tracing::warn!("the UI did not signal ready in time; showing the window anyway");
            show_main(&handle);
        }
    });
    Ok(window)
}

fn restore_geometry(window: &WebviewWindow, state: &AppState) {
    let Ok(bytes) = std::fs::read(state.data.state.join(FILE)) else { return };
    let Ok(g) = serde_json::from_slice::<Geometry>(&bytes) else { return };
    if g.width < 600 || g.height < 400 {
        return;
    }
    let on_screen = window.available_monitors().is_ok_and(|ms| {
        ms.iter().any(|m| {
            let (p, s) = (m.position(), m.size());
            let (cx, cy) = (g.x + (g.width / 2) as i32, g.y + (g.height / 2) as i32);
            cx >= p.x && cx < p.x + s.width as i32 && cy >= p.y && cy < p.y + s.height as i32
        })
    });
    if !on_screen {
        return;
    }
    let _ = window.set_size(PhysicalSize::new(g.width, g.height));
    let _ = window.set_position(PhysicalPosition::new(g.x, g.y));
    if g.maximized {
        let _ = window.maximize();
    }
}

pub fn save_geometry(window: &WebviewWindow, state: &AppState) {
    let maximized = window.is_maximized().unwrap_or(false);
    // While maximized, keep the previous *restored* size rather than the screen-sized one.
    let previous = std::fs::read(state.data.state.join(FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<Geometry>(&b).ok());
    let geometry = match (maximized, previous) {
        (true, Some(prev)) => Geometry { maximized: true, ..prev },
        _ => {
            let (Ok(pos), Ok(size)) = (window.outer_position(), window.inner_size()) else {
                return;
            };
            Geometry { x: pos.x, y: pos.y, width: size.width, height: size.height, maximized }
        }
    };
    if let Ok(json) = serde_json::to_vec(&geometry) {
        let _ = vexus_core::util::write_atomic(&state.data.state.join(FILE), &json);
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}
