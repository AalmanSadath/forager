use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use forager_core::matcher::select_asset;
use forager_core::model::{App, Format};
use forager_core::platform::{self, Platform};
use forager_core::source::Source;
use forager_core::store::{self, Store};
use forager_core::{install, verify, version};

#[derive(Parser)]
#[command(name = "forager", version, about = "Install and update apps from their GitHub releases")]
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
    /// List tracked apps.
    List,
    /// Check tracked apps for new releases.
    Check,
    /// Install new releases of tracked apps (all apps if none are given).
    Update {
        ids: Vec<String>,
        /// Do not ask for confirmation.
        #[arg(short, long)]
        yes: bool,
        /// Install assets that have no published checksum.
        #[arg(long)]
        allow_unverified: bool,
    },
    /// Stop tracking an app and uninstall it.
    Remove {
        id: String,
        /// Leave the installed package in place.
        #[arg(long)]
        keep_installed: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let path = cli.config.unwrap_or_else(Store::default_path);
    let mut store = Store::load(&path)?;

    match cli.command {
        Command::Add {
            url,
            id,
            filter,
            format,
            prerelease,
            bins,
        } => {
            let source = Source::from_url(&url)?;
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
        Command::Check => {
            let platform = Platform::detect();
            for app in &store.apps {
                match app.source.latest_release(app.prerelease) {
                    Ok(release) => {
                        let status = match &app.installed {
                            Some(i) if version::is_newer(&i.tag, &release.tag) => {
                                format!("{} -> {}", i.tag, release.tag)
                            }
                            Some(i) => format!("{} (up to date)", i.tag),
                            None => format!("{} available, not installed", release.tag),
                        };
                        let asset = match select_asset(app, &release, &platform) {
                            Ok((asset, _)) => asset.name.clone(),
                            Err(e) => e.to_string(),
                        };
                        println!("{:<20} {:<30} {}", app.id, status, asset);
                    }
                    Err(e) => println!("{:<20} error: {e}", app.id),
                }
            }
        }
        Command::Update {
            ids,
            yes,
            allow_unverified,
        } => update(&mut store, &ids, yes, allow_unverified)?,
        Command::Remove { id, keep_installed } => {
            let app = store.get(&id)?;
            if let (Some(installed), false) = (&app.installed, keep_installed) {
                install::uninstall(installed, &Platform::detect())?;
            }
            store.remove(&id)?;
            store.save()?;
            println!("Removed {id}.");
        }
    }
    Ok(())
}

fn update(store: &mut Store, ids: &[String], yes: bool, allow_unverified: bool) -> Result<()> {
    let platform = Platform::detect();
    let ids: Vec<String> = if ids.is_empty() {
        store.apps.iter().map(|a| a.id.clone()).collect()
    } else {
        ids.to_vec()
    };

    let mut failed = 0;
    for id in &ids {
        if let Err(e) = update_one(store, id, &platform, yes, allow_unverified) {
            eprintln!("{id}: {e:#}");
            failed += 1;
        }
    }
    if failed > 0 {
        bail!("{failed} of {} app(s) failed to update", ids.len());
    }
    Ok(())
}

fn update_one(store: &mut Store, id: &str, platform: &Platform, yes: bool, allow_unverified: bool) -> Result<()> {
    let app = store.get(id)?;
    let release = app.source.latest_release(app.prerelease)?;
    if let Some(installed) = &app.installed
        && !version::is_newer(&installed.tag, &release.tag)
    {
        println!("{id}: {} is up to date", installed.tag);
        return Ok(());
    }

    let (asset, format) = select_asset(app, &release, platform)?;
    let expected = verify::expected_sha256(asset, &release)?;
    let from = app.installed.as_ref().map_or("not installed".to_string(), |i| i.tag.clone());
    println!("{id}: {from} -> {}", release.tag);
    println!("  asset:  {} ({:.1} MiB, {format})", asset.name, asset.size as f64 / 1048576.0);
    println!("  source: {}", app.source);
    match &expected {
        Some(_) => println!("  sha256: published, will verify"),
        None => {
            println!("  sha256: NOT PUBLISHED, download cannot be verified");
            if yes && !allow_unverified {
                bail!("refusing unverified asset without --allow-unverified");
            }
        }
    }
    if matches!(format, Format::Deb | Format::Rpm) {
        println!("  note:   package scripts run as root");
    }
    if !yes && !confirm("Install?")? {
        println!("{id}: skipped");
        return Ok(());
    }

    let (file, actual) = verify::download(asset, &store::download_dir())?;
    if let Some(expected) = expected
        && expected != actual
    {
        std::fs::remove_file(&file).ok();
        return Err(forager_core::Error::ChecksumMismatch {
            asset: asset.name.clone(),
            expected,
            actual,
        }
        .into());
    }

    let installed = install::install(&file, format, store.get(id)?, &release.tag, platform)?;
    std::fs::remove_file(&file).ok();
    let links = installed.links.clone();
    store.get_mut(id)?.installed = Some(installed);
    store.save()?;
    println!("{id}: installed {}", release.tag);
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

fn confirm(question: &str) -> Result<bool> {
    print!("{question} [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}
