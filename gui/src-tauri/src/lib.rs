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

pub fn run() {
    forager_core::host::use_host_dirs();
    avoid_dmabuf_renderer();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::edit::ReleaseCache::default())
        .manage(commands::update::Updates::default())
        .setup(|app| {
            appearance::watch(app.handle().clone());
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
            commands::edit::create_menu_entries,
            commands::edit::skip_release,
            commands::edit::unskip_release,
            commands::system::restart_self,
            commands::update::plan_update,
            commands::update::start_update,
            commands::update::finish_update,
            commands::update::cancel_update,
            commands::settings::get_gui_settings,
            commands::settings::set_gui_settings,
            commands::settings::get_timer,
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
