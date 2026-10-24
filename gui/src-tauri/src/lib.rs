//! Forager's desktop app: a Tauri shell around `forager-core`, with a React
//! frontend in the style of BifroSSH.

mod appearance;
mod commands;
mod state;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            appearance::watch(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::apps::list_apps,
            commands::check::check_apps,
            commands::system::system_appearance,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Forager");
}
