mod commands;
mod state;

use tauri::Manager;

use crate::state::AppState;

pub fn run() {
    tauri::Builder::default()
        // Must be registered first: a second launch hands over and exits before anything else runs.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            tracing::info!(?args, "second instance launched; focusing main window");
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            let state = tauri::async_runtime::block_on(AppState::init())?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::app_info])
        .build(tauri::generate_context!())
        .expect("error while building rusty-downloader")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event
                && let Some(state) = app.try_state::<AppState>()
            {
                // Stop transfers cleanly so they pick up where they left off next launch.
                tauri::async_runtime::block_on(state.downloads.shutdown());
            }
        });
}
