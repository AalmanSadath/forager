//! Menu entries the user makes for apps nothing else puts in the menu: ones
//! installed by a vendor's own installer (MATLAB in `/usr/local/MATLAB`),
//! unpacked by hand, or built locally.
//!
//! Every entry in the user's applications folder can be listed, read and
//! edited here, except the `forager-<id>.desktop` ones Forager writes for
//! the apps it tracks: their app rewrites them on each update. An icon
//! chosen for an entry is copied into the user's icon theme as
//! `forager-entry-<id>`, so the entry keeps it when the app's folder moves,
//! and that copy goes with the entry.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::desktop::{self, field, first_word, quote, rest_after_first_word};
use crate::error::{Error, Result};
use crate::{host, icons, store};

/// The key Forager's own entries for tracked apps carry.
const OWNER_KEY: &str = "X-Forager-App";

/// Marks an entry made here.
const MADE_MARK: &str = "X-Forager-Entry-Made=1";

/// What a copied icon is called after, with the entry's id.
const ICON_PREFIX: &str = "forager-entry-";

/// hicolor size folders a copied icon may be in.
const ICON_SIZES: [&str; 9] = [
    "scalable", "512x512", "256x256", "192x192", "128x128", "96x96", "64x64", "48x48", "32x32",
];

/// One entry in the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// The file's name without `.desktop`: its desktop file id.
    pub id: String,
    pub file: PathBuf,
    pub name: String,
    pub exec: Option<String>,
    /// The entry's icon as a `data:` URL, when it can be found.
    pub icon_url: Option<String>,
    /// The tracked app whose entry this is; read-only here.
    pub owner: Option<String>,
    /// `NoDisplay=true` or `Hidden=true`: not shown in the menu.
    pub hidden: bool,
    /// The program `Exec` starts is not there any more.
    pub missing_program: bool,
    /// Made on the Desktop entries page or with `forager desktop-entry`.
    pub made_here: bool,
}

/// The fields the simple editor offers. Empty text removes the key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Fields {
    pub name: String,
    pub exec: String,
    pub icon: String,
    pub comment: String,
    pub categories: String,
    pub terminal: bool,
    pub wm_class: String,
    pub no_display: bool,
    /// The folder the app starts in (`Path`); the home folder when empty.
    pub path: String,
}

/// An icon to choose from: where it is, and how it looks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IconChoice {
    pub path: PathBuf,
    pub data_url: String,
}

/// What picking an app's location found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Detected {
    /// The program to start, the best of `programs` for a folder.
    pub program: Option<PathBuf>,
    /// Programs a folder holds, best first; just the program for a file.
    pub programs: Vec<PathBuf>,
    pub name: String,
    /// An id for a new entry, from the program's name.
    pub id: String,
    /// Arguments the program is known to need to start from the menu.
    pub args: String,
    /// Icons near the program, best first.
    pub icons: Vec<IconChoice>,
}

/// What checking an entry found: `errors` stop it being saved, `notes`
/// (from `desktop-file-validate`, when installed) are worth a look.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Validation {
    pub errors: Vec<String>,
    pub notes: Vec<String>,
}

/// The folders entries and copied icons live in.
struct Dirs {
    apps: PathBuf,
    icons: PathBuf,
}

impl Dirs {
    fn user() -> Result<Dirs> {
        Ok(Dirs {
            apps: desktop::applications_dir().ok_or(Error::NoHome)?,
            icons: desktop::icons_dir().ok_or(Error::NoHome)?,
        })
    }

    fn file(&self, id: &str) -> Result<PathBuf> {
        check_id(id)?;
        Ok(self.apps.join(format!("{id}.desktop")))
    }
}

/// Ids name a file in the applications folder and nothing else.
fn check_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    match ok {
        true => Ok(()),
        false => Err(Error::Entry(format!(
            "'{id}' cannot name an entry: use letters, digits, '.', '_' and '-'"
        ))),
    }
}

/// Every entry in the user's applications folder, by name.
pub fn list() -> Result<Vec<Summary>> {
    let dirs = Dirs::user()?;
    let Ok(read) = fs::read_dir(&dirs.apps) else {
        return Ok(Vec::new());
    };
    let mut entries: Vec<Summary> = read
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "desktop") && p.is_file())
        .filter_map(|file| {
            let text = fs::read_to_string(&file).ok()?;
            let id = file.file_stem()?.to_str()?.to_string();
            Some(summary(id, file, &text))
        })
        .collect();
    entries.sort_by_key(|e| (e.name.to_lowercase(), e.id.clone()));
    Ok(entries)
}

fn summary(id: String, file: PathBuf, text: &str) -> Summary {
    let exec = field(text, "Exec");
    let is_true = |key| field(text, key).is_some_and(|v| v == "true");
    Summary {
        name: field(text, "Name").unwrap_or_else(|| id.clone()),
        icon_url: field(text, "Icon").and_then(|i| icon_url(&i)),
        owner: field(text, OWNER_KEY),
        hidden: is_true("NoDisplay") || is_true("Hidden"),
        missing_program: exec
            .as_deref()
            .and_then(program_of)
            .is_some_and(|p| !program_exists(&p)),
        made_here: text.lines().any(|l| l.trim() == MADE_MARK),
        exec,
        id,
        file,
    }
}

/// The program an `Exec` value starts, past an `env VAR=value` prefix.
fn program_of(exec: &str) -> Option<String> {
    let program = first_word(exec)?;
    if program != "env" {
        return Some(program);
    }
    exec.split_whitespace()
        .skip(1)
        .find(|w| !w.contains('=') && !w.starts_with('-'))
        .map(|w| w.trim_matches('"').to_string())
}

/// Whether `program`, a path or a name on `PATH`, is on the host.
fn program_exists(program: &str) -> bool {
    if host::in_flatpak() {
        return host::command("sh")
            .args(["-c", r#"command -v -- "$1" >/dev/null"#, "sh", program])
            .status()
            .is_ok_and(|s| s.success());
    }
    if program.contains('/') {
        return Path::new(program).exists();
    }
    host::path().is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// An icon named in an entry, as a `data:` URL: a path, or a name looked
/// up in the hicolor theme and pixmaps.
fn icon_url(icon: &str) -> Option<String> {
    let path = match icon.starts_with('/') {
        true => PathBuf::from(icon),
        false => {
            let user = dirs::data_dir();
            let flatpak = user.as_ref().map(|d| d.join("flatpak/exports/share"));
            let mut roots: Vec<&Path> = user.iter().chain(flatpak.iter()).map(PathBuf::as_path).collect();
            roots.extend([
                Path::new("/usr/local/share"),
                Path::new("/usr/share"),
                Path::new("/var/lib/flatpak/exports/share"),
            ]);
            icons::first_existing(&icons::theme_candidates(&roots, icon))?
        }
    };
    preview(&path)
}

/// The image at `path` as a `data:` URL, when it is a PNG or SVG.
pub fn preview(path: &Path) -> Option<String> {
    icons::read(path).and_then(icons::Icon::new).map(|i| i.data_url())
}

/// The text of entry `id`.
pub fn read(id: &str) -> Result<String> {
    Ok(fs::read_to_string(Dirs::user()?.file(id)?)?)
}

/// The simple editor's fields, read from `text`.
pub fn fields(text: &str) -> Fields {
    let get = |key| field(text, key).unwrap_or_default();
    Fields {
        name: get("Name"),
        exec: get("Exec"),
        icon: get("Icon"),
        comment: get("Comment"),
        categories: get("Categories"),
        terminal: get("Terminal") == "true",
        wm_class: get("StartupWMClass"),
        no_display: get("NoDisplay") == "true",
        path: get("Path"),
    }
}

/// `text` with `fields` written into its `[Desktop Entry]` group; nothing
/// else in it changes.
pub fn with_fields(text: &str, fields: &Fields) -> String {
    let text_value = |v: &str| (!v.trim().is_empty()).then(|| v.trim().to_string());
    set_keys(
        text,
        &[
            ("Name", text_value(&fields.name)),
            ("Exec", text_value(&fields.exec)),
            ("Icon", text_value(&fields.icon)),
            ("Comment", text_value(&fields.comment)),
            ("Categories", text_value(&fields.categories)),
            ("Terminal", Some(fields.terminal.to_string())),
            ("StartupWMClass", text_value(&fields.wm_class)),
            ("NoDisplay", fields.no_display.then(|| "true".to_string())),
            ("Path", text_value(&fields.path)),
        ],
    )
}

/// `text` with each key in `keys` set in the `[Desktop Entry]` group, or
/// removed for `None`: in place where it is, else at the group's end.
/// Localised keys (`Name[de]`), other keys, groups and comments stay.
pub fn set_keys<'a>(text: &str, keys: &[(&'a str, Option<String>)]) -> String {
    let line_for = |key: &str, value: &str| format!("{key}={}", value.replace(['\n', '\r'], " "));
    let mut out: Vec<String> = Vec::new();
    let mut written: Vec<&'a str> = Vec::new();
    let mut in_main = false;
    let mut seen_main = false;
    let flush = |out: &mut Vec<String>, written: &mut Vec<&'a str>| {
        // Before the blank lines that end the group.
        let at = out.len() - out.iter().rev().take_while(|l| l.trim().is_empty()).count();
        let missing: Vec<String> = keys
            .iter()
            .filter(|(k, v)| v.is_some() && !written.contains(k))
            .map(|(k, v)| line_for(k, v.as_deref().unwrap_or_default()))
            .collect();
        written.extend(keys.iter().map(|(k, _)| *k));
        out.splice(at..at, missing);
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_main {
                flush(&mut out, &mut written);
            }
            in_main = trimmed == "[Desktop Entry]";
            seen_main |= in_main;
            out.push(line.to_string());
            continue;
        }
        let key = line.split_once('=').map(|(k, _)| k.trim());
        match key.and_then(|k| keys.iter().find(|(name, _)| *name == k)) {
            Some((key, value)) if in_main => {
                if let Some(value) = value
                    && !written.contains(key)
                {
                    out.push(line_for(key, value));
                }
                written.push(key);
            }
            _ => out.push(line.to_string()),
        }
    }
    if in_main {
        flush(&mut out, &mut written);
    }
    if !seen_main {
        let mut head = vec!["[Desktop Entry]".to_string(), "Type=Application".to_string()];
        head.extend(keys.iter().filter_map(|(k, v)| Some(line_for(k, v.as_deref()?))));
        if !out.is_empty() {
            head.push(String::new());
        }
        head.extend(out);
        out = head;
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// The `Exec` value that starts `program` with `args`.
pub fn exec_line(program: &Path, args: &str) -> String {
    match args.trim() {
        "" => quote(program),
        args => format!("{} {args}", quote(program)),
    }
}

/// `exec` starting `program` instead, with the arguments it had.
pub fn with_program(exec: &str, program: &Path) -> String {
    format!("{}{}", quote(program), rest_after_first_word(exec))
}

/// `exec` with its arguments replaced by `args`.
pub fn with_args(exec: &str, args: &str) -> String {
    let exec = exec.trim();
    let program = &exec[..exec.len() - rest_after_first_word(exec).len()];
    match args.trim() {
        "" => program.to_string(),
        args => format!("{program} {args}"),
    }
}

/// The arguments in an `Exec` value, after its program.
pub fn args_of(exec: &str) -> String {
    rest_after_first_word(exec).trim().to_string()
}

/// A new entry starting `program`.
pub fn template(program: &Path, name: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec={}\nTerminal=false\nCategories=Utility;\n{MADE_MARK}\n",
        name.replace(['\n', '\r'], " "),
        quote(program)
    )
}

/// What `text` needs before it can be saved, and what
/// `desktop-file-validate` says of it.
pub fn validate(text: &str) -> Validation {
    let errors = basic_errors(text);
    let notes = match host::has_command("desktop-file-validate") {
        true => desktop_file_validate(text).unwrap_or_default(),
        false => Vec::new(),
    };
    Validation { errors, notes }
}

fn desktop_file_validate(text: &str) -> Option<Vec<String>> {
    let dir = store::cache_dir();
    fs::create_dir_all(&dir).ok()?;
    let mut file = tempfile::Builder::new()
        .prefix("entry-check-")
        .suffix(".desktop")
        .tempfile_in(&dir)
        .ok()?;
    file.write_all(text.as_bytes()).ok()?;
    let path = file.path().to_path_buf();
    let out = host::query("desktop-file-validate").arg(&path).output().ok()?;
    let prefix = format!("{}: ", path.display());
    let lines = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    Some(
        lines
            .lines()
            .map(|l| l.strip_prefix(&prefix).unwrap_or(l).trim().to_string())
            .filter(|l| !l.is_empty())
            // Forager's own marker is meant to be there.
            .filter(|l| !l.contains("X-Forager"))
            .collect(),
    )
}

/// Saves `text` as entry `id`, with a copy of `icon_file` as its icon when
/// one is given. Fails for the entries of tracked apps and for text that
/// is not an entry.
pub fn save(id: &str, text: &str, icon_file: Option<&Path>) -> Result<PathBuf> {
    save_in(&Dirs::user()?, id, text, icon_file)
}

fn save_in(dirs: &Dirs, id: &str, text: &str, icon_file: Option<&Path>) -> Result<PathBuf> {
    let file = dirs.file(id)?;
    if let Ok(existing) = fs::read_to_string(&file)
        && field(&existing, OWNER_KEY).is_some()
    {
        return Err(owned(id));
    }
    let checked = basic_errors(text);
    if let Some(error) = checked.first() {
        return Err(Error::Entry(format!("cannot save {id}: {error}")));
    }
    let mut text = text.to_string();
    if let Some(icon_file) = icon_file {
        let data = icons::read(&host_path(icon_file))
            .ok_or_else(|| Error::Entry(format!("cannot read the icon {}", icon_file.display())))?;
        let icon = desktop::Icon::new(data)
            .ok_or_else(|| Error::Entry(format!("{} is not a PNG or SVG icon", icon_file.display())))?;
        remove_icons(dirs, id);
        let dir = dirs.icons.join(&icon.size).join("apps");
        fs::create_dir_all(&dir)?;
        let name = format!("{ICON_PREFIX}{id}");
        fs::write(
            dir.join(format!("{name}.{}", if icon.svg { "svg" } else { "png" })),
            &icon.data,
        )?;
        desktop::touch(&dirs.icons);
        text = set_keys(&text, &[("Icon", Some(name))]);
    }
    fs::create_dir_all(&dirs.apps)?;
    let mut temp = tempfile::NamedTempFile::new_in(&dirs.apps)?;
    temp.write_all(text.as_bytes())?;
    // Readable as entries usually are: a temporary file starts as the owner's only.
    use std::os::unix::fs::PermissionsExt;
    temp.as_file().set_permissions(fs::Permissions::from_mode(0o644))?;
    temp.persist(&file).map_err(|e| e.error)?;
    desktop::refresh(&dirs.apps);
    Ok(file)
}

fn basic_errors(text: &str) -> Vec<String> {
    let mut errors = Vec::new();
    if !text.lines().any(|l| l.trim() == "[Desktop Entry]") {
        errors.push("there is no [Desktop Entry] group".to_string());
    }
    for key in ["Name", "Exec"] {
        if field(text, key).is_none_or(|v| v.is_empty()) {
            errors.push(format!("{key} is missing"));
        }
    }
    if field(text, OWNER_KEY).is_some() {
        errors.push("it is the menu entry of an app Forager tracks".into());
    }
    errors
}

fn owned(id: &str) -> Error {
    Error::Entry(format!(
        "{id} is the menu entry of an app Forager tracks; change it from that app's page"
    ))
}

/// Deletes entry `id` and the icon copied for it, and nothing else.
pub fn remove(id: &str) -> Result<()> {
    remove_in(&Dirs::user()?, id)
}

fn remove_in(dirs: &Dirs, id: &str) -> Result<()> {
    let file = dirs.file(id)?;
    let text = fs::read_to_string(&file)?;
    if field(&text, OWNER_KEY).is_some() {
        return Err(owned(id));
    }
    fs::remove_file(&file)?;
    remove_icons(dirs, id);
    desktop::refresh(&dirs.apps);
    Ok(())
}

/// The icons copied for entry `id`.
fn remove_icons(dirs: &Dirs, id: &str) {
    for size in ICON_SIZES {
        for ext in ["png", "svg"] {
            fs::remove_file(
                dirs.icons
                    .join(size)
                    .join("apps")
                    .join(format!("{ICON_PREFIX}{id}.{ext}")),
            )
            .ok();
        }
    }
    desktop::touch(&dirs.icons);
}

/// Starts entry `id`, as the menu would.
pub fn launch(id: &str) -> Result<()> {
    let file = Dirs::user()?.file(id)?;
    let mut child = host::command_at_home("gio")
        .arg("launch")
        .arg(&file)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| Error::Entry(format!("cannot start {id}: {e}")))?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Works out the program, name and icons for an app at `path`: a program,
/// or the folder it is installed in.
pub fn detect(path: &Path) -> Result<Detected> {
    let path = host_path(path);
    let is_dir = host::command("test")
        .arg("-d")
        .arg(&path)
        .status()
        .is_ok_and(|s| s.success());
    let programs = match is_dir {
        true => rank_programs(&path, programs_in(&path)),
        false => vec![path.clone()],
    };
    let program = programs.first().cloned();
    let program_name = program
        .as_deref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = program_name.split('.').next().unwrap_or(&program_name).to_string();
    // Spelled as the folder it is installed in, when that is named after it.
    let mut name = path
        .ancestors()
        .filter_map(|a| a.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .find(|folder| folder.eq_ignore_ascii_case(&stem))
        .unwrap_or_else(|| capitalised(&stem));
    let mut icons = Vec::new();
    // An AppImage carries its own name and icon.
    if let Some(program) = &program
        && let Ok((own_name, icon)) = desktop::appimage_name_and_icon(program)
    {
        name = own_name.unwrap_or(name);
        if let Some(data) = icon
            && let Some(choice) = cached_icon(&stem, data)
        {
            icons.push(choice);
        }
    }
    if let Some(program) = &program {
        icons.extend(icons_near(program, &stem));
    }
    icons.dedup_by(|a, b| a.path == b.path);
    icons.truncate(8);
    Ok(Detected {
        args: known_args(&stem).to_string(),
        id: slug(&stem),
        program,
        programs,
        name,
        icons,
    })
}

/// Programs that start differently without a terminal unless told: MATLAB
/// opens no window started from the menu without `-desktop`, as its own
/// launchers pass.
const KNOWN_ARGS: [(&str, &str); 1] = [("matlab", "-desktop")];

/// The arguments the program called `stem` needs to start from the menu.
fn known_args(stem: &str) -> &'static str {
    KNOWN_ARGS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(stem))
        .map_or("", |(_, args)| args)
}

/// An id for a new entry from `name`.
pub fn slug(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches(['-', '.']).to_string();
    if slug.is_empty() { "app".into() } else { slug }
}

fn capitalised(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "App".into(),
    }
}

/// An icon from inside an AppImage, kept in the cache so it has a path to
/// show and copy.
fn cached_icon(stem: &str, data: Vec<u8>) -> Option<IconChoice> {
    let icon = desktop::Icon::new(data)?;
    let dir = store::cache_dir().join("entry-icons");
    fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{}.{}", slug(stem), if icon.svg { "svg" } else { "png" }));
    fs::write(&path, &icon.data).ok()?;
    Some(IconChoice {
        data_url: preview(&path)?,
        path,
    })
}

/// Executables right inside `dir` and its `bin/`.
fn programs_in(dir: &Path) -> Vec<PathBuf> {
    let out = host::query("find")
        .arg(dir)
        .arg(dir.join("bin"))
        .args(["-maxdepth", "1", "-type", "f", "-perm", "-u+x"])
        .stderr(std::process::Stdio::null())
        .output();
    let Ok(out) = out else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(PathBuf::from)
        .filter(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            !name.ends_with(".so") && !name.contains(".so.")
        })
        .collect()
}

/// `programs` in `dir`, best first: named after the folder or one of the
/// two above it (`matlab` in `MATLAB/R2024b`), in `bin/`, not an installer.
fn rank_programs(dir: &Path, mut programs: Vec<PathBuf>) -> Vec<PathBuf> {
    let folders: Vec<String> = dir
        .ancestors()
        .take(3)
        .filter_map(|a| a.file_name())
        .map(|n| n.to_string_lossy().to_lowercase())
        .collect();
    let rank = |p: &PathBuf| {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let stem = name.split('.').next().unwrap_or(&name).to_string();
        let tool = ["uninstall", "install", "setup", "update", "crash", "helper"]
            .iter()
            .any(|w| name.contains(w));
        let named = match () {
            _ if folders.contains(&stem) => 0,
            _ if folders.iter().any(|f| f.contains(&stem) || stem.contains(f.as_str())) => 1,
            _ => 2,
        };
        let in_bin = p.parent().and_then(|d| d.file_name()).is_some_and(|n| n == "bin");
        (tool, named, !in_bin, name.len(), name)
    };
    programs.sort_by_key(rank);
    programs.dedup();
    programs
}

/// Icons for `program`: the theme's of the same name, then images near it
/// named after it or looking like an icon or logo.
fn icons_near(program: &Path, stem: &str) -> Vec<IconChoice> {
    let mut found = Vec::new();
    let user = dirs::data_dir();
    let mut roots: Vec<&Path> = user.iter().map(PathBuf::as_path).collect();
    roots.extend([Path::new("/usr/local/share"), Path::new("/usr/share")]);
    if let Some(themed) = icons::first_existing(&icons::theme_candidates(&roots, &stem.to_lowercase())) {
        found.push(themed);
    }
    if let Some(root) = search_root(program) {
        let out = host::query("find")
            .arg(&root)
            .args([
                "-maxdepth",
                "5",
                "-type",
                "f",
                "(",
                "-iname",
                "*.png",
                "-o",
                "-iname",
                "*.svg",
                ")",
                "-size",
                "-512k",
            ])
            .stderr(std::process::Stdio::null())
            .output();
        if let Ok(out) = out {
            let files: Vec<PathBuf> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .take(2000)
                .map(PathBuf::from)
                .collect();
            found.extend(rank_icons(files, stem, &|p| icons::read(p)));
        }
    }
    found
        .into_iter()
        .filter_map(|path| {
            Some(IconChoice {
                data_url: preview(&path)?,
                path,
            })
        })
        .collect()
}

/// Where to look for an app's icons: the folder above `bin/`, or the
/// program's own; never a folder shared by everything, like `/usr`.
fn search_root(program: &Path) -> Option<PathBuf> {
    let dir = program.parent()?;
    let root = match dir.file_name().is_some_and(|n| n == "bin") {
        true => dir.parent()?,
        false => dir,
    };
    let home = dirs::home_dir();
    let shared = [
        Path::new("/"),
        Path::new("/usr"),
        Path::new("/usr/local"),
        Path::new("/opt"),
    ];
    let too_wide = shared.contains(&root)
        || home.as_deref() == Some(root)
        || home
            .as_deref()
            .is_some_and(|h| root == h.join(".local") || root == h.join("Applications"))
        || root.components().count() < 3;
    (!too_wide).then(|| root.to_path_buf())
}

/// The likely icons among `files`, best first: named after the program and
/// an icon or logo, then after the program, then any icon or logo; bigger
/// square PNGs and SVGs before small ones. `read` gives a file's bytes.
fn rank_icons(files: Vec<PathBuf>, stem: &str, read: &dyn Fn(&Path) -> Option<Vec<u8>>) -> Vec<PathBuf> {
    let program = stem.to_lowercase();
    let mut ranked: Vec<(u8, i64, usize, PathBuf)> = files
        .into_iter()
        .filter_map(|path| {
            let name = path.file_stem()?.to_string_lossy().to_lowercase();
            let iconish = name.contains("icon") || name.contains("logo");
            let tier = match () {
                _ if !program.is_empty() && name.contains(&program) && iconish => 0,
                _ if !program.is_empty() && (name == program || name.starts_with(&program)) => 1,
                _ if !program.is_empty() && name.contains(&program) => 2,
                _ if iconish => 3,
                _ => return None,
            };
            Some((tier, 0, path.components().count(), path))
        })
        .collect();
    ranked.sort();
    ranked.truncate(40);
    for item in &mut ranked {
        item.1 = -image_size(read, &item.3);
    }
    ranked.retain(|item| item.1 < -15);
    ranked.sort();
    ranked.into_iter().map(|(_, _, _, path)| path).take(8).collect()
}

/// How big the image at `path` is: an SVG scales, a PNG's width when it is
/// square, else small. Not an image at all is 0.
fn image_size(read: &dyn Fn(&Path) -> Option<Vec<u8>>, path: &Path) -> i64 {
    let Some(data) = read(path) else { return 0 };
    if data.starts_with(b"\x89PNG\r\n\x1a\n") && data.len() >= 24 {
        let width = u32::from_be_bytes(data[16..20].try_into().unwrap_or_default());
        let height = u32::from_be_bytes(data[20..24].try_into().unwrap_or_default());
        return match width == height {
            true => i64::from(width.min(1024)),
            false => 16,
        };
    }
    match String::from_utf8_lossy(&data[..data.len().min(1024)]).contains("<svg") {
        true => 1024,
        false => 0,
    }
}

/// The real path of a file the document portal handed a sandboxed Forager
/// (`/run/user/<uid>/doc/<id>/<name>`): menu entries run on the host, so
/// they need the host's path. Any other path is given back as it is.
pub fn host_path(path: &Path) -> PathBuf {
    let mut parts = path.components().map(|c| c.as_os_str().to_string_lossy().into_owned());
    let prefix: Vec<String> = parts.by_ref().take(4).collect();
    if prefix.len() < 4 || prefix[1] != "run" || prefix[2] != "user" {
        return path.to_path_buf();
    }
    let rest: Vec<String> = parts.collect();
    let (Some("doc"), Some(doc_id)) = (rest.first().map(String::as_str), rest.get(1)) else {
        return path.to_path_buf();
    };
    match portal_host_path(doc_id) {
        // The document is the file or folder itself; anything past it lies inside.
        Some(host) => rest.iter().skip(3).fold(host, |p, part| p.join(part)),
        None => path.to_path_buf(),
    }
}

fn portal_host_path(doc_id: &str) -> Option<PathBuf> {
    let conn = zbus::blocking::Connection::session().ok()?;
    let proxy = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.portal.Documents",
        "/org/freedesktop/portal/documents",
        "org.freedesktop.portal.Documents",
    )
    .ok()?;
    let paths: HashMap<String, Vec<u8>> = proxy.call("GetHostPaths", &(vec![doc_id],)).ok()?;
    let mut bytes = paths.get(doc_id)?.clone();
    // A C string.
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY: &str = "[Desktop Entry]\nType=Application\nName=Tool\nName[de]=Werkzeug\nExec=tool %U\n# a comment\nX-Other=1\n\n[Desktop Action new]\nName=New window\nExec=tool --new\n";

    fn dirs(root: &Path) -> Dirs {
        Dirs {
            apps: root.join("applications"),
            icons: root.join("icons/hicolor"),
        }
    }

    fn png(size: u32) -> Vec<u8> {
        let mut data = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        data.extend(size.to_be_bytes());
        data.extend(size.to_be_bytes());
        data.extend([0; 16]);
        data
    }

    #[test]
    fn fields_change_only_their_own_lines() {
        let mut f = fields(ENTRY);
        assert_eq!(f.name, "Tool");
        assert_eq!(f.exec, "tool %U");
        assert!(!f.terminal);
        f.name = "Better tool".into();
        f.comment = "Does things".into();
        f.no_display = true;
        let text = with_fields(ENTRY, &f);
        assert_eq!(
            text,
            "[Desktop Entry]\nType=Application\nName=Better tool\nName[de]=Werkzeug\nExec=tool %U\n# a comment\nX-Other=1\nComment=Does things\nTerminal=false\nNoDisplay=true\n\n[Desktop Action new]\nName=New window\nExec=tool --new\n"
        );
        assert_eq!(fields(&text), f);
        // Emptied in the form: the key goes.
        f.comment.clear();
        f.no_display = false;
        let text = with_fields(&text, &f);
        assert!(!text.contains("Comment=") && !text.contains("NoDisplay"));
        assert_eq!(with_fields(&text, &fields(&text)), text);
    }

    #[test]
    fn keys_are_set_in_an_entry_without_a_main_group() {
        let text = set_keys("", &[("Name", Some("Tool".into())), ("Comment", None)]);
        assert_eq!(text, "[Desktop Entry]\nType=Application\nName=Tool\n");
    }

    #[test]
    fn new_entries_quote_their_program() {
        let text = template(Path::new("/opt/My App/$bin/run"), "My App");
        assert!(text.contains("Exec=\"/opt/My App/\\$bin/run\"\n"));
        assert!(text.contains("Name=My App\n") && text.contains(MADE_MARK));
        assert!(basic_errors(&text).is_empty());
        let exec = exec_line(Path::new("/opt/a b/run"), "--flag %F");
        assert_eq!(exec, "\"/opt/a b/run\" --flag %F");
        assert_eq!(args_of(&exec), "--flag %F");
        assert_eq!(with_program(&exec, Path::new("/x/y")), "\"/x/y\" --flag %F");
        assert_eq!(with_args(&exec, "-x"), "\"/opt/a b/run\" -x");
        assert_eq!(with_args(&exec, ""), "\"/opt/a b/run\"");
        assert_eq!(program_of("env A=1 \"/opt/a\" --x"), Some("/opt/a".into()));
    }

    #[test]
    fn ids_name_only_files_in_the_folder() {
        for bad in ["../x", "a/b", "", ".hidden", "a b"] {
            assert!(check_id(bad).is_err(), "{bad}");
        }
        assert!(check_id("org.example.Tool_2-x").is_ok());
        assert_eq!(slug("MATLAB R2024b"), "matlab-r2024b");
        assert_eq!(known_args("matlab"), "-desktop");
        assert_eq!(known_args("tool"), "");
    }

    #[test]
    fn saving_copies_the_icon_and_removing_takes_it_away() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        let icon = root.path().join("source.png");
        fs::write(&icon, png(256)).unwrap();
        let other = dirs.icons.join("256x256/apps/other.png");
        fs::create_dir_all(other.parent().unwrap()).unwrap();
        fs::write(&other, png(256)).unwrap();

        let file = save_in(
            &dirs,
            "tool",
            &template(Path::new("/opt/tool/tool"), "Tool"),
            Some(&icon),
        )
        .unwrap();
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("Icon=forager-entry-tool\n"));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o644);
        let copied = dirs.icons.join("256x256/apps/forager-entry-tool.png");
        assert!(copied.exists());

        assert!(save_in(&dirs, "tool", "Name=x\n", None).is_err());
        remove_in(&dirs, "tool").unwrap();
        assert!(!file.exists() && !copied.exists());
        assert!(other.exists() && icon.exists());
    }

    #[test]
    fn the_entries_of_tracked_apps_are_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        fs::create_dir_all(&dirs.apps).unwrap();
        let file = dirs.apps.join("forager-tool.desktop");
        fs::write(&file, "[Desktop Entry]\nName=Tool\nExec=tool\nX-Forager-App=tool\n").unwrap();
        assert!(save_in(&dirs, "forager-tool", "[Desktop Entry]\nName=T\nExec=t\n", None).is_err());
        assert!(remove_in(&dirs, "forager-tool").is_err());
        assert!(file.exists());
        let summary = summary(
            "forager-tool".into(),
            file,
            &fs::read_to_string(dirs.apps.join("forager-tool.desktop")).unwrap(),
        );
        assert_eq!(summary.owner.as_deref(), Some("tool"));
    }

    #[test]
    fn icons_named_after_the_program_come_first() {
        let images: HashMap<PathBuf, Vec<u8>> = [
            ("/opt/m/bin/glnxa64/cef_resources/matlab_icon.png", png(256)),
            ("/opt/m/toolbox/x/logo.png", png(512)),
            ("/opt/m/toolbox/x/matlab.png", png(64)),
            ("/opt/m/toolbox/x/tiny_icon.png", png(8)),
            ("/opt/m/toolbox/x/picture.png", png(512)),
        ]
        .into_iter()
        .map(|(p, d)| (PathBuf::from(p), d))
        .collect();
        let files: Vec<PathBuf> = images.keys().cloned().collect();
        let ranked = rank_icons(files, "matlab", &|p| images.get(p).cloned());
        let names: Vec<String> = ranked
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["matlab_icon.png", "matlab.png", "logo.png"]);
    }

    #[test]
    fn a_folders_program_is_the_one_named_after_it() {
        let dir = Path::new("/usr/local/MATLAB/R2024b");
        let programs = vec![
            dir.join("bin/mbuild"),
            dir.join("uninstall/bin/uninstall"),
            dir.join("bin/matlab"),
            dir.join("install"),
        ];
        let ranked = rank_programs(dir, programs);
        assert_eq!(ranked[0], dir.join("bin/matlab"));
        // Installers come last.
        assert_eq!(ranked[1], dir.join("bin/mbuild"));
        assert!(ranked[2..].iter().all(|p| p.to_string_lossy().contains("install")));
        assert_eq!(search_root(&dir.join("bin/matlab")).as_deref(), Some(dir));
        assert_eq!(search_root(Path::new("/usr/bin/tool")), None);
    }

    #[test]
    fn paths_outside_the_document_portal_stay() {
        assert_eq!(host_path(Path::new("/opt/x/run")), PathBuf::from("/opt/x/run"));
        assert_eq!(
            host_path(Path::new("/run/user/1000/bus")),
            PathBuf::from("/run/user/1000/bus")
        );
    }
}
