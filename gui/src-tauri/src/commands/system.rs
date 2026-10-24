use crate::appearance::{self, SystemAppearance};

/// Desktop colour scheme and accent, read through the portal.
#[tauri::command]
pub async fn system_appearance() -> SystemAppearance {
    tauri::async_runtime::spawn_blocking(appearance::read)
        .await
        .unwrap_or_default()
}

/// What the app was asked to open from its command line:
/// `forager-gui add <url>` or `forager-gui open <id>`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StartRequest {
    Add { url: String },
    Open { id: String },
}

pub fn parse_start_args(args: &[String]) -> Option<StartRequest> {
    match args {
        [cmd, url, ..] if cmd == "add" => Some(StartRequest::Add { url: url.clone() }),
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
            Some(StartRequest::Add { url }) if url == "https://github.com/o/r"
        ));
        assert!(
            matches!(parse_start_args(&args(&["open", "ripgrep"])), Some(StartRequest::Open { id }) if id == "ripgrep")
        );
        assert!(parse_start_args(&args(&["add"])).is_none());
        assert!(parse_start_args(&[]).is_none());
    }
}
