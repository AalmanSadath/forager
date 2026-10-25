use crate::appearance::{self, SystemAppearance};
use forager_core::share::{self, AddRequest};

/// Desktop colour scheme and accent, read through the portal.
#[tauri::command]
pub async fn system_appearance() -> SystemAppearance {
    tauri::async_runtime::spawn_blocking(appearance::read)
        .await
        .unwrap_or_default()
}

/// What the app was asked to open from its command line:
/// `forager-gui add <url>`, `forager-gui open <id>`, or a `forager://add?…`
/// link, which brings the settings to start from.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StartRequest {
    Add { url: String, link: Option<AddRequest> },
    Open { id: String },
}

/// Event carrying a `StartRequest` from a second launch, e.g. a link
/// clicked while Forager is open.
pub const START_EVENT: &str = "start-request";

pub fn parse_start_args(args: &[String]) -> Option<StartRequest> {
    if let Some(link) = args.iter().find_map(|a| share::parse_link(a).ok()) {
        return Some(StartRequest::Add {
            url: link.url.clone(),
            link: Some(link),
        });
    }
    match args {
        [cmd, url, ..] if cmd == "add" => Some(StartRequest::Add {
            url: url.clone(),
            link: None,
        }),
        [cmd, id, ..] if cmd == "open" => Some(StartRequest::Open { id: id.clone() }),
        _ => None,
    }
}

#[tauri::command]
pub fn start_request() -> Option<StartRequest> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    parse_start_args(&args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_add_and_open_requests() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(matches!(
            parse_start_args(&args(&["add", "https://github.com/o/r"])),
            Some(StartRequest::Add { url, link: None }) if url == "https://github.com/o/r"
        ));
        assert!(matches!(
            parse_start_args(&args(&["forager://add?url=https://github.com/o/r&track_only=1"])),
            Some(StartRequest::Add { url, link: Some(link) }) if url == "https://github.com/o/r" && link.track_only
        ));
        assert!(
            matches!(parse_start_args(&args(&["open", "ripgrep"])), Some(StartRequest::Open { id }) if id == "ripgrep")
        );
        assert!(parse_start_args(&args(&["add"])).is_none());
        assert!(parse_start_args(&[]).is_none());
    }
}

/// Starts the newly installed Forager and closes this one: the AppImage
/// at `$APPIMAGE`, the Flatpak through the host's `flatpak run`, else the
/// binary this process was started from.
#[tauri::command]
pub fn restart_self(app: tauri::AppHandle) -> Result<(), String> {
    let (program, args) = relaunch_command(
        std::env::var("APPIMAGE").ok(),
        forager_core::host::in_flatpak().then(forager_core::host::flatpak_id),
        forager_core::install::running_exe().map(|p| p.to_string_lossy().into_owned()),
    )
    .ok_or("cannot tell how this copy of Forager was started")?;
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map_err(|e| format!("cannot start the new version: {e}"))?;
    app.exit(0);
    Ok(())
}

fn relaunch_command(
    appimage: Option<String>,
    flatpak_id: Option<String>,
    exe: Option<String>,
) -> Option<(String, Vec<String>)> {
    if let Some(appimage) = appimage {
        return Some((appimage, Vec::new()));
    }
    if let Some(id) = flatpak_id {
        let args = ["--host", "flatpak", "run", &id].map(String::from).to_vec();
        return Some(("flatpak-spawn".into(), args));
    }
    exe.map(|exe| (exe, Vec::new()))
}

#[cfg(test)]
mod relaunch_tests {
    use super::*;

    #[test]
    fn relaunches_the_way_it_was_installed() {
        let some = |s: &str| Some(s.to_string());
        assert_eq!(
            relaunch_command(
                some("/home/me/Applications/forager.AppImage"),
                None,
                some("/tmp/.mount/x")
            ),
            Some(("/home/me/Applications/forager.AppImage".into(), vec![]))
        );
        assert_eq!(
            relaunch_command(None, some("io.example.F"), some("/app/bin/forager-gui")).map(|c| c.1),
            Some(vec![
                "--host".into(),
                "flatpak".into(),
                "run".into(),
                "io.example.F".into()
            ])
        );
        assert_eq!(
            relaunch_command(None, None, some("/usr/bin/forager-gui")),
            Some(("/usr/bin/forager-gui".into(), vec![]))
        );
    }
}
