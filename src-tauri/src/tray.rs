//! System tray: quick status and a few actions without opening the window.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};
use vexus_core::model::DeletionPolicy;
use vexus_core::scan::Scan;
use vexus_core::util::human_bytes;

use crate::state::AppState;
use crate::window;

const TRAY_ID: &str = "vexus-tray";

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Vexus", true, None::<&str>)?;
    let scan = MenuItem::with_id(app, "scan", "Smart Scan", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Vexus", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &scan, &PredefinedMenuItem::separator(app)?, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Vexus")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => window::show_main(app),
            "scan" => {
                window::show_main(app);
                let _ = app.emit("app:command", "scan");
            }
            "quit" => {
                if let Some(state) = app.try_state::<AppState>() {
                    state.quitting.store(true, std::sync::atomic::Ordering::Relaxed);
                    state.cancel_job();
                }
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                window::show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Updates the tooltip with the latest numbers: what the recommended clean would reclaim, or
/// plain disk space if there's no scan in memory.
pub fn refresh(app: &AppHandle, state: &AppState, scan: Option<&Scan>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let text = match scan {
        Some(scan) if !scan.cancelled => {
            let plan = scan.plan(&scan.default_selection(), DeletionPolicy::Smart);
            if plan.bytes > 0 {
                format!("Vexus — {} can be cleaned safely", human_bytes(plan.bytes))
            } else {
                "Vexus — nothing to clean right now".to_string()
            }
        }
        _ => {
            let disks = vexus_core::system::disks(&state.engine.env().system_drive);
            match disks.iter().find(|d| d.system) {
                Some(d) => format!(
                    "Vexus — {} free on {}",
                    human_bytes(d.free),
                    d.root.trim_end_matches('\\')
                ),
                None => "Vexus".to_string(),
            }
        }
    };
    let _ = tray.set_tooltip(Some(text));
}
