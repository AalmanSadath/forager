//! Update flow shared by every frontend: work out what would be installed,
//! then download, verify and install it.

mod apply;
mod plan;
mod status;

pub use apply::{DeltaOutcome, Download, Progress, adopt, create_menu_entries, download, install};
pub use plan::{
    Delta, Plan, SignatureCheck, SummaryLine, Tone, asset_shape, changes, combined_notes, cooldown_until, now, plans,
    plans_for, repo_releases, summary,
};
pub use status::{InstallState, State, Status, asset_in, eligible, latest, prefetch, sort_installs, status, status_in};

#[cfg(test)]
mod fixtures {
    use super::*;
    use crate::model::{Asset, Format, Installed, Release};
    use crate::verify::{Algorithm, Checksum};

    pub(super) fn plan(format: Format, sha256: Option<String>) -> Plan {
        Plan {
            release: Release {
                tag: "v1".into(),
                name: None,
                prerelease: false,
                published_at: None,
                notes: None,
                assets: Vec::new(),
            },
            asset: Asset {
                name: "tool.tar.gz".into(),
                url: String::new(),
                size: 0,
                digest: None,
            },
            format,
            checksum: sha256.map(|hex| Checksum {
                algorithm: Algorithm::Sha256,
                hex,
                from: Some("SHA256SUMS".into()),
            }),
            from: None,
            missing: false,
            signature: None,
            replaces: Vec::new(),
            downgrade: false,
            delta: None,
            changes: Vec::new(),
            skip_after: None,
            asset_changed: None,
            via_repo: None,
            repo_commit: None,
            repo_file: None,
        }
    }

    pub(super) fn install(format: Format, tag: &str) -> Installed {
        Installed {
            tag: tag.into(),
            format,
            asset: String::new(),
            package: Some("tool".into()),
            path: None,
            links: Vec::new(),
            desktop: Vec::new(),
        }
    }
}
