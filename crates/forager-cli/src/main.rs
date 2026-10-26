use std::io::{self, BufRead, IsTerminal, Write};
use std::ops::ControlFlow;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use forager_core::conditions::{self, Avoid};
use forager_core::history::{self, Event, Kind, Via};
use forager_core::matcher::select_asset;
use forager_core::model::{App, Format, Installed, Signature};
use forager_core::platform::{self, Platform};
use forager_core::signature::{self, SignatureInput};
use forager_core::source::{Forge, Source};
use forager_core::store::Store;
use forager_core::update::{self, InstallState, Plan, State};
use forager_core::{host, install, share, timer, version};

#[derive(Parser)]
#[command(
    name = "forager",
    version,
    about = "Install and update apps from their GitHub, GitLab and Codeberg releases"
)]
struct Cli {
    /// Path to the tracked apps file.
    #[arg(long, global = true, env = "FORAGER_CONFIG")]
    config: Option<PathBuf>,

    /// Set by the background timer so its actions show as its own in the history.
    #[arg(long, global = true, hide = true)]
    from_timer: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Track an app by its project URL, or by a `forager://add?…` link.
    Add {
        url: String,
        /// API the host speaks, for hosts other than github.com, codeberg.org
        /// and well-known GitLab instances.
        #[arg(long)]
        forge: Option<Forge>,
        /// For a web page (`--forge html`): a regex matching the addresses of
        /// the app's downloads. A `(?P<version>...)` group names the version.
        #[arg(long, value_name = "REGEX")]
        pattern: Option<String>,
        /// Id to refer to the app by. Defaults to the repository name.
        #[arg(long)]
        id: Option<String>,
        /// Regex an asset name must match.
        #[arg(long)]
        filter: Option<String>,
        /// Regex a release tag must match, e.g. `^v` to skip `android-…` releases.
        #[arg(long, value_name = "REGEX")]
        tag_filter: Option<String>,
        /// Package format to use instead of the best one for this system.
        #[arg(long)]
        format: Option<Format>,
        /// Consider prereleases.
        #[arg(long)]
        prerelease: bool,
        /// Executable to link from a tarball, relative to its top directory.
        /// Repeat for several. Found automatically if not given.
        #[arg(long = "bin", value_name = "PATH")]
        bins: Vec<String>,
        /// Only report new releases, never install the app: for apps with no
        /// Linux build, or ones installed some other way.
        #[arg(long)]
        track_only: bool,
        #[command(flatten)]
        signature: SignatureArgs,
    },
    /// Change how a tracked app is installed. Takes effect on the next `update`.
    Edit {
        id: String,
        /// Package format to pin, or `auto` to pick the best one for this system.
        #[arg(long, value_name = "FORMAT")]
        format: Option<FormatPin>,
        /// Regex an asset name must match.
        #[arg(long, conflicts_with = "clear_filter")]
        filter: Option<String>,
        /// Remove the asset filter.
        #[arg(long)]
        clear_filter: bool,
        /// Regex a release tag must match, e.g. `^v` to skip `android-…` releases.
        #[arg(long, value_name = "REGEX", conflicts_with = "clear_tag_filter")]
        tag_filter: Option<String>,
        /// Follow releases with any tag again.
        #[arg(long)]
        clear_tag_filter: bool,
        /// Executable to link from a tarball, replacing the current list.
        /// Repeat for several.
        #[arg(long = "bin", value_name = "PATH", conflicts_with = "clear_bins")]
        bins: Vec<String>,
        /// Find a tarball's executables automatically again.
        #[arg(long)]
        clear_bins: bool,
        /// Whether to consider prereleases.
        #[arg(long, value_name = "BOOL")]
        prerelease: Option<bool>,
        /// Whether to only report new releases instead of installing them.
        #[arg(long, value_name = "BOOL")]
        track_only: Option<bool>,
        #[command(flatten)]
        signature: SignatureArgs,
        /// New download pattern, for an app tracked from a web page.
        #[arg(long, value_name = "REGEX")]
        pattern: Option<String>,
        /// Stop requiring a signature.
        #[arg(long, conflicts_with_all = ["minisign_key", "gpg_key", "attestation"])]
        no_signature: bool,
    },
    /// Show the release notes of an app's latest release, or of `--tag`.
    Notes {
        id: String,
        #[arg(long)]
        tag: Option<String>,
    },
    /// Add the menu entries missing from AppImages and tarballs (all apps if
    /// none are given), e.g. ones installed before Forager made entries.
    MenuEntries { ids: Vec<String> },
    /// List tracked apps.
    List {
        /// Print the list as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Check tracked apps for new releases.
    Check {
        /// Show a desktop notification if updates are available.
        #[arg(long)]
        notify: bool,
        /// Print nothing except errors.
        #[arg(long, conflicts_with = "json")]
        quiet: bool,
        /// Print the results as JSON, errors included.
        #[arg(long)]
        json: bool,
    },
    /// Install new releases of tracked apps (all apps if none are given).
    Update {
        ids: Vec<String>,
        /// Do not ask for confirmation.
        #[arg(short, long)]
        yes: bool,
        /// Install assets that have no published checksum.
        #[arg(long)]
        allow_unverified: bool,
        /// Skip updates that need root (deb, rpm and Arch packages).
        #[arg(long)]
        unprivileged_only: bool,
        /// Install nothing over a metered connection (as NetworkManager sees it).
        #[arg(long)]
        skip_metered: bool,
        /// Install nothing while running on battery.
        #[arg(long)]
        skip_on_battery: bool,
    },
    /// Install a particular release, e.g. to roll back to an older one. Going
    /// back from the latest release skips it, so it is not offered again.
    Install {
        id: String,
        /// The release's tag.
        #[arg(long)]
        tag: String,
        /// Do not ask for confirmation.
        #[arg(short, long)]
        yes: bool,
        /// Install assets that have no published checksum.
        #[arg(long)]
        allow_unverified: bool,
    },
    /// Do not offer a release (the latest by default) as an update. A newer
    /// release is offered as usual.
    Skip {
        id: String,
        #[arg(long)]
        tag: Option<String>,
    },
    /// Mark a tracked-only app's latest release (or `--tag`) as seen, so it
    /// is no longer reported as new.
    Seen {
        id: String,
        #[arg(long)]
        tag: Option<String>,
    },
    /// Offer the skipped release again.
    Unskip { id: String },
    /// Show what Forager installed, updated and removed, newest first.
    History {
        /// Number of events to show.
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
        /// Print the events as JSON, newest first.
        #[arg(long)]
        json: bool,
    },
    /// Manage the daily background check (a systemd user timer).
    Timer {
        #[command(subcommand)]
        action: TimerAction,
    },
    /// Track an AppImage downloaded by hand: it moves to ~/Applications, gets
    /// a menu entry, and is updated from the project its update information
    /// names, or from `--url`.
    Adopt {
        file: PathBuf,
        /// The project's address, for AppImages that do not name one.
        #[arg(long)]
        url: Option<String>,
        /// API the host speaks, as for `add`.
        #[arg(long, requires = "url")]
        forge: Option<Forge>,
        /// Id to refer to the app by. Defaults to the repository name.
        #[arg(long)]
        id: Option<String>,
        /// Regex an asset name must match, instead of the AppImage's own pattern.
        #[arg(long)]
        filter: Option<String>,
        /// Copy the file instead of moving it.
        #[arg(long)]
        keep: bool,
    },
    /// Write every tracked app's settings to a file (standard output if
    /// none is given), to track them on another machine with `import`.
    Export { file: Option<PathBuf> },
    /// Track the apps in a file written by `export`, those not tracked yet.
    Import {
        file: PathBuf,
        /// Then install them, as `update` would.
        #[arg(long)]
        install: bool,
        /// With --install: do not ask for confirmation.
        #[arg(long, requires = "install")]
        yes: bool,
    },
    /// Print a link that opens Forager's Add page with an app's settings.
    Link {
        id: String,
        /// The same through a web page, for sites that drop forager:// links.
        #[arg(long, conflicts_with = "badge")]
        web: bool,
        /// Markdown for an "Add to Forager" badge, for the project's README.
        #[arg(long)]
        badge: bool,
    },
    /// Print a shell completion script, e.g.
    /// `forager completions bash > ~/.local/share/bash-completion/completions/forager`.
    Completions { shell: clap_complete::Shell },
    /// Write man pages for forager and each subcommand into a directory.
    #[command(hide = true)]
    Man { dir: PathBuf },
    /// Stop tracking an app and uninstall it. With several installs (e.g. an
    /// rpm and a Flatpak) it asks which ones to uninstall.
    Remove {
        id: String,
        /// Leave the installed package in place.
        #[arg(long)]
        keep_installed: bool,
    },
}

/// Require assets to be signed by a key the user trusts.
#[derive(clap::Args)]
#[group(multiple = false)]
struct SignatureArgs {
    /// Minisign public key (base64) or path to a minisign.pub file.
    #[arg(long, value_name = "KEY")]
    minisign_key: Option<String>,
    /// OpenPGP public key file (armored or binary).
    #[arg(long, value_name = "FILE")]
    gpg_key: Option<PathBuf>,
    /// GitHub artifact attestation from the app's repository (needs `gh`).
    #[arg(long)]
    attestation: bool,
}

impl SignatureArgs {
    fn is_empty(&self) -> bool {
        self.minisign_key.is_none() && self.gpg_key.is_none() && !self.attestation
    }

    fn resolve(&self, id: &str, source: &Source) -> Result<Option<Signature>> {
        let input = if let Some(key) = &self.minisign_key {
            SignatureInput::Minisign(key.clone())
        } else if let Some(file) = &self.gpg_key {
            SignatureInput::GpgKeyFile(file.clone())
        } else if self.attestation {
            SignatureInput::Attestation
        } else {
            return Ok(None);
        };
        Ok(Some(signature::resolve(&input, id, source)?))
    }
}

#[derive(Subcommand)]
enum TimerAction {
    /// Install and start the timer.
    Enable {
        /// Also install updates that need no root (AppImage, tarball, Flatpak).
        #[arg(long)]
        auto_update: bool,
        /// With --auto-update: skip the day's updates over a metered connection.
        #[arg(long, requires = "auto_update")]
        skip_metered: bool,
        /// With --auto-update: skip the day's updates while on battery.
        #[arg(long, requires = "auto_update")]
        skip_on_battery: bool,
    },
    /// Stop and remove the timer.
    Disable,
    /// Show whether the timer is enabled and when it runs next.
    Status,
}

#[derive(Clone, Copy)]
struct UpdateOptions {
    yes: bool,
    allow_unverified: bool,
    unprivileged_only: bool,
    via: Via,
}

/// `--format` value for `edit`: a format, or `auto` to remove the pin.
#[derive(Clone)]
struct FormatPin(Option<Format>);

impl std::str::FromStr for FormatPin {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("auto") {
            Ok(FormatPin(None))
        } else {
            s.parse().map(|f| FormatPin(Some(f)))
        }
    }
}

fn main() -> Result<()> {
    host::use_host_dirs();
    let cli = Cli::parse();
    // These need no config, so they work without a home directory and do
    // not wait for another forager process.
    match &cli.command {
        Command::Completions { shell } => {
            clap_complete::generate(*shell, &mut Cli::command(), "forager", &mut io::stdout());
            return Ok(());
        }
        Command::Man { dir } => return write_man_pages(dir),
        _ => {}
    }
    let explicit_config = cli.config.clone();
    let via = if cli.from_timer { Via::Timer } else { Via::Cli };
    let path = cli.config.unwrap_or_else(Store::default_path);
    let mut store = Store::load_waiting(&path, || eprintln!("Waiting for another forager process to finish..."))?;

    match cli.command {
        Command::Add {
            url,
            forge,
            pattern,
            id,
            filter,
            tag_filter,
            format,
            prerelease,
            bins,
            track_only,
            signature,
        } => {
            // A `forager://add?…` link, or the badge page's address, brings
            // its own settings; options given here win.
            let link = (url.starts_with("forager:") || url.starts_with(share::WEB_ADD))
                .then(|| share::parse_link(&url))
                .transpose()?;
            let source = match (&link, forge, pattern) {
                (Some(link), None, None) => link.source()?,
                (Some(_), _, _) => bail!("--forge and --pattern do not go with a forager:// link"),
                (None, Some(Forge::Html) | None, Some(pattern)) => Source::html(&url, &pattern)?,
                (None, _, Some(_)) => bail!("--pattern is only for web pages (--forge html)"),
                (None, forge, None) => Source::from_url(&url, forge)?,
            };
            let link = link.unwrap_or_default();
            let id = id.or(link.id).unwrap_or_else(|| source.default_id());
            let mut app = App::new(source, id.clone());
            app.asset_filter = filter.or(link.asset_filter);
            app.tag_filter = tag_filter.or(link.tag_filter);
            app.format = format.or(link.format);
            app.prerelease = prerelease || link.prerelease;
            app.bins = if bins.is_empty() { link.bins } else { bins };
            app.track_only = track_only || link.track_only;
            let track_only = app.track_only;
            app.validate()?;
            if store.get(&id).is_ok() {
                bail!("app already tracked: {id}");
            }
            // Only releases after this one are news.
            if track_only {
                app.seen = Some(update::latest(&app, app.source.releases()?, &Platform::detect())?.tag);
            }
            // Last, since a GPG key is stored on disk under the app's id.
            app.signature = signature.resolve(&id, &app.source)?;
            store.add(app)?;
            store.save()?;
            match track_only {
                true => println!("Tracking {id}; `forager check` reports its new releases."),
                false => println!("Tracking {id}. Run `forager update {id}` to install it."),
            }
        }
        Command::Edit {
            id,
            format,
            filter,
            clear_filter,
            tag_filter,
            clear_tag_filter,
            bins,
            clear_bins,
            prerelease,
            track_only,
            signature,
            no_signature,
            pattern,
        } => {
            if signature.is_empty()
                && !no_signature
                && pattern.is_none()
                && format.is_none()
                && filter.is_none()
                && !clear_filter
                && tag_filter.is_none()
                && !clear_tag_filter
                && bins.is_empty()
                && !clear_bins
                && prerelease.is_none()
                && track_only.is_none()
            {
                bail!("nothing to change, see `forager edit --help`");
            }
            if let Some(filter) = &filter {
                regex::Regex::new(filter).context("invalid --filter")?;
            }
            if let Some(filter) = &tag_filter {
                regex::Regex::new(filter).context("invalid --tag-filter")?;
            }
            let app = store.get_mut(&id)?;
            if let Some(FormatPin(format)) = format {
                app.format = format;
            }
            if filter.is_some() || clear_filter {
                app.asset_filter = filter;
            }
            if tag_filter.is_some() || clear_tag_filter {
                app.tag_filter = tag_filter;
            }
            if !bins.is_empty() || clear_bins {
                app.bins = bins;
            }
            if let Some(prerelease) = prerelease {
                app.prerelease = prerelease;
            }
            if let Some(track_only) = track_only {
                app.track_only = track_only;
            }
            if let Some(pattern) = pattern {
                let Source::Html { url, .. } = &app.source else {
                    bail!("--pattern is only for apps tracked from a web page");
                };
                app.source = Source::html(&url.clone(), &pattern)?;
            }
            if !signature.is_empty() || no_signature {
                let source = app.source.clone();
                let new = signature.resolve(&id, &source)?;
                let app = store.get_mut(&id)?;
                if let Some(Signature::Gpg { keyring }) = &app.signature
                    && !matches!(&new, Some(Signature::Gpg { keyring: k }) if k == keyring)
                {
                    std::fs::remove_file(keyring).ok();
                }
                app.signature = new;
            }
            let app = store.get(&id)?;
            let settings = describe(app);
            store.save()?;
            println!("{id}:\n{settings}");
            println!("Changes apply on the next `forager update {id}`.");
        }
        Command::Notes { id, tag } => {
            let app = store.get(&id)?;
            let releases = app.source.releases()?;
            let release = match &tag {
                Some(tag) => releases
                    .into_iter()
                    .find(|r| r.tag == *tag)
                    .with_context(|| format!("{id} has no release {tag}"))?,
                None => update::latest(app, releases, &Platform::detect())?,
            };
            let name = release.name.as_deref().filter(|n| *n != release.tag);
            println!("{}{}", release.tag, name.map_or(String::new(), |n| format!(" ({n})")));
            println!();
            println!("{}", release.notes.as_deref().unwrap_or("No release notes."));
        }
        Command::MenuEntries { ids } => {
            let results = update::create_menu_entries(&mut store, &ids)?;
            if results.is_empty() {
                println!("Every AppImage and tarball that can have a menu entry has one.");
            }
            let mut failed = 0;
            for (id, result) in results {
                match result {
                    Ok(_) => println!("{id}: menu entry added"),
                    Err(e) => {
                        eprintln!("{id}: {e}");
                        failed += 1;
                    }
                }
            }
            if failed > 0 {
                bail!(
                    "{failed} menu entr{} could not be made",
                    if failed == 1 { "y" } else { "ies" }
                );
            }
        }
        Command::List { json: true } => {
            let apps: Vec<ListEntry> = store.apps.iter().map(ListEntry::new).collect();
            println!("{}", serde_json::to_string_pretty(&apps)?);
        }
        Command::List { json: false } => {
            if store.apps.is_empty() {
                println!("No apps tracked. Add one with `forager add <url>`.");
            }
            for app in &store.apps {
                let installs: Vec<String> = app.installs().map(|i| format!("{} ({})", i.tag, i.format)).collect();
                let state = match installs.is_empty() {
                    true => "not installed".into(),
                    false => installs.join(", "),
                };
                println!("{:<20} {:<20} {}", app.id, state, app.source);
            }
        }
        Command::Check { notify, quiet, json } => check(&mut store, notify, quiet, json, via)?,
        Command::History { limit, json: true } => {
            let events: Vec<Event> = history::read(limit)?.into_iter().rev().collect();
            println!("{}", serde_json::to_string_pretty(&events)?);
        }
        Command::History { limit, json: false } => {
            for event in history::read(limit)?.iter().rev() {
                println!("{}", describe_event(event));
            }
        }
        Command::Update {
            ids,
            yes,
            allow_unverified,
            unprivileged_only,
            skip_metered,
            skip_on_battery,
        } => {
            let avoid = Avoid {
                metered: skip_metered,
                battery: skip_on_battery,
            };
            if let Some(reason) = conditions::blocked(avoid) {
                println!("Updates skipped: {reason}.");
                return Ok(());
            }
            update(
                &mut store,
                &ids,
                &UpdateOptions {
                    yes,
                    allow_unverified,
                    unprivileged_only,
                    via,
                },
            )?
        }
        Command::Install {
            id,
            tag,
            yes,
            allow_unverified,
        } => {
            let options = UpdateOptions {
                yes,
                allow_unverified,
                unprivileged_only: false,
                via,
            };
            let failed = run_plans(&mut store, &id, &Platform::detect(), &options, Some(&tag))?;
            if failed > 0 {
                bail!("{failed} install(s) failed");
            }
        }
        Command::Skip { id, tag } => {
            let app = store.get(&id)?;
            let tag = match tag {
                Some(tag) => tag,
                None => update::latest(app, app.source.releases()?, &Platform::detect())?.tag,
            };
            store.get_mut(&id)?.skipped = Some(tag.clone());
            store.save()?;
            println!("{id}: {tag} skipped; newer releases are offered as usual.");
        }
        Command::Seen { id, tag } => {
            let app = store.get(&id)?;
            if !app.track_only {
                bail!("{id} is installed by Forager, not tracked only; see `forager skip`");
            }
            let tag = match tag {
                Some(tag) => tag,
                None => update::latest(app, app.source.releases()?, &Platform::detect())?.tag,
            };
            store.get_mut(&id)?.seen = Some(tag.clone());
            store.save()?;
            println!("{id}: {tag} seen; newer releases are reported as new.");
        }
        Command::Unskip { id } => {
            let skipped = store.get_mut(&id)?.skipped.take();
            store.save()?;
            match skipped {
                Some(tag) => println!("{id}: {tag} is offered again."),
                None => println!("{id}: no release is skipped."),
            }
        }
        Command::Timer { action } => timer(action, explicit_config)?,
        Command::Completions { .. } | Command::Man { .. } => unreachable!("handled before the config is loaded"),
        Command::Adopt {
            file,
            url,
            forge,
            id,
            filter,
            keep,
        } => {
            let info = forager_core::appimage::inspect(&file)
                .with_context(|| format!("{} is not an AppImage", file.display()))?;
            // The address given wins over the one the AppImage names.
            let request = info.source.clone().unwrap_or_default();
            let source = match url {
                Some(url) => Source::from_url(&url, forge)?,
                None if info.source.is_some() => request.source()?,
                None => bail!(
                    "{} does not say where it comes from{}; give its project with --url",
                    file.display(),
                    info.update_info.as_deref().map_or(String::new(), |u| format!(" ({u})"))
                ),
            };
            let named = info
                .source
                .as_ref()
                .is_some_and(|r| r.source().ok().as_ref() == Some(&source));
            let request = if named { request } else { share::AddRequest::default() };
            let mut app = App::new(source.clone(), id.unwrap_or_else(|| source.default_id()));
            app.asset_filter = filter.or(request.asset_filter);
            app.tag_filter = request.tag_filter;
            app.prerelease = request.prerelease;
            let installed = update::adopt(&mut store, app.clone(), &file, info.version.as_deref(), keep)?;
            let mut event = Event::new(&app.id, Kind::Installed, via);
            event.to = Some(installed.tag.clone());
            event.format = Some(Format::AppImage);
            event.message = Some(format!("adopted {}", file.display()));
            history::record(&event).ok();
            println!(
                "{}: tracking {} from {source}",
                app.id,
                info.name.as_deref().unwrap_or("the AppImage")
            );
            println!("  version: {}", installed.tag);
            if let Some(path) = &installed.path {
                let verb = if keep { "copied:" } else { "moved: " };
                println!("  {verb}  {}", path.display());
            }
            if let Some(filter) = &app.asset_filter {
                println!("  filter:  {filter}");
            }
        }
        Command::Export { file } => {
            let text = share::export(&store)?;
            match file {
                Some(file) => {
                    std::fs::write(&file, text).with_context(|| format!("cannot write {}", file.display()))?;
                    eprintln!("Exported {} apps to {}.", store.apps.len(), file.display());
                }
                None => print!("{text}"),
            }
        }
        Command::Import { file, install, yes } => {
            let text = std::fs::read_to_string(&file).with_context(|| format!("cannot read {}", file.display()))?;
            let results = share::import(&mut store, &text)?;
            store.save()?;
            let mut added = Vec::new();
            for (id, result) in results {
                match result {
                    Ok(()) => {
                        println!("{id}: tracked");
                        added.push(id);
                    }
                    Err(e) => println!("{id}: skipped, {e}"),
                }
            }
            if install && !added.is_empty() {
                let options = UpdateOptions {
                    yes,
                    allow_unverified: false,
                    unprivileged_only: false,
                    via,
                };
                update(&mut store, &added, &options)?;
            } else if !added.is_empty() {
                println!("Run `forager update` to install them.");
            }
        }
        Command::Link { id, web, badge } => {
            let app = store.get(&id)?;
            let text = match (web, badge) {
                (true, _) => share::web_link(app),
                (_, true) => share::badge(app),
                _ => share::link(app),
            };
            println!("{text}");
        }
        Command::Remove { id, keep_installed } => {
            let app = store.get(&id)?;
            let present: Vec<Installed> = app.installs().filter(|i| install::is_present(i)).cloned().collect();
            let mut uninstall = Vec::new();
            if !keep_installed {
                let ask = present.len() > 1 && io::stdin().is_terminal();
                for installed in present {
                    if !ask || confirm(&format!("Uninstall the {} {}?", installed.format, installed.tag))? {
                        uninstall.push(installed);
                    }
                }
            }
            let platform = Platform::detect();
            for installed in &uninstall {
                install::uninstall(installed, &platform)?;
            }
            let app = store.remove(&id)?;
            store.save()?;
            if uninstall.is_empty() {
                history::record(&Event::new(&id, Kind::Removed, via)).ok();
            }
            for installed in &uninstall {
                let mut event = Event::new(&id, Kind::Removed, via);
                event.from = Some(installed.tag.clone());
                event.format = Some(installed.format);
                history::record(&event).ok();
            }
            if let Some(Signature::Gpg { keyring }) = &app.signature {
                std::fs::remove_file(keyring).ok();
            }
            println!("Removed {id}.");
        }
    }
    Ok(())
}

/// `forager.1`, and `forager-<subcommand>.1` for each visible subcommand.
fn write_man_pages(dir: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let command = Cli::command();
    let mut pages = vec![("forager".to_string(), command.clone())];
    for sub in command.get_subcommands().filter(|c| !c.is_hide_set()) {
        let name = format!("forager-{}", sub.get_name());
        pages.push((
            name.clone(),
            sub.clone()
                .display_name(name.clone())
                .bin_name(format!("forager {}", sub.get_name())),
        ));
    }
    for (name, page) in pages {
        let mut out = Vec::new();
        clap_mangen::Man::new(page).render(&mut out)?;
        std::fs::write(dir.join(format!("{name}.1")), out)?;
    }
    Ok(())
}

/// An app in `list --json`.
#[derive(serde::Serialize)]
struct ListEntry {
    id: String,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<Format>,
    installs: Vec<InstallEntry>,
}

impl ListEntry {
    fn new(app: &App) -> ListEntry {
        ListEntry {
            id: app.id.clone(),
            source: app.source.to_string(),
            format: app.format,
            installs: app.installs().map(|i| InstallEntry::new(i, None)).collect(),
        }
    }
}

/// An install in `list --json` and `check --json`.
#[derive(serde::Serialize)]
struct InstallEntry {
    format: Format,
    tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<State>,
    #[serde(skip_serializing_if = "Option::is_none")]
    package: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<PathBuf>,
}

impl InstallEntry {
    fn new(installed: &Installed, state: Option<State>) -> InstallEntry {
        InstallEntry {
            format: installed.format,
            tag: installed.tag.clone(),
            state,
            package: installed.package.clone(),
            path: installed.path.clone(),
        }
    }
}

/// An app in `check --json`: its state, or the error that kept it from
/// being checked.
#[derive(serde::Serialize)]
struct CheckEntry {
    id: String,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<State>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest: Option<String>,
    skipped: bool,
    track_only: bool,
    installs: Vec<InstallEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl CheckEntry {
    fn new(app: &App, status: &update::Status) -> CheckEntry {
        CheckEntry {
            id: app.id.clone(),
            source: app.source.to_string(),
            state: Some(status.state),
            latest: Some(status.latest.tag.clone()),
            skipped: status.skipped,
            track_only: app.track_only,
            installs: status
                .installs
                .iter()
                .map(|i| InstallEntry::new(&i.installed, Some(i.state)))
                .collect(),
            error: None,
        }
    }

    fn failed(app: &App, error: String) -> CheckEntry {
        CheckEntry {
            id: app.id.clone(),
            source: app.source.to_string(),
            state: None,
            latest: None,
            skipped: false,
            track_only: app.track_only,
            installs: Vec::new(),
            error: Some(error),
        }
    }
}

fn check(store: &mut Store, notify: bool, quiet: bool, json: bool, via: Via) -> Result<()> {
    let platform = Platform::detect();
    let mut pending = Vec::new();
    let mut found = false;
    let mut entries = Vec::new();
    for index in 0..store.apps.len() {
        let status = match update::status(&store.apps[index]) {
            Ok(status) => status,
            Err(e) if json => {
                entries.push(CheckEntry::failed(&store.apps[index], e.to_string()));
                continue;
            }
            Err(e) if quiet => {
                eprintln!("{}: {e}", store.apps[index].id);
                continue;
            }
            Err(e) => {
                println!("{:<20} error: {e}", store.apps[index].id);
                continue;
            }
        };
        // Installed or changed outside Forager: keep what the system reports.
        if let Some(installs) = status.found.clone() {
            store.apps[index].set_installs(installs);
            found = true;
        }
        let app = &store.apps[index];
        if status.state.needs_update() {
            pending.push((app.id.clone(), status.latest.tag.clone()));
        }
        if json {
            entries.push(CheckEntry::new(app, &status));
            continue;
        }
        if quiet {
            continue;
        }

        let latest = &status.latest.tag;
        if app.track_only {
            let text = match (status.state, &app.seen) {
                (State::Outdated, Some(seen)) => format!("{latest} released (seen {seen})"),
                (State::Outdated, None) => format!("{latest} released"),
                _ => format!("{latest} (seen)"),
            };
            println!("{:<20} {:<30} {}", app.id, text, app.source.release_url(latest));
            continue;
        }
        let text = match (status.state, status.installs.as_slice()) {
            (State::NotInstalled, _) => format!("{latest} available, not installed"),
            (State::WrongFormat, installs) => format!(
                "{} ({}) -> {latest} ({})",
                installs.first().map_or("", |i| i.installed.tag.as_str()),
                installs
                    .iter()
                    .map(|i| i.installed.format.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                app.format.map_or(String::new(), |f| f.to_string())
            ),
            (_, [one]) => describe_install(one, latest, &platform, false),
            (_, installs) => installs
                .iter()
                .map(|i| describe_install(i, latest, &platform, true))
                .collect::<Vec<_>>()
                .join("; "),
        };
        let behind = status
            .installs
            .iter()
            .any(|i| version::is_newer(&i.installed.tag, latest));
        let text = match status.skipped && behind {
            true => format!("{} ({latest} skipped)", text.replace(" (up to date)", "")),
            false => text,
        };
        let asset = match select_asset(app, &status.latest, &platform) {
            Ok((asset, _)) => asset.name.clone(),
            Err(e) => e.to_string(),
        };
        println!("{:<20} {:<30} {}", app.id, text, asset);
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    }
    if found {
        store.save()?;
    }
    if notify {
        record_updates_found(store, &pending, via);
    }
    if notify && !pending.is_empty() {
        let (released, updates): (Vec<&str>, Vec<&str>) = pending
            .iter()
            .map(|(id, _)| id.as_str())
            .partition(|id| store.get(id).is_ok_and(|a| a.track_only));
        let (summary, body) = notification_text(&updates, &released);
        notify_rust::Notification::new()
            .appname("Forager")
            .summary(&summary)
            .body(&body)
            .icon("system-software-update")
            // Clicking the notification opens the app: Forager.desktop from
            // the deb and rpm packages, or the Flatpak's own entry.
            .hint(notify_rust::Hint::DesktopEntry(host::desktop_entry()))
            .show()
            .context("cannot show notification")?;
    }
    Ok(())
}

/// Summary and body of the background check's notification: updates to
/// install, and new releases of apps that are tracked only.
fn notification_text(updates: &[&str], released: &[&str]) -> (String, String) {
    let plural = |n: usize, one: &str, many: &str| match n {
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    };
    let mut summary = Vec::new();
    let mut body = Vec::new();
    if !updates.is_empty() {
        summary.push(plural(updates.len(), "update available", "updates available"));
        body.push(format!("{}\nRun `forager update` to install.", updates.join(", ")));
    }
    if !released.is_empty() {
        summary.push(plural(released.len(), "new release", "new releases"));
        body.push(format!("New releases: {}", released.join(", ")));
    }
    (summary.join(", "), body.join("\n"))
}

/// One install in `check`: `v1 (up to date)`, `v1 -> v2`, `missing (was v1)`,
/// with the format when the app has several, and the distro's version of a
/// system package.
fn describe_install(install: &InstallState, latest: &str, platform: &Platform, with_format: bool) -> String {
    let InstallState { installed, state } = install;
    let tag = &installed.tag;
    let format = installed.format;
    let text = match (state, with_format) {
        (State::Missing, false) => format!("missing (was {tag}), {latest} available"),
        (State::Missing, true) => format!("missing {format} (was {tag})"),
        (State::Outdated, false) => format!("{tag} -> {latest}"),
        (State::Outdated, true) => format!("{tag} ({format}) -> {latest}"),
        (_, false) => format!("{tag} (up to date)"),
        (_, true) => format!("{tag} ({format}, up to date)"),
    };
    match installed.package.as_deref().filter(|_| format.needs_root()) {
        Some(package) => match install::repo_version(package, platform) {
            Some(version) => format!("{text} [distro: {version}]"),
            None => text,
        },
        None => text,
    }
}

/// Records each newly found update once, not again on every daily check.
fn record_updates_found(store: &Store, pending: &[(String, String)], via: Via) {
    let recent = history::read(500).unwrap_or_default();
    for (id, tag) in pending {
        let seen = recent
            .iter()
            .any(|e| e.app == *id && e.kind == Kind::UpdatesFound && e.to.as_deref() == Some(tag.as_str()));
        if seen {
            continue;
        }
        let mut event = Event::new(id, Kind::UpdatesFound, via);
        event.from = store
            .get(id)
            .ok()
            .and_then(|a| a.installed.as_ref())
            .map(|i| i.tag.clone());
        event.to = Some(tag.clone());
        history::record(&event).ok();
    }
}

fn describe_event(event: &Event) -> String {
    let what = match event.kind {
        Kind::Installed => "installed",
        Kind::Updated => "updated",
        Kind::Removed => "removed",
        Kind::Failed => "failed",
        Kind::UpdatesFound => "update found",
    };
    let versions = match (&event.from, &event.to) {
        (Some(from), Some(to)) => format!(" {from} -> {to}"),
        (None, Some(to)) => format!(" {to}"),
        (Some(from), None) => format!(" {from}"),
        (None, None) => String::new(),
    };
    let format = event.format.map_or(String::new(), |f| format!(" ({f})"));
    let message = event.message.as_deref().map_or(String::new(), |m| format!(": {m}"));
    let via = match event.via {
        Via::Cli => "",
        Via::Gui => " [gui]",
        Via::Timer => " [timer]",
    };
    format!(
        "{}  {:<16} {what}{versions}{format}{via}{message}",
        utc_time(event.time),
        event.app
    )
}

/// `YYYY-MM-DD HH:MM UTC`, without a date library.
fn utc_time(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let (hour, minute) = (secs % 86400 / 3600, secs % 3600 / 60);
    // Howard Hinnant's civil_from_days.
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02} UTC")
}

fn timer(action: TimerAction, config: Option<PathBuf>) -> Result<()> {
    match action {
        TimerAction::Enable {
            auto_update,
            skip_metered,
            skip_on_battery,
        } => {
            let config = config.map(std::path::absolute).transpose()?;
            let settings = timer::Settings {
                command: host::cli_command(&std::env::current_exe().context("cannot locate the forager binary")?),
                config,
                auto_update,
                avoid: Avoid {
                    metered: skip_metered,
                    battery: skip_on_battery,
                },
            };
            timer::enable(&settings)?;
            println!("Daily check enabled ({}).", timer::TIMER);
            if auto_update {
                println!("Updates that need no root are installed automatically.");
            }
            match (skip_metered, skip_on_battery) {
                (true, true) => println!("Not over a metered connection or on battery."),
                (true, false) => println!("Not over a metered connection."),
                (false, true) => println!("Not on battery."),
                (false, false) => {}
            }
        }
        TimerAction::Disable => {
            timer::disable()?;
            println!("Daily check disabled.");
        }
        TimerAction::Status => match timer::next_run()? {
            Some(next) => println!("Enabled, next run: {next}"),
            None => println!("Disabled. Enable with `forager timer enable`."),
        },
    }
    Ok(())
}

fn update(store: &mut Store, ids: &[String], options: &UpdateOptions) -> Result<()> {
    let platform = Platform::detect();
    let ids: Vec<String> = if ids.is_empty() {
        store.apps.iter().map(|a| a.id.clone()).collect()
    } else {
        ids.to_vec()
    };

    let mut failed = 0;
    for id in &ids {
        match run_plans(store, id, &platform, options, None) {
            Ok(failures) => failed += failures,
            Err(e) => {
                eprintln!("{id}: {e:#}");
                let mut event = Event::new(id, Kind::Failed, options.via);
                event.message = Some(format!("{e:#}"));
                history::record(&event).ok();
                failed += 1;
            }
        }
    }
    if failed > 0 {
        bail!("{failed} update(s) failed");
    }
    Ok(())
}

/// Updates every install of `id` that needs it, each in its own format, or
/// moves every install to the release tagged `tag`. Returns how many of the
/// installs failed; those are reported and recorded.
fn run_plans(
    store: &mut Store,
    id: &str,
    platform: &Platform,
    options: &UpdateOptions,
    tag: Option<&str>,
) -> Result<usize> {
    let app = store.get_mut(id)?;
    if app.track_only && tag.is_none() {
        println!(
            "{id}: tracked only, not installed; releases at {}",
            app.source.web_url()
        );
        return Ok(0);
    }
    let recorded = app.install_list();
    let plans = match tag {
        Some(tag) => update::plans_for(app, platform, tag)?,
        None => update::plans(app, platform)?,
    };
    // Installed or changed outside Forager: keep what the system reports.
    if app.install_list() != recorded {
        store.save()?;
    }
    if plans.is_empty() {
        let app = store.get(id)?;
        match app.also_installed.is_empty() {
            true => println!(
                "{id}: {} is up to date",
                app.installed.as_ref().map_or("", |i| i.tag.as_str())
            ),
            false => {
                let installs: Vec<String> = app.installs().map(|i| format!("{} ({})", i.tag, i.format)).collect();
                println!("{id}: {} are up to date", installs.join(", "));
            }
        }
        return Ok(0);
    }
    let several = plans.len() > 1;
    let mut failed = 0;
    for plan in plans {
        let (name, result) = match plan {
            Ok(plan) => {
                let name = match several {
                    true => format!("{id} ({})", plan.format),
                    false => id.to_string(),
                };
                let result = apply(store, id, &name, &plan, platform, options);
                (name, result.map_err(|e| (e, Some(plan.format))))
            }
            Err(e) => (id.to_string(), Err((e.into(), None))),
        };
        if let Err((e, format)) = result {
            eprintln!("{name}: {e:#}");
            let mut event = Event::new(id, Kind::Failed, options.via);
            event.message = Some(format!("{e:#}"));
            event.format = format;
            history::record(&event).ok();
            failed += 1;
        }
    }
    Ok(failed)
}

/// Downloads, verifies and installs one plan of `id`, printed as `name`.
fn apply(
    store: &mut Store,
    id: &str,
    name: &str,
    plan: &Plan,
    platform: &Platform,
    options: &UpdateOptions,
) -> Result<()> {
    let UpdateOptions {
        yes,
        allow_unverified,
        unprivileged_only,
        via,
    } = *options;
    let app = store.get(id)?;
    let replaces_root_install = plan.replaces.iter().any(|i| i.format.needs_root());
    if unprivileged_only && (plan.format.needs_root() || replaces_root_install) {
        println!("{name}: {} needs root, skipped", plan.release.tag);
        return Ok(());
    }

    let from = match (&plan.from, plan.missing) {
        (Some(tag), true) => format!("missing (was {tag})"),
        (Some(tag), false) => tag.clone(),
        (None, _) => "not installed".into(),
    };
    let older = if plan.downgrade { " (older)" } else { "" };
    println!("{name}: {from} -> {}{older}", plan.release.tag);
    for line in update::summary(app, plan) {
        println!("  {:<8}{}", format!("{}:", line.label), line.value);
    }
    if plan.unverified() && yes && !allow_unverified {
        bail!("refusing unverified asset without --allow-unverified");
    }
    if !yes && !confirm("Install?")? {
        println!("{name}: skipped");
        return Ok(());
    }
    let first_install = app.install_of(plan.format).is_none();

    let mut line = ProgressLine::new();
    let download = update::download(plan, platform, &mut |p| line.show(p));
    line.finish();
    let download = download?;
    if let Some(delta) = &download.delta {
        println!("  delta:  {delta}");
    }
    if let (Some(package), Some(version)) = (&download.package, &download.repo_version) {
        println!("  note:   the distro repositories also have {package} {version}; system updates");
        println!("          and this build may replace each other");
        // Asked once; later updates of the same package only show the note.
        if first_install && !yes && !confirm("Install anyway?")? {
            println!("{name}: skipped");
            return Ok(());
        }
    }
    let installed = update::install(store, id, plan, &download, platform, &mut |p| line.show(p));
    line.finish();
    let installed = installed?;
    let kind = if plan.from.is_some() {
        Kind::Updated
    } else {
        Kind::Installed
    };
    let mut event = Event::new(id, kind, via);
    event.from = plan.from.clone();
    event.to = Some(plan.release.tag.clone());
    event.format = Some(plan.format);
    history::record(&event).ok();
    let restart = install::is_running_self(&installed);
    let links = installed.links;
    println!("{name}: installed {}", plan.release.tag);
    if restart {
        println!(
            "  note:   restart Forager to use {}; open windows still run the old version",
            plan.release.tag
        );
    }
    if let Some(skipped) = &plan.skip_after {
        println!("  note:   {skipped} is skipped; `forager unskip {id}` offers it again");
    }
    for link in &links {
        println!("  linked: {}", link.display());
    }
    if !links.is_empty()
        && let Some(bin_dir) = platform::bin_dir()
        && !host::path().is_some_and(|paths| std::env::split_paths(&paths).any(|p| p == bin_dir))
    {
        println!("  note:   {} is not on PATH", bin_dir.display());
    }
    Ok(())
}

fn describe(app: &App) -> String {
    let format = app.format.map_or("auto".to_string(), |f| f.to_string());
    let filter = app.asset_filter.as_deref().unwrap_or("none");
    let tags = app.tag_filter.as_deref().unwrap_or("any");
    let bins = if app.bins.is_empty() {
        "auto".to_string()
    } else {
        app.bins.join(", ")
    };
    let prerelease = if app.prerelease { "yes" } else { "no" };
    let signature = app.signature.as_ref().map_or("none".to_string(), |s| s.to_string());
    let pattern = match &app.source {
        Source::Html { pattern, .. } => format!("\n  pattern:    {pattern}"),
        _ => String::new(),
    };
    let pattern = match app.track_only {
        true => format!("{pattern}\n  track only: yes"),
        false => pattern,
    };
    format!(
        "  format:     {format}\n  filter:     {filter}\n  tags:       {tags}\n  bins:       {bins}\n  prerelease: {prerelease}\n  signature:  {signature}{pattern}"
    )
}

/// One self-overwriting status line, shown only on a terminal.
struct ProgressLine {
    tty: bool,
    shown: Option<String>,
}

impl ProgressLine {
    fn new() -> ProgressLine {
        ProgressLine {
            tty: io::stdout().is_terminal(),
            shown: None,
        }
    }

    fn show(&mut self, progress: update::Progress) -> ControlFlow<()> {
        if !self.tty {
            return ControlFlow::Continue(());
        }
        let mib = |bytes: u64| bytes as f64 / 1048576.0;
        let text = match progress {
            update::Progress::Download {
                done,
                total: Some(total),
            } if total > 0 => {
                format!("  download: {:>3}% of {:.1} MiB", done * 100 / total, mib(total))
            }
            update::Progress::Download { done, .. } => format!("  download: {:.1} MiB", mib(done)),
            update::Progress::Install { percent } => format!("  install:  {percent:>3}%"),
        };
        // Repaint only when the text changes, not for every 64 KiB chunk.
        if self.shown.as_deref() != Some(text.as_str()) {
            print!("\r{text:<40}");
            io::stdout().flush().ok();
            self.shown = Some(text);
        }
        ControlFlow::Continue(())
    }

    fn finish(&mut self) {
        if self.shown.take().is_some() {
            println!();
        }
    }
}

fn confirm(question: &str) -> Result<bool> {
    print!("{question} [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_tell_updates_from_new_releases() {
        assert_eq!(
            notification_text(&["a", "b"], &[]),
            (
                "2 updates available".into(),
                "a, b\nRun `forager update` to install.".into()
            )
        );
        assert_eq!(
            notification_text(&["a"], &["c"]),
            (
                "1 update available, 1 new release".into(),
                "a\nRun `forager update` to install.\nNew releases: c".into()
            )
        );
    }

    #[test]
    fn check_entries_serialise_as_json() {
        let app = App::new(
            Source::GitHub {
                owner: "o".into(),
                repo: "tool".into(),
            },
            "tool".into(),
        );
        let failed = serde_json::to_value(CheckEntry::failed(&app, "offline".into())).unwrap();
        assert_eq!(
            failed,
            serde_json::json!({"id": "tool", "source": "github.com/o/tool", "skipped": false,
                               "track_only": false, "installs": [], "error": "offline"})
        );
        let installed = Installed {
            tag: "v1".into(),
            format: Format::AppImage,
            asset: "tool.AppImage".into(),
            package: None,
            path: Some("/a/tool.AppImage".into()),
            links: Vec::new(),
            desktop: Vec::new(),
        };
        let entry = serde_json::to_value(InstallEntry::new(&installed, Some(State::Outdated))).unwrap();
        assert_eq!(
            entry,
            serde_json::json!({"format": "appimage", "tag": "v1", "state": "outdated", "path": "/a/tool.AppImage"})
        );
    }

    #[test]
    fn formats_utc_times() {
        assert_eq!(utc_time(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc_time(951_782_400), "2000-02-29 00:00 UTC");
        assert_eq!(utc_time(1_792_800_000 + 3_660), "2026-10-24 01:01 UTC");
    }
}
