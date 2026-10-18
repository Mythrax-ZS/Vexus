//! Vexus desktop shell.
//!
//! This crate is deliberately thin: windowing, tray, IPC and persistence of settings. Every
//! decision about what may be removed is made by `vexus-core`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod args;
mod commands;
mod elevation;
mod logging;
mod osinfo;
mod state;
mod tray;
mod window;

use std::path::{Path, PathBuf};
use std::time::Duration;

use tauri::{Manager, WindowEvent};
use vexus_core::env::Env;
use vexus_core::paths::{DataDirs, DataMode};
use vexus_core::settings::Settings;
use vexus_core::util::now_unix;
use vexus_core::{Engine, system};

use crate::args::Args;
use crate::logging::Logging;
use crate::state::{AppState, JobKind};

/// Reports a start-up problem where the user will see it (the release build has no console).
fn fatal(message: &str) -> ! {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    use windows::core::{HSTRING, w};
    eprintln!("Vexus: {message}");
    // SAFETY: both strings are valid for the duration of the call; no owner window.
    unsafe {
        MessageBoxW(None, &HSTRING::from(message), w!("Vexus can't start"), MB_OK | MB_ICONERROR);
    }
    std::process::exit(2);
}

fn main() {
    let args = Args::parse();
    if !args.errors.is_empty() {
        fatal(&args.errors.join("\n"));
    }
    if let Some(pid) = args.wait_pid {
        elevation::wait_for_exit(pid, Duration::from_secs(10));
    }

    let (env, mode) = match &args.sandbox {
        Some(dir) => {
            let root = prepare_sandbox(dir);
            (Env::sandbox(&root), DataMode::Custom(root.join(".vexus")))
        }
        None => {
            let exe_dir =
                std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
            let mode = match (&exe_dir, args.portable) {
                (Some(dir), true) => DataMode::Portable { exe_dir: dir.clone() },
                _ => DataMode::detect(exe_dir.as_deref()),
            };
            (Env::detect(), mode)
        }
    };
    let data = DataDirs::resolve(&mode, &env);
    if let Err(e) = data.ensure() {
        eprintln!("Vexus couldn't create its data folder {}: {e}", data.root.display());
    }
    let settings = Settings::load(&data.state);
    let logging = Logging::init(&data.logs, settings.verbose_logging || args.verbose);

    let elevated = system::is_elevated();
    let sandboxed = env.sandboxed;
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = %osinfo::describe(),
        elevated,
        sandboxed,
        portable = data.portable,
        data = %data.root.display(),
        "Vexus starting"
    );

    let engine = Engine::new(env, &data, elevated);
    let state = AppState::new(engine, data, args.clone(), sandboxed, elevated, settings, logging);

    let mut builder = tauri::Builder::default();
    if !sandboxed {
        // One real instance at a time. Sandbox runs are independent so they can sit beside it.
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::show_main(app);
        }));
    }

    let setup_state = state.clone();
    builder
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::app_summary,
            commands::system_overview,
            commands::settings_set,
            commands::scan_start,
            commands::job_cancel,
            commands::scan_items,
            commands::selection_apply,
            commands::clean_start,
            commands::recovery_list,
            commands::recovery_restore,
            commands::recovery_purge,
            commands::reveal_item,
            commands::open_folder,
            commands::restart_elevated,
            commands::app_ready,
            commands::quit_app,
            commands::log_frontend,
        ])
        .setup(move |app| {
            window::create_main(app.handle(), &setup_state)?;
            tray::build(app.handle())?;
            tray::refresh(app.handle(), &setup_state, None);

            // Housekeeping off the UI path: free Recovery copies whose time is up — at start-up
            // and then hourly, so a long-running (tray) instance doesn't hold on to expired data.
            let st = setup_state.clone();
            let _ =
                std::thread::Builder::new().name("vexus-housekeeping".into()).spawn(move || {
                    loop {
                        // Holding the job slot keeps this from deleting a batch that a clean is
                        // still filling or a restore is still reading; if something is running,
                        // the next round gets it.
                        let removed = match st.begin_job(JobKind::Purge) {
                            Ok((_, _guard)) => st.engine.recovery().purge_expired(now_unix()),
                            Err(_) => Vec::new(),
                        };
                        if !removed.is_empty() {
                            tracing::info!(
                                count = removed.len(),
                                "expired recovery batches removed"
                            );
                        }
                        for _ in 0..120 {
                            if st.is_quitting() {
                                return;
                            }
                            std::thread::sleep(Duration::from_secs(30));
                        }
                    }
                });
            Ok(())
        })
        .on_window_event(|win, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if win.label() != window::MAIN {
                    return;
                }
                let app = win.app_handle();
                let Some(state) = app.try_state::<AppState>() else { return };
                if let Some(w) = app.get_webview_window(window::MAIN) {
                    window::save_geometry(&w, &state);
                }
                if state.settings().close_to_tray && !state.is_quitting() {
                    api.prevent_close();
                    let _ = win.hide();
                } else {
                    state.cancel_job();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Vexus");
}

/// Creates the sandbox folder and fills it with sample data the first time it's used.
fn prepare_sandbox(dir: &Path) -> PathBuf {
    let _ = std::fs::create_dir_all(dir);
    let root = vexus_core::fsops::canonical_dir(dir).unwrap_or_else(|_| dir.to_path_buf());
    let is_empty = std::fs::read_dir(&root).map_or(true, |mut rd| rd.next().is_none());
    if is_empty {
        match vexus_core::sandbox::populate(&Env::sandbox(&root)) {
            Ok(s) => eprintln!("Created sandbox profile ({} files) in {}", s.files, root.display()),
            Err(e) => eprintln!("Couldn't populate the sandbox: {e}"),
        }
    }
    root
}
