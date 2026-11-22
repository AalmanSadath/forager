//! Bringing an app's settings and data along when it switches formats.
//!
//! System packages, AppImages and tarballs all keep theirs in the XDG
//! folders (`~/.config`, `~/.local/share`, `~/.local/state`); a Flatpak
//! keeps them under `~/.var/app/<id>`. So only a switch into or out of a
//! Flatpak has anything to copy. Folders are copied, never moved: the old
//! ones stay as they were, and nothing is ever deleted. Caches are left
//! behind, since an app makes them again.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::{App, Format, Installed};
use crate::{cleanup, host};

/// Which XDG folder a copied entry lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Config,
    Data,
    State,
}

impl Kind {
    const ALL: [Kind; 3] = [Kind::Config, Kind::Data, Kind::State];

    /// The folder on the host.
    fn host_root(self) -> Option<PathBuf> {
        match self {
            Kind::Config => dirs::config_dir(),
            Kind::Data => dirs::data_dir(),
            Kind::State => dirs::state_dir(),
        }
    }

    /// Where a Flatpak's own copy of the folder is, under `~/.var/app/<id>`.
    fn in_flatpak(self) -> &'static str {
        match self {
            Kind::Config => "config",
            Kind::Data => "data",
            Kind::State => ".local/state",
        }
    }
}

/// One folder or file to copy from the old install's place to the new one's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Move {
    pub kind: Kind,
    pub name: String,
    pub from: PathBuf,
    pub to: PathBuf,
    /// Something is already at `to`: the user decides which to keep.
    pub conflict: bool,
    /// Size of `from`, links not followed.
    pub bytes: u64,
}

/// What to do with one move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    /// Copy it; for a move with no conflict.
    Copy,
    /// Leave what is at the destination, and copy nothing.
    KeepNew,
    /// Rename what is at the destination to a backup, then copy.
    UseOld,
    /// Copy over what is at the destination, keeping no backup of it: it
    /// is deleted once the copy has worked, and put back if it has not.
    Replace,
}

/// Names the runtime or the desktop keeps in a Flatpak's folders for every
/// app, never copied out of one: on the host they belong to the desktop.
const SANDBOX: [&str; 22] = [
    "baloofilerc",
    "breezerc",
    "enchant",
    "glib-2.0",
    "gtkrc",
    "gtkrc-2.0",
    "gvfs-metadata",
    "ibus",
    "kcminputrc",
    "kconf_updaterc",
    "kdedefaults",
    "kdeglobals",
    "kioslaverc",
    "kwinrc",
    "mimeapps.list",
    "pipewire",
    "qtproject.conf",
    "speech-dispatcher",
    "trolltech.conf",
    "user-dirs.locale",
    "webkitgtk",
    "wireplumber",
];

/// The names `app` keeps its folders under, lowercase. Call before the old
/// install is removed: the menu entries Forager wrote for it name it too.
pub fn names(app: &App) -> BTreeSet<String> {
    cleanup::names(app)
}

/// Where the folders are, for `moves_in`: the host's XDG folders by kind,
/// and `~/.var/app`.
struct Roots {
    host: Vec<(Kind, PathBuf)>,
    var_app: PathBuf,
}

impl Roots {
    fn current() -> Option<Roots> {
        Some(Roots {
            host: Kind::ALL.iter().filter_map(|k| Some((*k, k.host_root()?))).collect(),
            var_app: dirs::home_dir()?.join(".var/app"),
        })
    }
}

/// What to copy when `new` replaces `replaced`, for an app that keeps its
/// folders under `names` (taken before `replaced` was removed). Empty when
/// no switch crosses into or out of a Flatpak.
pub fn moves(names: &BTreeSet<String>, replaced: &[Installed], new: &Installed) -> Vec<Move> {
    match Roots::current() {
        Some(roots) => moves_in(&roots, names, replaced, new),
        None => Vec::new(),
    }
}

/// Whether switching from `old` to `new` has anything to copy.
pub fn crosses_flatpak(old: Format, new: Format) -> bool {
    (old == Format::Flatpak) != (new == Format::Flatpak)
}

fn moves_in(roots: &Roots, names: &BTreeSet<String>, replaced: &[Installed], new: &Installed) -> Vec<Move> {
    let flatpak_dir = |i: &Installed| {
        (i.format == Format::Flatpak)
            .then_some(i.package.as_deref())
            .flatten()
            .filter(|id| !id.is_empty() && !id.contains('/'))
            .map(|id| roots.var_app.join(id))
    };
    let mut moves = Vec::new();
    for old in replaced.iter().filter(|o| crosses_flatpak(o.format, new.format)) {
        match (flatpak_dir(old), flatpak_dir(new)) {
            // Into a Flatpak: the app's own folders in the host's.
            (None, Some(app_dir)) => {
                for (kind, root) in &roots.host {
                    for (name, from) in entries(root, |n| names.contains(&n.to_lowercase())) {
                        let to = app_dir.join(kind.in_flatpak()).join(&name);
                        moves.push(movement(*kind, name, from, to));
                    }
                }
            }
            // Out of a Flatpak: what it keeps, but for the runtime's own.
            (Some(app_dir), None) => {
                for (kind, root) in &roots.host {
                    let inside = app_dir.join(kind.in_flatpak());
                    for (name, from) in entries(&inside, |n| !sandbox_name(n)) {
                        let to = root.join(&name);
                        moves.push(movement(*kind, name, from, to));
                    }
                }
            }
            _ => {}
        }
    }
    moves.retain(|m| m.from != m.to);
    moves.sort_by(|a, b| a.to.cmp(&b.to));
    moves.dedup_by(|a, b| a.to == b.to);
    moves
}

fn sandbox_name(name: &str) -> bool {
    let name = name.to_lowercase();
    cleanup::SHARED.contains(&name.as_str()) || SANDBOX.contains(&name.as_str())
}

/// What a backup made by `Choice::UseOld` is called after.
const BACKUP: &str = ".forager-backup-";

/// Entries right inside `dir` whose name `keep` accepts, by name; never a
/// backup made by an earlier switch.
fn entries(dir: &Path, keep: impl Fn(&str) -> bool) -> Vec<(String, PathBuf)> {
    let Ok(read) = fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<(String, PathBuf)> = read
        .filter_map(|e| e.ok())
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
        .filter(|(name, _)| !name.contains(BACKUP) && keep(name))
        .collect();
    found.sort();
    found
}

fn movement(kind: Kind, name: String, from: PathBuf, to: PathBuf) -> Move {
    Move {
        kind,
        name,
        bytes: cleanup::size(&from),
        conflict: fs::symlink_metadata(&to).is_ok(),
        from,
        to,
    }
}

/// The choice made when nobody is asked: copy what has no conflict, keep
/// what is already there.
pub fn default_choice(movement: &Move) -> Choice {
    match movement.conflict {
        true => Choice::KeepNew,
        false => Choice::Copy,
    }
}

/// Carries out `moves`, each with the choice at the same place in
/// `choices`; a missing one is `default_choice`. Copies run on the host, so
/// a sandboxed Forager reaches every app's folders. Gives each move with
/// what became of it; `Ok(false)` is one left alone.
pub fn apply(moves: &[Move], choices: &[Choice]) -> Vec<(Move, Result<bool>)> {
    let today = crate::source::iso_from_unix(crate::update::now())[..10].replace('-', "");
    moves
        .iter()
        .enumerate()
        .map(|(n, m)| {
            let choice = choices.get(n).copied().unwrap_or_else(|| default_choice(m));
            (m.clone(), carry_out(m, choice, &today))
        })
        .collect()
}

fn carry_out(movement: &Move, choice: Choice, today: &str) -> Result<bool> {
    let exists = fs::symlink_metadata(&movement.to).is_ok();
    match choice {
        Choice::KeepNew => return Ok(false),
        // Something appeared since it was asked about: never overwritten.
        Choice::Copy if exists => {
            return Err(Error::CommandFailed(format!(
                "{} already exists; left as it is",
                movement.to.display()
            )));
        }
        Choice::Copy => {}
        Choice::UseOld | Choice::Replace if exists => {
            let aside = backup_path(&movement.to, today);
            move_aside(&movement.to, &aside)?;
            if choice == Choice::Replace {
                if let Err(e) = copy(movement) {
                    // Put back as it was: the partial copy goes, the original returns.
                    run(host::command("rm").arg("-rf").arg("--").arg(&movement.to)).ok();
                    run(host::command("mv").arg("-n").arg("-T").arg(&aside).arg(&movement.to)).ok();
                    return Err(e);
                }
                run(host::command("rm").arg("-rf").arg("--").arg(&aside))?;
                return Ok(true);
            }
        }
        Choice::UseOld | Choice::Replace => {}
    }
    copy(movement)?;
    Ok(true)
}

/// Renames `path` to the free name `aside`.
fn move_aside(path: &Path, aside: &Path) -> Result<()> {
    run(host::command("mv").arg("-n").arg("-T").arg(path).arg(aside))?;
    match fs::symlink_metadata(path).is_ok() {
        true => Err(Error::CommandFailed(format!("cannot move {} aside", path.display()))),
        false => Ok(()),
    }
}

fn copy(movement: &Move) -> Result<()> {
    if let Some(parent) = movement.to.parent() {
        run(host::command("mkdir").arg("-p").arg(parent))?;
    }
    run(host::command("cp")
        .arg("-a")
        .arg("-T")
        .arg(&movement.from)
        .arg(&movement.to))
}

/// A free name beside `path` for the backup of what is there.
fn backup_path(path: &Path, today: &str) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let base = path.with_file_name(format!("{name}{BACKUP}{today}"));
    let mut candidate = base.clone();
    let mut n = 2;
    while fs::symlink_metadata(&candidate).is_ok() {
        candidate = PathBuf::from(format!("{}-{n}", base.display()));
        n += 1;
    }
    candidate
}

fn run(command: &mut Command) -> Result<()> {
    let out = command.stdin(std::process::Stdio::null()).output()?;
    match out.status.success() {
        true => Ok(()),
        false => Err(Error::CommandFailed(format!(
            "{command:?}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(format: Format, package: Option<&str>) -> Installed {
        Installed {
            tag: "1".into(),
            format,
            asset: String::new(),
            package: package.map(str::to_string),
            path: None,
            links: Vec::new(),
            desktop: Vec::new(),
            extract_and_run: false,
        }
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn roots(dir: &Path) -> Roots {
        Roots {
            host: vec![
                (Kind::Config, dir.join("config")),
                (Kind::Data, dir.join("share")),
                (Kind::State, dir.join("state")),
            ],
            var_app: dir.join("var/app"),
        }
    }

    const ID: &str = "md.obsidian.Obsidian";

    #[test]
    fn copies_the_apps_folders_into_a_flatpak() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("config/Obsidian/settings.json"), "{}");
        write(&dir.path().join("config/other/x"), "x");
        write(&dir.path().join("share/obsidian/vaults"), "x");
        // Caches are not one of the roots, so never copied.
        write(&dir.path().join("cache/obsidian/blob"), "x");
        let names = BTreeSet::from(["obsidian".to_string()]);
        let moves = moves_in(
            &roots(dir.path()),
            &names,
            &[install(Format::Rpm, Some("obsidian"))],
            &install(Format::Flatpak, Some(ID)),
        );
        let app = dir.path().join("var/app").join(ID);
        let pairs: Vec<(&Path, &Path)> = moves.iter().map(|m| (m.from.as_path(), m.to.as_path())).collect();
        let (config, data) = (dir.path().join("config/Obsidian"), dir.path().join("share/obsidian"));
        assert_eq!(
            pairs,
            [
                (config.as_path(), app.join("config/Obsidian").as_path()),
                (data.as_path(), app.join("data/obsidian").as_path()),
            ]
        );
        assert_eq!(moves[0].kind, Kind::Config);
        assert_eq!(moves[0].bytes, 2);
        assert!(!moves[0].conflict);
    }

    #[test]
    fn copies_a_flatpaks_folders_out_but_not_the_runtimes() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("var/app").join(ID);
        write(&app.join("config/obsidian/settings.json"), "{}");
        write(&app.join("config/gtk-3.0/settings.ini"), "x");
        write(&app.join("config/kdeglobals"), "x");
        write(&app.join("config/obsidian.forager-backup-20260101/settings.json"), "{}");
        write(&app.join(".local/state/obsidian/log"), "x");
        write(&app.join("cache/obsidian/blob"), "x");
        // Already on the host: asked about rather than overwritten.
        write(&dir.path().join("state/obsidian/old"), "x");
        let moves = moves_in(
            &roots(dir.path()),
            &BTreeSet::new(),
            &[install(Format::Flatpak, Some(ID))],
            &install(Format::AppImage, None),
        );
        let found: Vec<(&str, Kind, bool)> = moves.iter().map(|m| (m.name.as_str(), m.kind, m.conflict)).collect();
        assert_eq!(
            found,
            [("obsidian", Kind::Config, false), ("obsidian", Kind::State, true)]
        );
        assert_eq!(moves[0].to, dir.path().join("config/obsidian"));
    }

    #[test]
    fn nothing_to_copy_between_formats_sharing_folders() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("config/obsidian/settings.json"), "{}");
        let names = BTreeSet::from(["obsidian".to_string()]);
        let moves = moves_in(
            &roots(dir.path()),
            &names,
            &[install(Format::AppImage, None)],
            &install(Format::Rpm, Some("obsidian")),
        );
        assert!(moves.is_empty());
        assert!(!crosses_flatpak(Format::AppImage, Format::Tarball));
        assert!(crosses_flatpak(Format::Flatpak, Format::Deb));
    }

    #[test]
    fn using_the_old_folder_keeps_the_new_one_as_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("config/obsidian/settings.json"), "old");
        let app = dir.path().join("var/app").join(ID);
        write(&app.join("config/obsidian/settings.json"), "new");
        write(&dir.path().join("share/obsidian/vault"), "old");
        let names = BTreeSet::from(["obsidian".to_string()]);
        let moves = moves_in(
            &roots(dir.path()),
            &names,
            &[install(Format::Deb, Some("obsidian"))],
            &install(Format::Flatpak, Some(ID)),
        );
        assert_eq!(moves.iter().map(|m| m.conflict).collect::<Vec<_>>(), [true, false]);

        // Keeping the new one changes nothing.
        let kept = apply(&moves, &[Choice::KeepNew, Choice::KeepNew]);
        assert!(kept.iter().all(|(_, r)| matches!(r, Ok(false))));
        assert_eq!(
            fs::read_to_string(app.join("config/obsidian/settings.json")).unwrap(),
            "new"
        );
        assert!(!app.join("data").exists());

        let done = apply(&moves, &[Choice::UseOld, Choice::Copy]);
        assert!(done.iter().all(|(_, r)| matches!(r, Ok(true))), "{done:?}");
        assert_eq!(
            fs::read_to_string(app.join("config/obsidian/settings.json")).unwrap(),
            "old"
        );
        assert_eq!(fs::read_to_string(app.join("data/obsidian/vault")).unwrap(), "old");
        let backups: Vec<String> = fs::read_dir(app.join("config"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("obsidian.forager-backup-"))
            .collect();
        assert_eq!(backups.len(), 1);
        let backup = app.join("config").join(&backups[0]).join("settings.json");
        assert_eq!(fs::read_to_string(backup).unwrap(), "new");
        // The originals stay.
        assert_eq!(
            fs::read_to_string(dir.path().join("config/obsidian/settings.json")).unwrap(),
            "old"
        );

        // Copying again over what is now there is refused, not overwritten.
        let again = apply(&moves[1..], &[Choice::Copy]);
        assert!(again[0].1.is_err());
    }

    #[test]
    fn replacing_keeps_no_backup_and_restores_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("config/obsidian/settings.json"), "old");
        let app = dir.path().join("var/app").join(ID);
        write(&app.join("config/obsidian/settings.json"), "new");
        let names = BTreeSet::from(["obsidian".to_string()]);
        let moves = moves_in(
            &roots(dir.path()),
            &names,
            &[install(Format::Deb, Some("obsidian"))],
            &install(Format::Flatpak, Some(ID)),
        );
        let done = apply(&moves, &[Choice::Replace]);
        assert!(matches!(done[0].1, Ok(true)), "{done:?}");
        assert_eq!(
            fs::read_to_string(app.join("config/obsidian/settings.json")).unwrap(),
            "old"
        );
        assert_eq!(fs::read_dir(app.join("config")).unwrap().count(), 1);

        // A copy that fails leaves the folder that was there.
        write(&app.join("config/obsidian/settings.json"), "new");
        let mut missing = moves[0].clone();
        missing.from = dir.path().join("gone");
        let failed = apply(&[missing], &[Choice::Replace]);
        assert!(failed[0].1.is_err());
        assert_eq!(
            fs::read_to_string(app.join("config/obsidian/settings.json")).unwrap(),
            "new"
        );
        assert_eq!(fs::read_dir(app.join("config")).unwrap().count(), 1);
    }
}
