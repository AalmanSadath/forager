//! Installs Flatpak bundles into the user installation, so no root is needed,
//! unless the app is already in the system installation: then it is updated
//! there, with polkit asking for authorisation. A runtime the app needs is
//! fetched from the repository the bundle names (usually Flathub) if it is
//! not installed yet.

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;

use regex::bytes::Regex;
use serde::Serialize;

use crate::error::{Error, Result};
use crate::host;

/// The bundle header is a GVariant that stores the ref (`app/<id>/<arch>/<branch>`)
/// as plain text near the start of the file.
static APP_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"app/([A-Za-z0-9_.-]+)/[A-Za-z0-9_]+/[A-Za-z0-9_.-]+").unwrap());

/// Application id of the bundle, e.g. `org.example.Tool`.
pub fn app_id(file: &Path) -> Result<String> {
    let mut header = Vec::new();
    File::open(file)?.take(1 << 20).read_to_end(&mut header)?;
    find_app_id(&header).ok_or_else(|| Error::Archive("not a Flatpak app bundle".into()))
}

fn find_app_id(header: &[u8]) -> Option<String> {
    let captures = APP_REF.captures(header)?;
    Some(String::from_utf8_lossy(&captures[1]).into_owned())
}

/// `--reinstall` lets the same command install, upgrade, or replace a copy
/// installed from somewhere else. App data in `~/.var/app` is kept.
pub fn install(file: &Path) -> Result<()> {
    let id = app_id(file)?;
    let inst = installation(&id);
    run(host::command("flatpak")
        .args(["install", inst, no_questions(inst), "--reinstall", "--bundle"])
        .arg(file))
}

/// The flag that keeps flatpak from asking questions on the terminal. In
/// the system installation, `--noninteractive` would also stop polkit from
/// asking for the password that adding a repository (for a bundle that
/// names one) needs, so the answers are given with `--assumeyes` instead.
fn no_questions(installation: &str) -> &'static str {
    match installation {
        "--system" => "--assumeyes",
        _ => "--noninteractive",
    }
}

/// Runs a flatpak command; when it fails, the error says why, from what
/// flatpak printed, not only its exit status.
fn run(command: &mut std::process::Command) -> Result<()> {
    let out = command.stderr(std::process::Stdio::piped()).output()?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Still shown on a terminal, as without capturing.
    eprint!("{stderr}");
    match out.status.success() {
        true => Ok(()),
        false => Err(Error::CommandFailed(match reason(&stderr) {
            Some(reason) => format!("flatpak: {reason}"),
            None => format!("{command:?} exited with {}", out.status),
        })),
    }
}

/// The error flatpak gave, from its output: the last `error:` line, else
/// the last line.
fn reason(stderr: &str) -> Option<String> {
    let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    lines
        .iter()
        .rev()
        .find_map(|l| l.strip_prefix("error:"))
        .or(lines.last().copied())
        .map(|l| l.trim().to_string())
}

/// Installs the app a `.flatpakref` names from the repository it points
/// to. Flatpak adds that repository as a remote, verified with the ref's
/// key, so later updates come from it. Gives the app's id.
pub fn install_ref(file: &Path) -> Result<String> {
    let id = ref_app_id(&std::fs::read_to_string(file)?)
        .ok_or_else(|| Error::Archive("not a Flatpak app reference".into()))?;
    let inst = installation(&id);
    run(host::command("flatpak")
        .args(["install", inst, no_questions(inst), "--from"])
        .arg(file))?;
    Ok(id)
}

/// Adds the repository a `.flatpakrepo` describes, unless one of that name
/// exists, and installs `id` from it in place of the copy from a bundle,
/// so later updates come from the repository.
pub fn install_from_repo_file(file: &Path, id: &str) -> Result<()> {
    let inst = installation(id);
    let url = repo_url(&std::fs::read_to_string(file)?)
        .ok_or_else(|| Error::Archive("not a Flatpak repository file".into()))?;
    // A remote of that name for another address is someone else's.
    let base = remote_name(file);
    let name = match remotes(inst).into_iter().find(|(n, _)| *n == base) {
        Some((_, other)) if other.trim_end_matches('/') != url.trim_end_matches('/') => format!("{base}-forager"),
        _ => base,
    };
    run(host::command("flatpak")
        .args(["remote-add", inst, "--if-not-exists", "--from", &name])
        .arg(file))?;
    run(host::command("flatpak").args(["install", inst, no_questions(inst), "--reinstall", &name, id]))
}

/// Updates `id` from the repository it was installed from.
pub fn update(id: &str) -> Result<()> {
    let inst = installation(id);
    run(host::command("flatpak").args(["update", inst, no_questions(inst), id]))
}

/// Moves `id` to `commit` of the repository it came from, older or newer
/// than the one installed. Flatpak's system helper only follows the
/// newest commit, so in the system installation this runs as root through
/// `pkexec`, which asks for the password.
pub fn update_to(id: &str, commit: &str) -> Result<()> {
    let inst = installation(id);
    let commit = format!("--commit={commit}");
    let mut command = match inst {
        "--system" => super::pkexec(["flatpak", "update", inst, "--assumeyes"]),
        _ => {
            let mut command = host::command("flatpak");
            command.args(["update", inst, no_questions(inst)]);
            command
        }
    };
    run(command.arg(commit).arg(id))
}

/// The commits of `id` that `remote` still has, newest first, as
/// `(commit, subject)`. Projects usually name the version in the subject.
pub fn history(remote: &Remote, id: &str) -> Vec<(String, String)> {
    host::query("flatpak")
        .args(["remote-info", remote.installation(), "--log", &remote.name, id])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| parse_log(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or_default()
}

/// The version of `id` that `remote` has now, as its appstream data
/// declares it; `None` when the app declares none or the repository cannot
/// be reached.
pub fn repo_version(remote: &Remote, id: &str) -> Option<String> {
    // `remote-info` reads the version from the appstream data cached for
    // the remote, which only this refreshes. Unprivileged even system-wide;
    // if it fails, the cached data still answers.
    host::query("flatpak")
        .args([
            "update",
            remote.installation(),
            "--appstream",
            "--noninteractive",
            &remote.name,
        ])
        .output()
        .ok();
    host::query("flatpak")
        .args(["remote-info", remote.installation(), &remote.name, id])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| info_version(&String::from_utf8_lossy(&out.stdout)))
}

/// The `Version:` of `flatpak remote-info` or `flatpak info` output.
fn info_version(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix("Version:"))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Pairs each `Commit:` of `flatpak remote-info --log` with the
/// `Subject:` after it.
fn parse_log(text: &str) -> Vec<(String, String)> {
    let mut commits = Vec::new();
    let mut commit: Option<String> = None;
    for line in text.lines().map(str::trim) {
        if let Some(value) = line.strip_prefix("Commit:") {
            commit = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("Subject:")
            && let Some(commit) = commit.take()
        {
            commits.push((commit, value.trim().to_string()));
        }
    }
    commits
}

/// A Flatpak repository, as configured in one installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Remote {
    pub name: String,
    pub url: String,
    /// In the system installation rather than the user's.
    pub system: bool,
}

impl Remote {
    fn installation(&self) -> &'static str {
        if self.system { "--system" } else { "--user" }
    }
}

/// The repository `id` was installed from, if it has an address to update
/// from. A bundle without one gets an origin with no address, so it is not
/// a repository.
pub fn origin(id: &str) -> Option<Remote> {
    let inst = installation(id);
    let out = host::query("flatpak")
        .args(["info", inst, "--show-origin", id])
        .output()
        .ok()
        .filter(|out| out.status.success())?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let url = remotes(inst).into_iter().find(|(n, _)| *n == name)?.1;
    (!url.is_empty()).then_some(Remote {
        name,
        url,
        system: inst == "--system",
    })
}

/// Repositories shared by many apps: never offered for removal.
const SHARED_REMOTES: [&str; 3] = ["flathub", "flathub-beta", "fedora"];

/// The repository of `id` when nothing else installed comes from it, so it
/// can go with the app. Work it out before uninstalling.
pub fn removable_remote(id: &str) -> Option<Remote> {
    let remote = origin(id)?;
    let out = host::query("flatpak")
        .args(["list", remote.installation(), "--columns=application,origin"])
        .output()
        .ok()?;
    let refs = parse_list(&String::from_utf8_lossy(&out.stdout));
    only_used_by(&remote.name, id, &refs).then_some(remote)
}

/// Removes `remote`. One already gone (Flatpak drops a bundle's origin with
/// its app) is fine.
pub fn remove_remote(remote: &Remote) -> Result<()> {
    if !remotes(remote.installation()).iter().any(|(n, _)| *n == remote.name) {
        return Ok(());
    }
    run(host::command("flatpak").args(["remote-delete", remote.installation(), &remote.name]))
}

/// Whether only `id` (and its own extensions, such as its `.Locale`) comes
/// from `remote` among `refs` (`(ref id, origin)`), and the remote is not
/// a shared one.
fn only_used_by(remote: &str, id: &str, refs: &[(String, String)]) -> bool {
    let own = |r: &str| r == id || r.strip_prefix(id).is_some_and(|rest| rest.starts_with('.'));
    !SHARED_REMOTES.contains(&remote) && refs.iter().all(|(r, origin)| origin != remote || own(r))
}

/// `(name, url)` of the remotes in `installation`.
fn remotes(installation: &str) -> Vec<(String, String)> {
    host::query("flatpak")
        .args(["remotes", installation, "--columns=name,url"])
        .output()
        .map(|out| parse_list(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or_default()
}

/// The app a `.flatpakref` names.
fn ref_app_id(text: &str) -> Option<String> {
    let mut in_group = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_group = line == "[Flatpak Ref]";
        } else if in_group && let Some(value) = line.strip_prefix("Name=") {
            let value = value.trim();
            return (!value.is_empty()).then(|| value.to_string());
        }
    }
    None
}

/// The address a `.flatpakrepo` gives.
fn repo_url(text: &str) -> Option<String> {
    let mut in_group = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_group = line == "[Flatpak Repo]";
        } else if in_group && let Some(value) = line.strip_prefix("Url=") {
            return Some(value.trim().to_string()).filter(|v| !v.is_empty());
        }
    }
    None
}

/// A remote name from a `.flatpakrepo` file's name, without its version:
/// `Tool-1.2.0.flatpakrepo` becomes `tool`.
fn remote_name(file: &Path) -> String {
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let version = |rest: &str| rest.trim_start_matches('v').starts_with(|c: char| c.is_ascii_digit());
    let stem = stem
        .char_indices()
        .find(|&(i, c)| (c == '-' || c == '_') && version(&stem[i + 1..]))
        .map_or(stem.as_str(), |(i, _)| &stem[..i]);
    let name: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    match name.trim_matches('-') {
        "" => "repo".to_string(),
        name => name.to_string(),
    }
}

pub fn uninstall(id: &str) -> Result<()> {
    let inst = installation(id);
    run(host::command("flatpak").args(["uninstall", inst, no_questions(inst), id]))
}

/// Whether `id` is installed, in the user or the system installation.
pub fn is_present(id: &str) -> bool {
    installed_in(id, "--user") || installed_in(id, "--system")
}

/// The installation holding `id`: the system one only if the app is there
/// and not in the user's, so an app installed system-wide is updated in
/// place instead of gaining a second copy.
fn installation(id: &str) -> &'static str {
    if !installed_in(id, "--user") && installed_in(id, "--system") {
        "--system"
    } else {
        "--user"
    }
}

fn installed_in(id: &str, installation: &str) -> bool {
    host::query("flatpak")
        .args(["info", installation, id])
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Installed apps of both installations, as `(id, version)`; the version is
/// empty when the app does not declare one.
pub fn installed_apps() -> Vec<(String, String)> {
    let Ok(out) = host::query("flatpak")
        .args(["list", "--app", "--columns=application,version"])
        .output()
    else {
        return Vec::new();
    };
    parse_list(&String::from_utf8_lossy(&out.stdout))
}

fn parse_list(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let mut columns = line.split('\t');
            let id = columns.next()?.trim();
            (!id.is_empty()).then(|| (id.to_string(), columns.next().unwrap_or("").trim().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_app_id_in_header() {
        let header = b"flatpak\0\0\x01\x02app/it.example.Tool/x86_64/stable\0metadata\0[Application]\nname=x";
        assert_eq!(find_app_id(header).as_deref(), Some("it.example.Tool"));
        assert_eq!(find_app_id(b"flatpak\0runtime/org.example.Platform/x86_64/1\0"), None);
    }

    #[test]
    fn reads_flatpakrefs() {
        let text = "[Flatpak Ref]\nTitle=Tool\nName=org.example.Tool\nBranch=stable\nUrl=https://example.org/repo/\n";
        assert_eq!(ref_app_id(text).as_deref(), Some("org.example.Tool"));
        assert_eq!(ref_app_id("[Other]\nName=x\n"), None);
        assert_eq!(remote_name(Path::new("/tmp/My Tool.flatpakrepo")), "my-tool");
        assert_eq!(remote_name(Path::new("/tmp/tool-v1.2.0.flatpakrepo")), "tool");
        let repo = "[Flatpak Repo]\nTitle=Tool\nUrl=https://example.org/repo/\nGPGKey=abc\n";
        assert_eq!(repo_url(repo).as_deref(), Some("https://example.org/repo/"));
        assert_eq!(repo_url("[Flatpak Ref]\nUrl=x\n"), None);
        assert_eq!(remote_name(Path::new("/tmp/.flatpakrepo")), "flatpakrepo");
    }

    #[test]
    fn offers_a_remote_only_its_app_uses() {
        let refs = |list: &[(&str, &str)]| -> Vec<(String, String)> {
            list.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
        };
        let own = refs(&[
            ("org.example.Tool", "tool-origin"),
            ("org.example.Tool.Locale", "tool-origin"),
            ("org.gnome.Platform", "flathub"),
        ]);
        assert!(only_used_by("tool-origin", "org.example.Tool", &own));
        let shared = refs(&[("org.example.Tool", "tool"), ("org.example.Toolbox", "tool")]);
        assert!(!only_used_by("tool", "org.example.Tool", &shared));
        assert!(!only_used_by("flathub", "org.gnome.Platform", &own));
    }

    #[test]
    fn reads_a_repositorys_history() {
        let log = "        Commit: aaa\n        Parent: bbb\n       Subject: Tool 1.1.0\n          Date: x\n       History: \n\n        Commit: bbb\n       Subject: Tool 1.0.0\n          Date: y\n";
        assert_eq!(
            parse_log(log),
            [
                ("aaa".to_string(), "Tool 1.1.0".to_string()),
                ("bbb".to_string(), "Tool 1.0.0".to_string())
            ]
        );
    }

    #[test]
    fn reads_the_repositorys_version() {
        let text = "\nForager Test - A test app\n\n          ID: org.example.Tool\n         Ref: app/org.example.Tool/x86_64/master\n     Version: 1.3.0\n     License: MIT\n";
        assert_eq!(info_version(text).as_deref(), Some("1.3.0"));
        assert_eq!(info_version("          ID: org.example.Tool\n     Version: \n"), None);
        assert_eq!(info_version("          ID: org.example.Tool\n"), None);
    }

    #[test]
    fn says_why_flatpak_failed() {
        let stderr =
            "Installing app/x\nF: warning\nerror: Flatpak system operation ConfigureRemote not allowed for user\n";
        assert_eq!(
            reason(stderr).as_deref(),
            Some("Flatpak system operation ConfigureRemote not allowed for user")
        );
        assert_eq!(reason("something broke\n\n").as_deref(), Some("something broke"));
        assert_eq!(reason(""), None);
        assert_eq!(no_questions("--system"), "--assumeyes");
        assert_eq!(no_questions("--user"), "--noninteractive");
    }

    #[test]
    fn parses_flatpak_list() {
        let apps = parse_list("org.example.Tool\t1.2.0\nio.github.o.Other\t\n\n");
        assert_eq!(
            apps,
            [
                ("org.example.Tool".into(), "1.2.0".into()),
                ("io.github.o.Other".into(), String::new())
            ]
        );
    }
}
