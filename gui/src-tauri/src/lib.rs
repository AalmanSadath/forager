//! Forager's desktop app: a Tauri shell around `forager-core`, with a React
//! frontend in the style of BifroSSH.

mod appearance;
mod commands;
mod state;

pub fn run() {
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
            commands::update::plan_update,
            commands::update::start_update,
            commands::update::finish_update,
            commands::update::cancel_update,
            commands::system::system_appearance,
            commands::system::start_request,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Forager");
}
