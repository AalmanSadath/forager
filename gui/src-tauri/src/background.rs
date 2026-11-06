//! Forager without a window: started at login, and kept running when its
//! window is closed. This only keeps the window ready to show again; the
//! scheduled checks stay the systemd timer's, which runs with Forager closed.

use std::collections::HashMap;
use std::path::PathBuf;

/// The argument the autostart entry passes: start without showing the window.
pub const BACKGROUND_ARG: &str = "--background";

/// The autostart entry's file name. Development builds use their own, so a
/// dev build never replaces the installed Forager's entry.
fn entry_name() -> String {
    let id = "io.github.aalmansadath.Forager";
    if cfg!(debug_assertions) {
        format!("{id}.Dev.desktop")
    } else {
        format!("{id}.desktop")
    }
}

/// `$XDG_CONFIG_HOME/autostart/<entry>`, where the desktop looks at login.
/// Inside a Flatpak, `use_host_dirs` points the config dir at the host's.
fn autostart_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("autostart").join(entry_name()))
}

/// Whether Forager starts at login. The entry itself is the record, so one
/// removed elsewhere, in GNOME Tweaks say, shows as off here too.
pub fn launch_at_login() -> bool {
    autostart_path().is_some_and(|p| p.exists())
}

pub fn set_launch_at_login(on: bool) -> Result<(), String> {
    if forager_core::host::in_flatpak() {
        return portal::request_autostart(on);
    }
    let path = autostart_path().ok_or("cannot find the config directory")?;
    if !on {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("cannot remove {}: {e}", path.display())),
            _ => Ok(()),
        };
    }
    let program = std::env::var("APPIMAGE")
        .ok()
        .or_else(|| forager_core::install::running_exe().map(|p| p.to_string_lossy().into_owned()))
        .ok_or("cannot tell where this copy of Forager is")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    std::fs::write(&path, autostart_entry(&program)).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// The autostart entry for the program at `program`.
fn autostart_entry(program: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Forager\n\
         Comment=Keeps Forager ready in the background\n\
         Exec={} {BACKGROUND_ARG}\n\
         Icon=io.github.aalmansadath.Forager\n\
         Terminal=false\n\
         NoDisplay=true\n\
         X-GNOME-Autostart-enabled=true\n",
        quote(program)
    )
}

/// A path as one `Exec` argument, as the desktop entry spec quotes it.
fn quote(program: &str) -> String {
    let mut quoted = String::from('"');
    for c in program.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

/// Whether the command line asks to start without a window.
pub fn starts_hidden(args: &[String]) -> bool {
    args.iter().any(|a| a == BACKGROUND_ARG)
}

/// A Flatpak cannot write an entry that would start it, so it asks the
/// Background portal, which writes or removes one for it.
mod portal {
    use super::*;
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedValue, Value};

    pub(super) fn request_autostart(on: bool) -> Result<(), String> {
        let fail = |e: zbus::Error| format!("the Background portal did not answer: {e}");
        let conn = Connection::session().map_err(fail)?;
        let token = format!("forager{}", std::process::id());
        let sender = conn
            .unique_name()
            .map(|n| n.trim_start_matches(':').replace('.', "_"))
            .unwrap_or_default();
        // Listened to before asking, so a quick answer is not missed.
        let request = Proxy::new(
            &conn,
            "org.freedesktop.portal.Desktop",
            format!("/org/freedesktop/portal/desktop/request/{sender}/{token}"),
            "org.freedesktop.portal.Request",
        )
        .map_err(fail)?;
        let mut responses = request.receive_signal("Response").map_err(fail)?;

        let background = Proxy::new(
            &conn,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Background",
        )
        .map_err(fail)?;
        let mut options: HashMap<&str, Value> = HashMap::new();
        options.insert("handle_token", Value::from(token.as_str()));
        options.insert("reason", Value::from("Start Forager when you log in"));
        options.insert("autostart", Value::from(on));
        options.insert("commandline", Value::from(vec!["forager-gui", BACKGROUND_ARG]));
        options.insert("dbus-activatable", Value::from(false));
        let _: zbus::zvariant::OwnedObjectPath = background.call("RequestBackground", &("", options)).map_err(fail)?;

        let message = responses.next().ok_or("the Background portal did not answer")?;
        let (code, results): (u32, HashMap<String, OwnedValue>) =
            message.body().deserialize().map_err(|e| e.to_string())?;
        let granted = results
            .get("autostart")
            .and_then(|v| bool::try_from(v).ok())
            .unwrap_or(false);
        match code {
            0 if granted == on => Ok(()),
            0 => Err("the desktop did not allow Forager to start at login".into()),
            _ => Err("starting at login was refused".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_an_entry_that_starts_hidden() {
        let entry = autostart_entry("/home/me/Apps/For ager $1.AppImage");
        assert!(entry.contains("Exec=\"/home/me/Apps/For ager \\$1.AppImage\" --background\n"));
        assert!(entry.contains("X-GNOME-Autostart-enabled=true\n"));
        assert!(entry.starts_with("[Desktop Entry]\n"));
    }

    #[test]
    fn only_the_background_argument_starts_hidden() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(starts_hidden(&args(&["--background"])));
        assert!(!starts_hidden(&args(&["open", "fd"])));
        assert!(!starts_hidden(&[]));
    }
}
