use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use forager_core::matcher::select_asset;
use forager_core::model::{App, Format};
use forager_core::platform::{self, Platform};
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
        } => {
            let source = Source::from_url(&url, forge)?;
            let id = id.unwrap_or_else(|| source.default_id());
            if let Some(filter) = &filter {
                regex::Regex::new(filter).context("invalid --filter")?;
            }
            store.add(App {
                id: id.clone(),
                source,
                asset_filter: filter,
                format,
                prerelease,
                bins,
                installed: None,
            })?;
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
        } => {
            if format.is_none()
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
        Command::Check { notify, quiet } => check(&store, notify, quiet)?,
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
            store.remove(&id)?;
            store.save()?;
            println!("Removed {id}.");
        }
    }
    Ok(())
}

fn check(store: &Store, notify: bool, quiet: bool) -> Result<()> {
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
            pending.push(app.id.as_str());
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
        let asset = match select_asset(app, &status.latest, &platform) {
            Ok((asset, _)) => asset.name.clone(),
            Err(e) => e.to_string(),
        };
        println!("{:<20} {:<30} {}", app.id, text, asset);
    }

    if notify && !pending.is_empty() {
        let summary = match pending.len() {
            1 => "1 update available".to_string(),
            n => format!("{n} updates available"),
        };
        notify_rust::Notification::new()
            .appname("Forager")
            .summary(&summary)
            .body(&format!("{}\nRun `forager update` to install.", pending.join(", ")))
            .icon("system-software-update")
            .show()
            .context("cannot show notification")?;
    }
    Ok(())
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
    // GitLab does not report asset sizes.
    let size = match plan.asset.size {
        0 => String::new(),
        bytes => format!("{:.1} MiB, ", bytes as f64 / 1048576.0),
    };
    println!("  asset:  {} ({size}{})", plan.asset.name, plan.format);
    println!("  source: {}", app.source);
    match &plan.expected_sha256 {
        Some(_) => println!("  sha256: published, will verify"),
        None => {
            println!("  sha256: NOT PUBLISHED, download cannot be verified");
            if yes && !allow_unverified {
                bail!("refusing unverified asset without --allow-unverified");
            }
        }
    }
    if plan.format.needs_root() {
        println!("  note:   package scripts run as root");
    }
    if plan.format == Format::Flatpak {
        println!("  note:   a missing runtime is fetched from the bundle's runtime repo (usually Flathub)");
    }
    if let Some(previous) = &app.installed
        && previous.format != plan.format
    {
        println!(
            "  note:   replaces the {} install, which will be removed",
            previous.format
        );
    }
    if !yes && !confirm("Install?")? {
        println!("{id}: skipped");
        return Ok(());
    }

    let installed = update::apply(store, id, &plan, platform)?;
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
    format!("  format:     {format}\n  filter:     {filter}\n  bins:       {bins}\n  prerelease: {prerelease}")
}

fn confirm(question: &str) -> Result<bool> {
    print!("{question} [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}
