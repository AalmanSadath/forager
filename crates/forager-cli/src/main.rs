use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use forager_core::history::{self, Event, Kind, Via};
use forager_core::matcher::select_asset;
use forager_core::model::{App, Format, Signature};
use forager_core::platform::{self, Platform};
use forager_core::signature::{self, SignatureInput};
use forager_core::source::{Forge, Source};
use forager_core::store::Store;
use forager_core::update::{self, State};
use forager_core::{install, timer};

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
    /// Track an app by its project URL.
    Add {
        url: String,
        /// API the host speaks, for hosts other than github.com, codeberg.org
        /// and well-known GitLab instances.
        #[arg(long)]
        forge: Option<Forge>,
        /// Id to refer to the app by. Defaults to the repository name.
        #[arg(long)]
        id: Option<String>,
        /// Regex an asset name must match.
        #[arg(long)]
        filter: Option<String>,
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
        #[command(flatten)]
        signature: SignatureArgs,
        /// Stop requiring a signature.
        #[arg(long, conflicts_with_all = ["minisign_key", "gpg_key", "attestation"])]
        no_signature: bool,
    },
    /// List tracked apps.
    List,
    /// Check tracked apps for new releases.
    Check {
        /// Show a desktop notification if updates are available.
        #[arg(long)]
        notify: bool,
        /// Print nothing except errors.
        #[arg(long)]
        quiet: bool,
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
    },
    /// Show what Forager installed, updated and removed, newest first.
    History {
        /// Number of events to show.
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
    /// Manage the daily background check (a systemd user timer).
    Timer {
        #[command(subcommand)]
        action: TimerAction,
    },
    /// Stop tracking an app and uninstall it.
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
    let cli = Cli::parse();
    let explicit_config = cli.config.clone();
    let via = if cli.from_timer { Via::Timer } else { Via::Cli };
    let path = cli.config.unwrap_or_else(Store::default_path);
    let mut store = Store::load_waiting(&path, || eprintln!("Waiting for another forager process to finish..."))?;

    match cli.command {
        Command::Add {
            url,
            forge,
            id,
            filter,
            format,
            prerelease,
            bins,
            signature,
        } => {
            let source = Source::from_url(&url, forge)?;
            let id = id.unwrap_or_else(|| source.default_id());
            let mut app = App::new(source, id.clone());
            app.asset_filter = filter;
            app.format = format;
            app.prerelease = prerelease;
            app.bins = bins;
            app.validate()?;
            if store.get(&id).is_ok() {
                bail!("app already tracked: {id}");
            }
            // Last, since a GPG key is stored on disk under the app's id.
            app.signature = signature.resolve(&id, &app.source)?;
            store.add(app)?;
            store.save()?;
            println!("Tracking {id}. Run `forager update {id}` to install it.");
        }
        Command::Edit {
            id,
            format,
            filter,
            clear_filter,
            bins,
            clear_bins,
            prerelease,
            signature,
            no_signature,
        } => {
            if signature.is_empty()
                && !no_signature
                && format.is_none()
                && filter.is_none()
                && !clear_filter
                && bins.is_empty()
                && !clear_bins
                && prerelease.is_none()
            {
                bail!("nothing to change, see `forager edit --help`");
            }
            if let Some(filter) = &filter {
                regex::Regex::new(filter).context("invalid --filter")?;
            }
            let app = store.get_mut(&id)?;
            if let Some(FormatPin(format)) = format {
                app.format = format;
            }
            if filter.is_some() || clear_filter {
                app.asset_filter = filter;
            }
            if !bins.is_empty() || clear_bins {
                app.bins = bins;
            }
            if let Some(prerelease) = prerelease {
                app.prerelease = prerelease;
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
        Command::List => {
            if store.apps.is_empty() {
                println!("No apps tracked. Add one with `forager add <url>`.");
            }
            for app in &store.apps {
                let state = match &app.installed {
                    Some(i) => format!("{} ({})", i.tag, i.format),
                    None => "not installed".into(),
                };
                println!("{:<20} {:<20} {}", app.id, state, app.source);
            }
        }
        Command::Check { notify, quiet } => check(&store, notify, quiet, via)?,
        Command::History { limit } => {
            for event in history::read(limit)?.iter().rev() {
                println!("{}", describe_event(event));
            }
        }
        Command::Update {
            ids,
            yes,
            allow_unverified,
            unprivileged_only,
        } => update(
            &mut store,
            &ids,
            &UpdateOptions {
                yes,
                allow_unverified,
                unprivileged_only,
                via,
            },
        )?,
        Command::Timer { action } => timer(action, explicit_config)?,
        Command::Remove { id, keep_installed } => {
            let app = store.get(&id)?;
            if let Some(installed) = &app.installed
                && !keep_installed
                && install::is_present(installed)
            {
                install::uninstall(installed, &Platform::detect())?;
            }
            let app = store.remove(&id)?;
            store.save()?;
            let mut event = Event::new(&id, Kind::Removed, via);
            event.from = app
                .installed
                .as_ref()
                .filter(|_| !keep_installed)
                .map(|i| i.tag.clone());
            history::record(&event).ok();
            if let Some(Signature::Gpg { keyring }) = &app.signature {
                std::fs::remove_file(keyring).ok();
            }
            println!("Removed {id}.");
        }
    }
    Ok(())
}

fn check(store: &Store, notify: bool, quiet: bool, via: Via) -> Result<()> {
    let platform = Platform::detect();
    let mut pending = Vec::new();
    for app in &store.apps {
        let status = match update::status(app) {
            Ok(status) => status,
            Err(e) if quiet => {
                eprintln!("{}: {e}", app.id);
                continue;
            }
            Err(e) => {
                println!("{:<20} error: {e}", app.id);
                continue;
            }
        };
        if status.state.needs_update() {
            pending.push((app.id.as_str(), status.latest.tag.clone()));
        }
        if quiet {
            continue;
        }

        let latest = &status.latest.tag;
        let installed = app.installed.as_ref();
        let tag = installed.map_or("", |i| i.tag.as_str());
        let text = match status.state {
            State::NotInstalled => format!("{latest} available, not installed"),
            State::Missing => format!("missing (was {tag}), {latest} available"),
            State::WrongFormat => format!(
                "{tag} ({}) -> {latest} ({})",
                installed.map_or(String::new(), |i| i.format.to_string()),
                app.format.map_or(String::new(), |f| f.to_string())
            ),
            State::Outdated => format!("{tag} -> {latest}"),
            State::UpToDate => format!("{tag} (up to date)"),
        };
        let text = match installed.and_then(|i| i.package.as_deref().filter(|_| i.format.needs_root())) {
            Some(package) => match install::repo_version(package, &platform) {
                Some(version) => format!("{text} [distro: {version}]"),
                None => text,
            },
            None => text,
        };
        let asset = match select_asset(app, &status.latest, &platform) {
            Ok((asset, _)) => asset.name.clone(),
            Err(e) => e.to_string(),
        };
        println!("{:<20} {:<30} {}", app.id, text, asset);
    }

    if notify {
        record_updates_found(store, &pending, via);
    }
    if notify && !pending.is_empty() {
        let summary = match pending.len() {
            1 => "1 update available".to_string(),
            n => format!("{n} updates available"),
        };
        notify_rust::Notification::new()
            .appname("Forager")
            .summary(&summary)
            .body(&format!(
                "{}\nRun `forager update` to install.",
                pending.iter().map(|(id, _)| *id).collect::<Vec<_>>().join(", ")
            ))
            .icon("system-software-update")
            .show()
            .context("cannot show notification")?;
    }
    Ok(())
}

/// Records each newly found update once, not again on every daily check.
fn record_updates_found(store: &Store, pending: &[(&str, String)], via: Via) {
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
        TimerAction::Enable { auto_update } => {
            let config = config.map(std::path::absolute).transpose()?;
            let settings = timer::Settings {
                exe: std::env::current_exe().context("cannot locate the forager binary")?,
                config,
                auto_update,
            };
            timer::enable(&settings)?;
            println!("Daily check enabled ({}).", timer::TIMER);
            if auto_update {
                println!("Updates that need no root are installed automatically.");
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
        if let Err(e) = update_one(store, id, &platform, options) {
            eprintln!("{id}: {e:#}");
            let mut event = Event::new(id, Kind::Failed, options.via);
            event.message = Some(format!("{e:#}"));
            history::record(&event).ok();
            failed += 1;
        }
    }
    if failed > 0 {
        bail!("{failed} of {} app(s) failed to update", ids.len());
    }
    Ok(())
}

fn update_one(store: &mut Store, id: &str, platform: &Platform, options: &UpdateOptions) -> Result<()> {
    let UpdateOptions {
        yes,
        allow_unverified,
        unprivileged_only,
        via,
    } = *options;
    let app = store.get(id)?;
    let Some(plan) = update::plan(app, platform)? else {
        let tag = app.installed.as_ref().map_or("", |i| i.tag.as_str());
        println!("{id}: {tag} is up to date");
        return Ok(());
    };
    let replaces_root_install = app
        .installed
        .as_ref()
        .is_some_and(|i| i.format != plan.format && i.format.needs_root());
    if unprivileged_only && (plan.format.needs_root() || replaces_root_install) {
        println!("{id}: {} needs root, skipped", plan.release.tag);
        return Ok(());
    }

    let from = match (&plan.from, plan.missing) {
        (Some(tag), true) => format!("missing (was {tag})"),
        (Some(tag), false) => tag.clone(),
        (None, _) => "not installed".into(),
    };
    println!("{id}: {from} -> {}", plan.release.tag);
    for line in update::summary(app, &plan) {
        println!("  {:<8}{}", format!("{}:", line.label), line.value);
    }
    if plan.unverified() && yes && !allow_unverified {
        bail!("refusing unverified asset without --allow-unverified");
    }
    if !yes && !confirm("Install?")? {
        println!("{id}: skipped");
        return Ok(());
    }
    let first_install = app.installed.as_ref().is_none_or(|i| i.format != plan.format);

    let mut line = ProgressLine::new();
    let download = update::download(&plan, platform, &mut |p| line.show(p));
    line.finish();
    let download = download?;
    if let (Some(package), Some(version)) = (&download.package, &download.repo_version) {
        println!("  note:   the distro repositories also have {package} {version}; system updates");
        println!("          and this build may replace each other");
        // Asked once; later updates of the same package only show the note.
        if first_install && !yes && !confirm("Install anyway?")? {
            println!("{id}: skipped");
            return Ok(());
        }
    }
    let installed = update::install(store, id, &plan, &download, platform, &mut |p| line.show(p));
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
    let links = installed.links;
    println!("{id}: installed {}", plan.release.tag);
    for link in &links {
        println!("  linked: {}", link.display());
    }
    if !links.is_empty()
        && let Some(bin_dir) = platform::bin_dir()
        && !std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|p| p == bin_dir))
    {
        println!("  note:   {} is not on PATH", bin_dir.display());
    }
    Ok(())
}

fn describe(app: &App) -> String {
    let format = app.format.map_or("auto".to_string(), |f| f.to_string());
    let filter = app.asset_filter.as_deref().unwrap_or("none");
    let bins = if app.bins.is_empty() {
        "auto".to_string()
    } else {
        app.bins.join(", ")
    };
    let prerelease = if app.prerelease { "yes" } else { "no" };
    let signature = app.signature.as_ref().map_or("none".to_string(), |s| s.to_string());
    format!(
        "  format:     {format}\n  filter:     {filter}\n  bins:       {bins}\n  prerelease: {prerelease}\n  signature:  {signature}"
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

    fn show(&mut self, progress: update::Progress) {
        if !self.tty {
            return;
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
    fn formats_utc_times() {
        assert_eq!(utc_time(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc_time(951_782_400), "2000-02-29 00:00 UTC");
        assert_eq!(utc_time(1_792_800_000 + 3_660), "2026-10-24 01:01 UTC");
    }
}
