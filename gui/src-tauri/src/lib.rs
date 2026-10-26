//! Forager's desktop app: a Tauri shell around `forager-core`, with a React
//! frontend in the style of BifroSSH.

mod appearance;
mod commands;
mod state;

/// WebKitGTK's DMA-BUF renderer dies at startup on some GPU and compositor
/// pairs, NVIDIA's driver under Wayland among them, and the window never
/// appears. The fallback draws the same pixels, so it is used everywhere,
/// as the Flatpak already does, unless the variable was set by the user.
fn avoid_dmabuf_renderer() {
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // SAFETY: called at the start of `main`, before any thread starts.
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }
}

/// The D-Bus name that makes Forager single-instance. Development builds
/// use their own, so they never hand their arguments to an installed
/// Forager that is running.
fn instance_id() -> String {
    let id = "io.github.aalmansadath.Forager";
    if cfg!(debug_assertions) {
        format!("{id}.Dev")
    } else {
        id.to_string()
    }
}

/// Shows the open window and passes it what a second launch asked for.
fn forward_start(app: &tauri::AppHandle, args: &[String]) {
    use tauri::{Emitter, Manager};
    if let Some(window) = app.get_webview_window("main") {
        window.unminimize().ok();
        window.show().ok();
        window.set_focus().ok();
    }
    let args: Vec<String> = args.iter().skip(1).cloned().collect();
    if let Some(request) = commands::system::parse_start_args(&args) {
        app.emit(commands::system::START_EVENT, request).ok();
    }
}

/// Packages declare the forager:// scheme in their desktop file; an
/// AppImage has none installed, so it registers itself when run.
fn register_links(app: &tauri::App) {
    use tauri_plugin_deep_link::DeepLinkExt;
    if std::env::var_os("APPIMAGE").is_some() {
        app.deep_link().register_all().ok();
    }
}

pub fn run() {
    forager_core::host::use_host_dirs();
    avoid_dmabuf_renderer();
    tauri::Builder::default()
        // First, so a second launch (a clicked forager:// link, `forager-gui
        // add <url>`) hands its arguments to the open window and exits.
        .plugin(
            tauri_plugin_single_instance::Builder::new()
                .callback(|app, args, _cwd| forward_start(app, &args))
                .dbus_id(instance_id())
                .build(),
        )
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::edit::ReleaseCache::default())
        .manage(commands::update::Updates::default())
        .setup(|app| {
            appearance::watch(app.handle().clone());
            register_links(app);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::activity::list_history,
            commands::apps::list_apps,
            commands::check::check_apps,
            commands::edit::parse_source,
            commands::edit::preview,
            commands::edit::add_app,
            commands::edit::save_app,
            commands::edit::remove_app,
            commands::edit::list_releases,
            commands::edit::export_apps,
            commands::edit::inspect_appimage,
            commands::edit::import_apps,
            commands::edit::app_link,
            commands::edit::app_badge,
            commands::edit::parse_link,
            commands::edit::create_menu_entries,
            commands::edit::skip_release,
            commands::edit::unskip_release,
            commands::edit::mark_seen,
            commands::edit::follow_move,
            commands::system::restart_self,
            commands::update::plan_update,
            commands::update::start_update,
            commands::update::finish_update,
            commands::update::cancel_update,
            commands::settings::get_gui_settings,
            commands::settings::set_gui_settings,
            commands::settings::get_timer,
            commands::settings::run_doctor,
            commands::settings::set_timer,
            commands::settings::get_tokens,
            commands::settings::set_tokens,
            commands::settings::about_info,
            commands::system::system_appearance,
            commands::system::start_request,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Forager");
}
