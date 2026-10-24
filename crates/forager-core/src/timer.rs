//! A systemd user timer that checks tracked apps once a day and, if asked,
//! installs updates that need no root.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::host;

pub const SERVICE: &str = "forager-check.service";
pub const TIMER: &str = "forager-check.timer";

/// What the timer's service runs.
#[derive(Debug, Clone)]
pub struct Settings {
    /// How to run the `forager` CLI: its path, or `flatpak run ...` from a
    /// Flatpak; see `host::cli_command`.
    pub command: Vec<String>,
    /// Config passed with `--config`; the default location when `None`.
    pub config: Option<PathBuf>,
    /// Also install updates that need no root (AppImage, tarball, Flatpak).
    pub auto_update: bool,
}

/// `$XDG_CONFIG_HOME/systemd/user`.
pub fn unit_dir() -> Result<PathBuf> {
    Ok(dirs::config_dir().ok_or(Error::NoHome)?.join("systemd").join("user"))
}

pub fn service_unit(settings: &Settings) -> String {
    let mut base = settings.command.clone();
    if let Some(config) = &settings.config {
        base.extend(["--config".to_string(), config.to_string_lossy().into_owned()]);
    }
    // Marks what the timer does as the timer's in the history.
    base.push("--from-timer".to_string());
    let command = |args: &[&str]| {
        base.iter()
            .map(String::as_str)
            .chain(args.iter().copied())
            .map(quote)
            .collect::<Vec<_>>()
            .join(" ")
    };

    let mut unit = String::from(
        "[Unit]\n\
         Description=Check apps tracked by Forager for updates\n\
         Wants=network-online.target\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         # Put GITHUB_TOKEN=... here to raise the GitHub API rate limit.\n\
         EnvironmentFile=-%h/.config/forager/env\n",
    );
    if settings.auto_update {
        // `-`: a failed update must not stop the check that follows.
        unit += &format!("ExecStart=-{}\n", command(&["update", "--yes", "--unprivileged-only"]));
    }
    unit += &format!("ExecStart={}\n", command(&["check", "--notify", "--quiet"]));
    unit
}

pub fn timer_unit() -> String {
    "[Unit]\n\
     Description=Check apps tracked by Forager for updates daily\n\
     \n\
     [Timer]\n\
     OnCalendar=daily\n\
     Persistent=true\n\
     RandomizedDelaySec=1h\n\
     \n\
     [Install]\n\
     WantedBy=timers.target\n"
        .to_string()
}

/// Writes both units into `dir`.
pub fn write_units(dir: &Path, settings: &Settings) -> Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(dir.join(SERVICE), service_unit(settings))?;
    fs::write(dir.join(TIMER), timer_unit())?;
    Ok(())
}

/// Installs the units and starts the timer.
pub fn enable(settings: &Settings) -> Result<()> {
    write_units(&unit_dir()?, settings)?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--now", TIMER])
}

/// Stops the timer and removes the units.
pub fn disable() -> Result<()> {
    let dir = unit_dir()?;
    if dir.join(TIMER).exists() {
        systemctl(&["disable", "--now", TIMER])?;
    }
    for unit in [TIMER, SERVICE] {
        match fs::remove_file(dir.join(unit)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    systemctl(&["daemon-reload"])
}

/// Whether the installed service also installs updates, read back from it.
pub fn auto_update_enabled() -> bool {
    unit_dir()
        .ok()
        .and_then(|dir| fs::read_to_string(dir.join(SERVICE)).ok())
        .is_some_and(|unit| unit.contains("--unprivileged-only"))
}

/// When the timer next fires, or `None` if it is not enabled.
pub fn next_run() -> Result<Option<String>> {
    if !unit_dir()?.join(TIMER).exists() {
        return Ok(None);
    }
    let out = host::command("systemctl")
        .args(["--user", "show", TIMER, "--property=NextElapseUSecRealtime", "--value"])
        .output()?;
    let next = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(Some(if next.is_empty() { "not scheduled".into() } else { next }))
}

fn systemctl(args: &[&str]) -> Result<()> {
    let status = host::command("systemctl").arg("--user").args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::CommandFailed(format!(
            "systemctl --user {} exited with {status}",
            args.join(" ")
        )))
    }
}

/// Quotes one ExecStart argument: systemd splits on spaces, unescapes `\`
/// and `"` inside double quotes, and expands `%` specifiers.
fn quote(arg: &str) -> String {
    let safe = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+=:,@".contains(c));
    if safe {
        return arg.to_string();
    }
    let escaped = arg.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(auto_update: bool) -> Settings {
        Settings {
            command: vec!["/usr/bin/forager".into()],
            config: Some("/home/me/My Apps/100%.toml".into()),
            auto_update,
        }
    }

    #[test]
    fn service_runs_check_with_quoted_config() {
        let unit = service_unit(&settings(false));
        assert!(unit.contains(
            "ExecStart=/usr/bin/forager --config \"/home/me/My Apps/100%%.toml\" --from-timer check --notify --quiet\n"
        ));
        assert!(!unit.contains("--unprivileged-only"));
    }

    #[test]
    fn auto_update_runs_before_check_and_may_fail() {
        let unit = service_unit(&settings(true));
        let update = unit.find("ExecStart=-/usr/bin/forager").unwrap();
        let check = unit.find("check --notify").unwrap();
        assert!(update < check);
        assert!(unit.contains("update --yes --unprivileged-only\n"));
    }

    #[test]
    fn writes_units() {
        let tmp = tempfile::tempdir().unwrap();
        write_units(tmp.path(), &settings(false)).unwrap();
        assert!(
            fs::read_to_string(tmp.path().join(TIMER))
                .unwrap()
                .contains("OnCalendar=daily")
        );
        assert!(tmp.path().join(SERVICE).exists());
    }
}
