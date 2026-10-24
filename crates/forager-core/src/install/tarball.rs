//! Unpacks tarballs into `<opt>/<id>` and links their executables into a bin
//! directory (`~/.local/opt` and `~/.local/bin` in normal use).
//!
//! The new tree is unpacked next to the old one and swapped in by rename, so
//! a failed download or a bad archive leaves the installed version working.

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};

#[derive(Debug)]
pub struct Placed {
    pub dir: PathBuf,
    pub links: Vec<PathBuf>,
}

/// Installs `file` as app `id`. `bins` names the executables to link,
/// relative to the archive's top directory; when empty they are found
/// automatically. `old_links` are the links made by the previous install,
/// removed if this version no longer provides them.
pub fn install(
    file: &Path,
    id: &str,
    bins: &[String],
    opt_dir: &Path,
    bin_dir: &Path,
    old_links: &[PathBuf],
) -> Result<Placed> {
    fs::create_dir_all(opt_dir)?;
    fs::create_dir_all(bin_dir)?;
    let target = opt_dir.join(id);
    let staging = opt_dir.join(format!(".{id}.new"));
    remove_dir_if_exists(&staging)?;
    fs::create_dir(&staging)?;

    let prepared = unpack(file, &staging, id, bins).and_then(|(root, executables)| {
        let links = plan_links(&executables, &target, bin_dir, old_links)?;
        Ok((root, executables, links))
    });
    let (root, executables, links) = match prepared {
        Ok(prepared) => prepared,
        Err(e) => {
            remove_dir_if_exists(&staging).ok();
            return Err(e);
        }
    };

    let old = opt_dir.join(format!(".{id}.old"));
    remove_dir_if_exists(&old)?;
    let had_target = target.exists();
    if had_target {
        fs::rename(&target, &old)?;
    }
    if let Err(e) = fs::rename(&root, &target) {
        if had_target {
            fs::rename(&old, &target).ok();
        }
        remove_dir_if_exists(&staging).ok();
        return Err(e.into());
    }
    if root != staging {
        fs::remove_dir(&staging)?;
    }
    remove_dir_if_exists(&old)?;

    for stale in old_links.iter().filter(|l| !links.contains(l)) {
        remove_link(stale)?;
    }
    for (link, rel) in links.iter().zip(&executables) {
        remove_link(link)?;
        symlink(target.join(rel), link)?;
    }
    Ok(Placed { dir: target, links })
}

/// Removes the links (only those still pointing into `dir`) and `dir` itself.
pub fn uninstall(dir: &Path, links: &[PathBuf]) -> Result<()> {
    for link in links {
        if fs::read_link(link).is_ok_and(|to| to.starts_with(dir)) {
            remove_link(link)?;
        }
    }
    remove_dir_if_exists(dir)
}

/// Extracts the archive and returns its top directory and the executables
/// to link, relative to that directory.
fn unpack(file: &Path, staging: &Path, id: &str, bins: &[String]) -> Result<(PathBuf, Vec<PathBuf>)> {
    // GNU tar detects the compression itself and refuses members with `..`
    // or absolute paths.
    super::run(
        Command::new("tar")
            .arg("--extract")
            .arg("--no-same-owner")
            .arg("--file")
            .arg(file)
            .arg("--directory")
            .arg(staging),
    )?;
    let root = content_root(staging)?;
    let executables = find_executables(&root, id, bins)?;
    Ok((root, executables))
}

/// The archive's single top-level directory, or `staging` if it has several
/// entries at the top or the one directory is part of a layout like `bin/`.
fn content_root(staging: &Path) -> Result<PathBuf> {
    let entries: Vec<_> = fs::read_dir(staging)?.collect::<std::io::Result<_>>()?;
    match entries.as_slice() {
        [] => Err(Error::Archive("archive is empty".into())),
        [only] if only.file_type()?.is_dir() && !LAYOUT_DIRS.iter().any(|d| only.file_name() == *d) => Ok(only.path()),
        _ => Ok(staging.to_path_buf()),
    }
}

const LAYOUT_DIRS: [&str; 4] = ["bin", "lib", "share", "usr"];

/// Executables are looked for in the top directory and in `bin/`. With
/// several candidates, the one named after the app wins.
fn find_executables(root: &Path, id: &str, bins: &[String]) -> Result<Vec<PathBuf>> {
    if !bins.is_empty() {
        return bins
            .iter()
            .map(|bin| {
                let rel = PathBuf::from(bin);
                let inside = rel.components().all(|c| matches!(c, Component::Normal(_)));
                if inside && is_executable(&root.join(&rel)) {
                    Ok(rel)
                } else {
                    Err(Error::Archive(format!(
                        "{bin} is not an executable file in the archive"
                    )))
                }
            })
            .collect();
    }

    let mut candidates = Vec::new();
    for dir in [Path::new(""), Path::new("bin")] {
        let Ok(entries) = fs::read_dir(root.join(dir)) else {
            continue;
        };
        for entry in entries {
            let rel = dir.join(entry?.file_name());
            let name = rel.to_string_lossy();
            if is_executable(&root.join(&rel)) && !name.ends_with(".so") && !name.contains(".so.") {
                candidates.push(rel);
            }
        }
    }
    candidates.sort();

    match candidates.as_slice() {
        [] => Err(Error::Archive("no executable found in the archive".into())),
        [only] => Ok(vec![only.clone()]),
        many => many
            .iter()
            .find(|rel| {
                rel.file_name()
                    .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(id))
            })
            .map(|rel| vec![rel.clone()])
            .ok_or_else(|| {
                let names: Vec<_> = many.iter().map(|rel| rel.to_string_lossy()).collect();
                Error::Archive(format!("several executables in the archive: {}", names.join(", ")))
            }),
    }
}

/// Link paths for `executables`, refusing to replace anything in `bin_dir`
/// that Forager did not create for this app.
fn plan_links(executables: &[PathBuf], target: &Path, bin_dir: &Path, old_links: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut links = Vec::new();
    for rel in executables {
        let name = rel
            .file_name()
            .ok_or_else(|| Error::Archive(format!("bad path {}", rel.display())))?;
        let link = bin_dir.join(name);
        let ours = old_links.contains(&link) || fs::read_link(&link).is_ok_and(|to| to.starts_with(target));
        if !ours && fs::symlink_metadata(&link).is_ok() {
            return Err(Error::LinkConflict(link));
        }
        if links.contains(&link) {
            return Err(Error::Archive(format!(
                "two executables named {}",
                name.to_string_lossy()
            )));
        }
        links.push(link);
    }
    Ok(links)
}

/// A regular file (not a symlink, which could point outside the tree) with an
/// execute bit set.
fn is_executable(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Removes `link` if it is a symlink; leaves anything else alone.
fn remove_link(link: &Path) -> Result<()> {
    match fs::symlink_metadata(link) {
        Ok(m) if m.file_type().is_symlink() => Ok(fs::remove_file(link)?),
        _ => Ok(()),
    }
}

fn remove_dir_if_exists(dir: &Path) -> Result<()> {
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        tmp: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Fixture {
            Fixture {
                tmp: tempfile::tempdir().unwrap(),
            }
        }

        fn opt(&self) -> PathBuf {
            self.tmp.path().join("opt")
        }

        fn bin(&self) -> PathBuf {
            self.tmp.path().join("bin")
        }

        /// Builds `<name>.tar.gz` from `(path, executable)` entries.
        fn archive(&self, name: &str, files: &[(&str, bool)]) -> PathBuf {
            let src = self.tmp.path().join(format!("src-{name}"));
            for (path, executable) in files {
                let path = src.join(path);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, name).unwrap();
                let mode = if *executable { 0o755 } else { 0o644 };
                fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            }
            let file = self.tmp.path().join(format!("{name}.tar.gz"));
            let status = Command::new("tar")
                .arg("-czf")
                .arg(&file)
                .arg("-C")
                .arg(&src)
                .arg(".")
                .status()
                .unwrap();
            assert!(status.success());
            file
        }

        fn install(&self, file: &Path, bins: &[String], old_links: &[PathBuf]) -> Result<Placed> {
            install(file, "tool", bins, &self.opt(), &self.bin(), old_links)
        }
    }

    #[test]
    fn unwraps_top_directory_and_links_single_executable() {
        let f = Fixture::new();
        let file = f.archive("v1", &[("tool-1.0/tool", true), ("tool-1.0/README", false)]);
        let placed = f.install(&file, &[], &[]).unwrap();

        assert_eq!(placed.dir, f.opt().join("tool"));
        assert_eq!(placed.links, vec![f.bin().join("tool")]);
        assert_eq!(fs::read_to_string(f.bin().join("tool")).unwrap(), "v1");
        assert!(f.opt().join("tool/README").exists());
        assert!(!f.opt().join(".tool.new").exists());
    }

    #[test]
    fn prefers_executable_named_after_app() {
        let f = Fixture::new();
        let file = f.archive("v1", &[("tool", true), ("helper", true), ("bin/other", true)]);
        let placed = f.install(&file, &[], &[]).unwrap();
        assert_eq!(placed.links, vec![f.bin().join("tool")]);

        let file = f.archive("v2", &[("a", true), ("b", true)]);
        assert!(matches!(f.install(&file, &[], &[]), Err(Error::Archive(_))));
    }

    #[test]
    fn upgrade_replaces_tree_and_drops_stale_links() {
        let f = Fixture::new();
        let v1 = f.archive("v1", &[("bin/tool", true), ("bin/old", true)]);
        let bins = ["bin/tool".to_string(), "bin/old".to_string()];
        let first = f.install(&v1, &bins, &[]).unwrap();

        let v2 = f.archive("v2", &[("bin/tool", true)]);
        let second = f.install(&v2, &[], &first.links).unwrap();

        assert_eq!(second.links, vec![f.bin().join("tool")]);
        assert_eq!(fs::read_to_string(f.bin().join("tool")).unwrap(), "v2");
        assert!(fs::symlink_metadata(f.bin().join("old")).is_err());
        assert!(!f.opt().join("tool/bin/old").exists());
    }

    #[test]
    fn failed_install_keeps_previous_version() {
        let f = Fixture::new();
        let v1 = f.archive("v1", &[("tool", true)]);
        let first = f.install(&v1, &[], &[]).unwrap();

        let broken = f.tmp.path().join("broken.tar.gz");
        fs::write(&broken, "not a tarball").unwrap();
        assert!(f.install(&broken, &[], &first.links).is_err());
        assert_eq!(fs::read_to_string(f.bin().join("tool")).unwrap(), "v1");
        assert!(!f.opt().join(".tool.new").exists());
    }

    #[test]
    fn refuses_to_replace_foreign_file_in_bin() {
        let f = Fixture::new();
        fs::create_dir_all(f.bin()).unwrap();
        fs::write(f.bin().join("tool"), "mine").unwrap();
        let file = f.archive("v1", &[("tool", true)]);

        assert!(matches!(f.install(&file, &[], &[]), Err(Error::LinkConflict(_))));
        assert_eq!(fs::read_to_string(f.bin().join("tool")).unwrap(), "mine");
        assert!(!f.opt().join("tool").exists());
    }

    #[test]
    fn rejects_bins_outside_archive() {
        let f = Fixture::new();
        let file = f.archive("v1", &[("tool", true)]);
        let result = f.install(&file, &["../../bin/sh".to_string()], &[]);
        assert!(matches!(result, Err(Error::Archive(_))));
    }

    #[test]
    fn uninstall_leaves_links_it_does_not_own() {
        let f = Fixture::new();
        let file = f.archive("v1", &[("tool", true)]);
        let placed = f.install(&file, &[], &[]).unwrap();
        let foreign = f.bin().join("other");
        symlink("/usr/bin/true", &foreign).unwrap();

        uninstall(&placed.dir, &[placed.links[0].clone(), foreign.clone()]).unwrap();
        assert!(!placed.dir.exists());
        assert!(fs::symlink_metadata(&placed.links[0]).is_err());
        assert!(fs::symlink_metadata(&foreign).is_ok());
    }
}
