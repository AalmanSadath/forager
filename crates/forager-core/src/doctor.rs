//! `forager doctor`: what Forager needs from the system, checked one by one,
//! each with a fix when something is missing. Checks that only matter for
//! some apps (signatures, Flatpaks, tarballs) only run when such an app is
//! tracked.

use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::model::{App, Format, Signature};
use crate::platform::{self, PackageManager, Platform};
use crate::{conditions, host, http, install, store, timer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub level: Level,
    pub detail: String,
    /// What to do about a warning or failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

fn ok(name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        name,
        level: Level::Ok,
        detail: detail.into(),
        fix: None,
    }
}

fn warn(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Check {
    Check {
        name,
        level: Level::Warn,
        detail: detail.into(),
        fix: Some(fix.into()),
    }
}

fn fail(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Check {
    Check {
        name,
        level: Level::Fail,
        detail: detail.into(),
        fix: Some(fix.into()),
    }
}

/// Runs every check for the apps tracked in `config`. Reads the config
/// without taking its lock, so it never waits for another process.
pub fn run(config: &Path) -> Vec<Check> {
    let mut checks = Vec::new();
    let apps = match store::read_apps(config) {
        Ok(apps) => {
            let count = match apps.len() {
                1 => "1 app".to_string(),
                n => format!("{n} apps"),
            };
            checks.push(ok("config", format!("{} ({count})", config.display())));
            apps
        }
        Err(e) => {
            checks.push(fail("config", e.to_string(), "fix or move the file aside"));
            Vec::new()
        }
    };
    let platform = Platform::detect();
    let uses = |f: &dyn Fn(&App) -> bool| apps.iter().any(f);
    let has_format = |format: Format| uses(&|a| a.format == Some(format) || a.install_of(format).is_some());
    let root_formats =
        uses(&|a| a.format.is_some_and(Format::needs_root) || a.installs().any(|i| i.format.needs_root()));

    checks.push(package_manager(&platform));
    checks.push(system_packages(&platform, root_formats));
    if has_format(Format::Tarball) {
        checks.push(bin_on_path());
    }
    if has_format(Format::Flatpak) {
        checks.push(match platform.flatpak {
            true => ok("flatpak", "installed"),
            false => fail("flatpak", "not installed, but an app uses Flatpak", "install flatpak"),
        });
    }
    if uses(&|a| matches!(a.signature, Some(Signature::Gpg { .. }))) {
        checks.push(match host::has_command("gpgv") {
            true => ok("gpgv", "installed, for GPG signatures"),
            false => fail(
                "gpgv",
                "not installed, but an app's GPG signature needs it",
                "install gnupg",
            ),
        });
    }
    if uses(&|a| matches!(a.signature, Some(Signature::Attestation))) {
        checks.push(gh());
    }
    checks.push(github_token());
    checks.extend(background());
    if has_format(Format::Tarball) {
        checks.extend(zsh_completions());
    }
    checks
}

fn package_manager(platform: &Platform) -> Check {
    match platform.package_manager {
        Some(pm) => ok(
            "package manager",
            match pm {
                PackageManager::Dnf => "dnf (rpm)",
                PackageManager::Zypper => "zypper (rpm)",
                PackageManager::Pacman => "pacman (Arch packages)",
                PackageManager::Apt => "apt (deb)",
            },
        ),
        None => warn(
            "package manager",
            "none found; deb, rpm and Arch packages cannot be installed",
            "use AppImages, tarballs or Flatpaks (--format)",
        ),
    }
}

/// System packages install through PackageKit, or the native tool under
/// pkexec when PackageKit is missing.
fn system_packages(platform: &Platform, needed: bool) -> Check {
    if platform.package_manager.is_none() {
        return ok("PackageKit", "not needed without a package manager");
    }
    if install::packagekit::available() {
        return ok("PackageKit", "available; system packages ask for a password once");
    }
    match (host::has_command("pkexec"), needed) {
        (true, _) => ok("PackageKit", "missing; pkexec is used instead"),
        (false, true) => fail(
            "PackageKit",
            "neither PackageKit nor pkexec found; system packages cannot be installed",
            "install PackageKit or polkit",
        ),
        (false, false) => warn(
            "PackageKit",
            "neither PackageKit nor pkexec found",
            "install PackageKit or polkit before installing deb, rpm or Arch packages",
        ),
    }
}

fn bin_on_path() -> Check {
    let Some(bin) = platform::bin_dir() else {
        return fail("~/.local/bin", "no home directory", "set HOME");
    };
    let on_path = host::path().is_some_and(|paths| std::env::split_paths(&paths).any(|p| p == bin));
    match on_path {
        true => ok("~/.local/bin", "on PATH, so tarball apps' commands run"),
        false => warn(
            "~/.local/bin",
            format!(
                "{} is not on PATH; tarball apps' commands are linked there",
                bin.display()
            ),
            format!("add export PATH=\"{}:$PATH\" to your shell's profile", bin.display()),
        ),
    }
}

fn gh() -> Check {
    if !host::has_command("gh") {
        return fail(
            "gh",
            "not installed, but an app's attestation check needs it",
            "install the GitHub CLI",
        );
    }
    let authed = host::command("gh")
        .args(["auth", "status"])
        .output()
        .is_ok_and(|out| out.status.success());
    match authed {
        true => ok("gh", "installed and logged in, for attestations"),
        false => fail("gh", "not logged in; attestation checks fail", "run gh auth login"),
    }
}

/// Whether a GitHub token is set, and how much of the rate limit is left.
/// `/rate_limit` itself does not count against the limit.
fn github_token() -> Check {
    let token = http::token_for("github.com", Some(("github.com", "GITHUB_TOKEN")));
    let mut request = http::get("https://api.github.com/rate_limit");
    if let Some(token) = &token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let remaining = request
        .call()
        .ok()
        .and_then(|mut r| r.body_mut().read_json::<serde_json::Value>().ok())
        .and_then(|v| {
            let core = &v["resources"]["core"];
            Some((core["remaining"].as_u64()?, core["limit"].as_u64()?))
        });
    let left = remaining.map_or("rate limit unknown".to_string(), |(r, l)| {
        format!("{r} of {l} requests left this hour")
    });
    match token {
        Some(_) if remaining.is_some_and(|(_, limit)| limit <= 60) => fail(
            "GitHub token",
            format!("set but not accepted ({left})"),
            "make a new token; see the README",
        ),
        Some(_) => ok("GitHub token", format!("set; {left}")),
        None => warn(
            "GitHub token",
            format!("not set; {left}"),
            "put GITHUB_TOKEN=… in ~/.config/forager/env for 5000 requests an hour",
        ),
    }
}

/// The timer, the command it runs, and what its options rely on.
fn background() -> Vec<Check> {
    let next = match timer::next_run() {
        Ok(Some(next)) => next,
        Ok(None) => return vec![ok("timer", "off; forager timer enable checks daily")],
        Err(e) => return vec![warn("timer", e.to_string(), "check systemctl --user status")],
    };
    let mut checks = Vec::new();
    let unit = timer::unit_dir()
        .ok()
        .and_then(|dir| fs::read_to_string(dir.join(timer::SERVICE)).ok())
        .unwrap_or_default();
    match exec_program(&unit) {
        Some(program) if !program_exists(&program) => checks.push(fail(
            "timer",
            format!("runs {program}, which no longer exists"),
            "run forager timer enable again, or turn the daily check off and on in Settings → Background",
        )),
        _ => checks.push(ok("timer", format!("on; next run {next}"))),
    }
    let notifications = zbus::blocking::Connection::session().ok().is_some_and(|conn| {
        zbus::blocking::fdo::DBusProxy::new(&conn)
            .ok()
            .and_then(|dbus| {
                dbus.name_has_owner("org.freedesktop.Notifications".try_into().ok()?)
                    .ok()
            })
            .unwrap_or(false)
    });
    checks.push(match notifications {
        true => ok("notifications", "a notification service is running"),
        false => warn(
            "notifications",
            "no notification service on the session bus; the daily check cannot tell you about updates",
            "run a desktop or notification daemon",
        ),
    });
    let avoid = timer::auto_update_avoids();
    if avoid.metered && conditions::metered().is_none() {
        checks.push(warn(
            "NetworkManager",
            "not reachable; updates are not skipped on metered connections",
            "NetworkManager decides what is metered",
        ));
    }
    if avoid.battery && conditions::on_battery().is_none() {
        checks.push(warn(
            "UPower",
            "not reachable; updates are not skipped on battery",
            "install upower",
        ));
    }
    checks
}

/// The program the service's first `ExecStart` runs, unquoted.
fn exec_program(unit: &str) -> Option<String> {
    let line = unit.lines().find_map(|l| l.strip_prefix("ExecStart="))?;
    let line = line.trim_start_matches(['-', '@', '+', '!']);
    match line.strip_prefix('"') {
        Some(rest) => Some(rest.split('"').next()?.replace("\\\\", "\\").replace("%%", "%")),
        None => line.split_whitespace().next().map(str::to_string),
    }
}

fn program_exists(program: &str) -> bool {
    match program.starts_with('/') {
        true => host::command("test")
            .args(["-x", program])
            .status()
            .is_ok_and(|s| s.success()),
        false => host::has_command(program),
    }
}

/// zsh only finds completions on its `fpath`; tarball apps link theirs
/// into `~/.local/share/zsh/site-functions`.
fn zsh_completions() -> Option<Check> {
    let shell = std::env::var("SHELL").ok()?;
    if !shell.ends_with("/zsh") {
        return None;
    }
    let functions = platform::data_dir()?.join("zsh/site-functions");
    if fs::read_dir(&functions).map_or(true, |mut d| d.next().is_none()) {
        return None;
    }
    let zshrc = dirs::home_dir()?.join(".zshrc");
    let listed = fs::read_to_string(zshrc).is_ok_and(|text| text.contains("site-functions"));
    Some(match listed {
        true => ok("zsh completions", "~/.zshrc adds them to fpath"),
        false => warn(
            "zsh completions",
            "tarball apps' completions are not on zsh's fpath",
            "add fpath+=~/.local/share/zsh/site-functions to ~/.zshrc before compinit",
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_program_the_timer_runs() {
        assert_eq!(
            exec_program("[Service]\nExecStart=-/usr/bin/forager --from-timer update\nExecStart=/x\n").as_deref(),
            Some("/usr/bin/forager")
        );
        assert_eq!(
            exec_program("ExecStart=\"/home/me/My Apps/forager\" check").as_deref(),
            Some("/home/me/My Apps/forager")
        );
        assert_eq!(
            exec_program("ExecStart=flatpak run --command=forager io.x check").as_deref(),
            Some("flatpak")
        );
        assert_eq!(exec_program("[Unit]\n"), None);
    }

    #[test]
    fn failures_outrank_warnings() {
        assert!(Level::Fail > Level::Warn && Level::Warn > Level::Ok);
    }
}
