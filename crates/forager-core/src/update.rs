//! Update flow shared by every frontend: work out what would be installed,
//! then download, verify and install it.

use std::fs;
use std::path::Path;

use crate::error::{Error, Result};
use crate::matcher::select_asset;
use crate::model::{App, Asset, Format, Installed, Release};
use crate::platform::Platform;
use crate::store::{self, Store};
use crate::{install, verify, version};

/// What `apply` would install for an app.
#[derive(Debug, Clone)]
pub struct Plan {
    pub release: Release,
    pub asset: Asset,
    pub format: Format,
    /// SHA-256 the download must match, if the release publishes one.
    pub expected_sha256: Option<String>,
    /// Tag currently installed.
    pub from: Option<String>,
}

/// The update for `app`, or `None` if it is up to date.
pub fn plan(app: &App, platform: &Platform) -> Result<Option<Plan>> {
    let release = app.source.latest_release(app.prerelease)?;
    if let Some(installed) = &app.installed
        && !version::is_newer(&installed.tag, &release.tag)
    {
        return Ok(None);
    }
    let (asset, format) = select_asset(app, &release, platform)?;
    let asset = asset.clone();
    let expected_sha256 = verify::expected_sha256(&asset, &release)?;
    Ok(Some(Plan {
        from: app.installed.as_ref().map(|i| i.tag.clone()),
        release,
        asset,
        format,
        expected_sha256,
    }))
}

/// Downloads, verifies and installs `plan` for app `id`, and records the
/// install in `store`. An install in another format is removed afterwards,
/// so a failed install never leaves the app missing.
pub fn apply(store: &mut Store, id: &str, plan: &Plan, platform: &Platform) -> Result<Installed> {
    let (file, actual) = verify::download(&plan.asset, &store::download_dir())?;
    let result = install_file(store, id, plan, &file, &actual, platform);
    fs::remove_file(&file).ok();
    result
}

fn install_file(
    store: &mut Store,
    id: &str,
    plan: &Plan,
    file: &Path,
    actual: &str,
    platform: &Platform,
) -> Result<Installed> {
    if let Some(expected) = &plan.expected_sha256
        && expected != actual
    {
        return Err(Error::ChecksumMismatch {
            asset: plan.asset.name.clone(),
            expected: expected.clone(),
            actual: actual.to_string(),
        });
    }

    let previous = store.get(id)?.installed.clone();
    let installed = install::install(file, plan.format, store.get(id)?, &plan.release.tag, platform)?;
    store.get_mut(id)?.installed = Some(installed.clone());
    store.save()?;

    if let Some(previous) = previous
        && previous.format != installed.format
    {
        install::uninstall(&previous, platform).map_err(|source| Error::PreviousNotRemoved {
            format: previous.format,
            source: Box::new(source),
        })?;
    }
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Source;

    #[test]
    fn checksum_mismatch_installs_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = Store::load(&tmp.path().join("apps.toml")).unwrap();
        store
            .add(App {
                id: "tool".into(),
                source: Source::GitHub {
                    owner: "o".into(),
                    repo: "tool".into(),
                },
                asset_filter: None,
                format: None,
                prerelease: false,
                bins: Vec::new(),
                installed: None,
            })
            .unwrap();
        let plan = Plan {
            release: Release {
                tag: "v1".into(),
                name: None,
                prerelease: false,
                published_at: None,
                assets: Vec::new(),
            },
            asset: Asset {
                name: "tool.tar.gz".into(),
                url: String::new(),
                size: 0,
                digest: None,
            },
            format: Format::Tarball,
            expected_sha256: Some("a".repeat(64)),
            from: None,
        };
        let file = tmp.path().join("tool.tar.gz");
        fs::write(&file, "x").unwrap();

        let result = install_file(&mut store, "tool", &plan, &file, &"b".repeat(64), &Platform::detect());
        assert!(matches!(result, Err(Error::ChecksumMismatch { .. })));
        assert!(store.get("tool").unwrap().installed.is_none());
    }
}
