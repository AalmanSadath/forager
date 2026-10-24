//! Running Forager inside a Flatpak.
//!
//! The sandbox has none of the tools Forager drives (dnf, rpm, pacman,
//! flatpak, tar, gpgv, systemctl), and the packages it installs belong to
//! the host anyway, so programs run on the host through `flatpak-spawn
//! --host`. Its data directories are pointed back at the host's, so the
//! app, the CLI and the background check share one config, history and set
//! of keys whether or not they run sandboxed. Outside a Flatpak nothing here
//! changes anything.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

static IN_FLATPAK: LazyLock<bool> = LazyLock::new(|| Path::new("/.flatpak-info").exists());

pub fn in_flatpak() -> bool {
    *IN_FLATPAK
}

/// A command for `program` that runs where the packages live: on the host.
pub fn command(program: &str) -> Command {
    command_for(program, in_flatpak())
}

fn command_for(program: &str, flatpak: bool) -> Command {
    if flatpak {
        let mut command = Command::new("flatpak-spawn");
        command.arg("--host").arg(program);
        command
    } else {
        Command::new(program)
    }
}

/// Whether `name` is installed on the host.
pub fn has_command(name: &str) -> bool {
    if in_flatpak() {
        // Names come from Forager itself, never from input, but stay safe to
        // put in a shell word regardless.
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) {
            return false;
        }
        return command("sh")
            .args(["-c", &format!("command -v {name}")])
            .output()
            .is_ok_and(|out| out.status.success());
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

/// Inside a Flatpak, points the XDG data directories at the host's own,
/// instead of the sandbox's `~/.var/app/<id>`.
///
/// Call it first thing in `main`, before any thread starts: it changes the
/// process environment.
pub fn use_host_dirs() {
    if !in_flatpak() {
        return;
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    for (var, dir) in host_dirs(&home) {
        // SAFETY: called at the start of `main`, while the process has one
        // thread, so nothing can read the environment concurrently.
        unsafe { std::env::set_var(var, dir) };
    }
}

fn host_dirs(home: &Path) -> [(&'static str, PathBuf); 4] {
    [
        ("XDG_CONFIG_HOME", home.join(".config")),
        ("XDG_DATA_HOME", home.join(".local/share")),
        ("XDG_CACHE_HOME", home.join(".cache")),
        ("XDG_STATE_HOME", home.join(".local/state")),
    ]
}

/// The command the background timer runs to reach the CLI: the binary
/// itself, or, from a Flatpak, `flatpak run` of the app's own CLI, since a
/// path inside the sandbox means nothing to the host's systemd.
pub fn cli_command(cli: &Path) -> Vec<String> {
    cli_command_for(cli, in_flatpak().then(flatpak_id))
}

fn cli_command_for(cli: &Path, flatpak_id: Option<String>) -> Vec<String> {
    match flatpak_id {
        Some(id) => vec!["flatpak".into(), "run".into(), "--command=forager".into(), id],
        None => vec![cli.to_string_lossy().into_owned()],
    }
}

/// The app id inside a Flatpak.
pub fn flatpak_id() -> String {
    std::env::var("FLATPAK_ID").unwrap_or_else(|_| "io.github.aalmansadath.Forager".into())
}

/// Name of the desktop entry notifications point at: the Flatpak's, or the
/// one the deb and rpm packages install.
pub fn desktop_entry() -> String {
    if in_flatpak() { flatpak_id() } else { "Forager".into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawns_on_the_host_only_inside_a_flatpak() {
        let host = command_for("rpm", true);
        assert_eq!(host.get_program(), "flatpak-spawn");
        assert_eq!(host.get_args().collect::<Vec<_>>(), ["--host", "rpm"]);
        let local = command_for("rpm", false);
        assert_eq!(local.get_program(), "rpm");
        assert_eq!(local.get_args().count(), 0);
    }

    #[test]
    fn points_data_dirs_at_the_host() {
        let dirs = host_dirs(Path::new("/home/me"));
        assert!(dirs.contains(&("XDG_CONFIG_HOME", PathBuf::from("/home/me/.config"))));
        assert!(dirs.contains(&("XDG_STATE_HOME", PathBuf::from("/home/me/.local/state"))));
    }

    #[test]
    fn timer_runs_the_flatpak_through_flatpak_run() {
        assert_eq!(
            cli_command_for(Path::new("/usr/bin/forager"), None),
            ["/usr/bin/forager"]
        );
        assert_eq!(
            cli_command_for(Path::new("/app/bin/forager"), Some("io.example.F".into())),
            ["flatpak", "run", "--command=forager", "io.example.F"]
        );
    }
}
