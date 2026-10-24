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
        .run(tauri::generate_context!())
        .expect("error while running win-pydl");
}
