mod bridge;
mod clipboard;
mod commands;
mod events;
mod grabber_commands;
mod keep_awake;
mod native_host;
mod notify;
mod on_complete;
mod queue_commands;
mod settings_commands;
mod speed;
mod state;
mod tool_commands;
mod tray;

/// Passed by the Start with Windows entry: open in the tray, not on screen.
const MINIMIZED_ARG: &str = "--minimized";

use specta_typescript::Typescript;
use tauri::Manager;
use tauri_specta::{Builder, collect_commands, collect_events};

use crate::state::AppState;

/// Generated TypeScript bindings for every command and event.
const BINDINGS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/src/lib/bindings.ts");

fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::app_info,
            commands::list_packages,
            commands::default_download_dir,
            commands::add_links,
            commands::pause_download,
            commands::resume_download,
            commands::pause_all,
            commands::resume_all,
            commands::remove_download,
            commands::remove_package,
            commands::open_download,
            commands::reveal_download,
            commands::open_package_folder,
            grabber_commands::grabber_list,
            grabber_commands::grabber_add,
            grabber_commands::grabber_remove,
            grabber_commands::grabber_clear,
            grabber_commands::grabber_rename,
            grabber_commands::grabber_set_package,
            grabber_commands::grabber_set_folder,
            grabber_commands::grabber_recheck,
            grabber_commands::grabber_set_format,
            grabber_commands::grabber_set_gallery_options,
            grabber_commands::gallerydl_flags,
            grabber_commands::grabber_confirm,
            grabber_commands::get_clipboard_watch,
            grabber_commands::set_clipboard_watch,
            tool_commands::tools_list,
            tool_commands::tool_install,
            tool_commands::tool_check,
            tool_commands::tool_rollback,
            tool_commands::tool_set_channel,
            tool_commands::open_tools_folder,
            settings_commands::get_settings,
            settings_commands::system_downloads_dir,
            settings_commands::set_settings,
            settings_commands::history_list,
            settings_commands::history_remove,
            settings_commands::history_clear,
            settings_commands::history_open,
            settings_commands::move_package,
            speed::speed_limit_status,
            speed::set_speed_limit_on,
            commands::set_download_speed_limit,
            commands::set_checksum,
            queue_commands::list_queues,
            queue_commands::add_queue,
            queue_commands::update_queue,
            queue_commands::remove_queue,
            queue_commands::move_package_to_queue,
            on_complete::on_complete_pending,
            on_complete::cancel_on_complete,
        ])
        .events(collect_events![
            events::DownloadEvents,
            events::GrabberEvents,
            events::ToolEvents,
            clipboard::ClipboardOffer,
            speed::SpeedLimitStatus,
            on_complete::OnCompleteCountdown
        ])
        // Sizes and ids are far below 2^53, so plain numbers are safe in JS.
        .dangerously_cast_bigints_to_number()
}

fn typescript() -> Typescript {
    Typescript::default()
}

pub fn run() {
    let specta = specta_builder();
    #[cfg(debug_assertions)]
    if let Err(e) = specta.export(typescript(), BINDINGS_PATH) {
        eprintln!("could not export TypeScript bindings: {e}");
    }

    tauri::Builder::default()
        // Must be registered first: a second launch hands over and exits before anything else runs.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            tracing::info!(?args, "second instance launched; focusing main window");
            tray::show_main(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![MINIMIZED_ARG]),
        ))
        .on_window_event(|window, event| {
            // With close-to-tray on, the window hides and the app keeps
            // downloading; Quit is in the tray menu.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
                && window
                    .try_state::<AppState>()
                    .is_some_and(|s| s.settings().close_to_tray)
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(specta.invoke_handler())
        .setup(move |app| {
            specta.mount_events(app);
            let state = tauri::async_runtime::block_on(AppState::init())?;
            let download_events = state.downloads.subscribe();
            let grabber_events = state.grabber.subscribe();
            let tool_events = state.tools.subscribe();
            let tray_events = state.downloads.subscribe();
            let notify_events = state.downloads.subscribe();
            let finished_events = state.downloads.subscribe();
            let hidden =
                state.settings().start_minimized && std::env::args().any(|a| a == MINIMIZED_ARG);
            app.manage(state);
            app.manage(on_complete::Countdown::default());
            let handle = app.handle();
            tray::create(handle)?;
            notify::register(handle);
            native_host::register(handle);
            tauri::async_runtime::spawn(tray::track(handle.clone(), tray_events));
            tauri::async_runtime::spawn(notify::watch(handle.clone(), notify_events));
            // The window starts hidden (tauri.conf.json) so a start into the
            // tray doesn't flash it.
            if !hidden {
                tray::show_main(handle);
            }

            tauri::async_runtime::spawn(events::forward::<_, events::DownloadBatch>(
                handle.clone(),
                download_events,
            ));
            tauri::async_runtime::spawn(events::forward::<_, events::GrabberBatch>(
                handle.clone(),
                grabber_events,
            ));
            tauri::async_runtime::spawn(events::forward::<_, events::ToolBatch>(
                handle.clone(),
                tool_events,
            ));
            tauri::async_runtime::spawn(clipboard::watch(handle.clone()));
            bridge::start(handle);
            tauri::async_runtime::spawn(keep_awake::watch(handle.clone()));
            tauri::async_runtime::spawn(speed::follow_schedule(handle.clone()));
            tauri::async_runtime::spawn(on_complete::watch(handle.clone(), finished_events));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building rusty-downloader")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event
                && let Some(state) = app.try_state::<AppState>()
            {
                // Stop transfers cleanly so they pick up where they left off next launch.
                state.shutdown.cancel();
                tauri::async_runtime::block_on(state.downloads.shutdown());
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fails when a command or type changed but `bindings.ts` wasn't regenerated.
    /// Regenerate by starting the app in debug (`pnpm dev`) or with
    /// `cargo test -p rusty-downloader -- --ignored regenerate_bindings`.
    #[test]
    fn typescript_bindings_are_up_to_date() {
        let fresh_path =
            std::env::temp_dir().join(format!("rdl-bindings-{}.ts", std::process::id()));
        specta_builder()
            .export(typescript(), &fresh_path)
            .expect("export bindings");
        let fresh = std::fs::read_to_string(&fresh_path).unwrap();
        let _ = std::fs::remove_file(&fresh_path);
        let committed = std::fs::read_to_string(BINDINGS_PATH)
            .unwrap_or_default()
            .replace("\r\n", "\n");

        assert!(
            fresh.replace("\r\n", "\n") == committed,
            "ui/src/lib/bindings.ts is out of date; regenerate it (see this test's docs)"
        );
    }

    #[test]
    #[ignore = "writes ui/src/lib/bindings.ts"]
    fn regenerate_bindings() {
        specta_builder()
            .export(typescript(), BINDINGS_PATH)
            .expect("export bindings");
    }
}
