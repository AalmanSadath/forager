//! Moving apps between machines and people: an export file of the tracked
//! apps and their settings, and `forager://add?…` links that open the Add
//! page for one app.
//!
//! Neither carries installs. A link never carries a signing key: a key is
//! only as trustworthy as where it came from, and a link can come from
//! anywhere. An export file is the user's own, so it keeps the keys.

use std::fs;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{Error, Result};
use crate::model::{App, Format, Signature};
use crate::signature;
use crate::source::{Forge, Source};
use crate::store::{self, Store};

/// An app's settings as exported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportedApp {
    pub id: String,
    pub source: Source,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub versions: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cooldown_days: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub prerelease: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bins: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub track_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<ExportedSignature>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// A signing key travels with the app; a GPG keyring as base64.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ExportedSignature {
    Minisign { public_key: String },
    Gpg { keyring: String },
    Attestation,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExportFile {
    #[serde(default)]
    apps: Vec<ExportedApp>,
}

const HEADER: &str = "# Apps tracked by Forager, from `forager export`.\n\
                      # Track them on another machine with `forager import <file>`.\n\n";

/// Every tracked app's settings, as TOML.
pub fn export(store: &Store) -> Result<String> {
    let apps = store.apps.iter().map(exported).collect::<Result<Vec<_>>>()?;
    let text = toml::to_string(&ExportFile { apps })?;
    Ok(format!("{HEADER}{text}"))
}

fn exported(app: &App) -> Result<ExportedApp> {
    let signature = match &app.signature {
        None => None,
        Some(Signature::Minisign { public_key }) => Some(ExportedSignature::Minisign {
            public_key: public_key.clone(),
        }),
        Some(Signature::Gpg { keyring }) => Some(ExportedSignature::Gpg {
            keyring: STANDARD.encode(fs::read(keyring)?),
        }),
        Some(Signature::Attestation) => Some(ExportedSignature::Attestation),
    };
    Ok(ExportedApp {
        id: app.id.clone(),
        source: app.source.clone(),
        asset_filter: app.asset_filter.clone(),
        tag_filter: app.tag_filter.clone(),
        versions: app.versions.clone(),
        cooldown_days: app.cooldown_days,
        format: app.format,
        prerelease: app.prerelease,
        bins: app.bins.clone(),
        track_only: app.track_only,
        seen: app.seen.clone(),
        signature,
    })
}

/// Adds the apps of an export file to `store`, each one that is valid and
/// not tracked yet (by id or by source); the caller saves the store.
/// Returns each app's id and whether it was added.
pub fn import(store: &mut Store, text: &str) -> Result<Vec<(String, Result<()>)>> {
    let file: ExportFile = toml::from_str(text).map_err(|e| Error::InvalidExport(e.to_string()))?;
    Ok(file
        .apps
        .into_iter()
        .map(|exported| {
            let id = exported.id.clone();
            let result = imported(store, exported).and_then(|app| store.add(app));
            (id, result)
        })
        .collect())
}

fn imported(store: &Store, exported: ExportedApp) -> Result<App> {
    let mut app = App::new(exported.source, exported.id);
    app.asset_filter = exported.asset_filter;
    app.tag_filter = exported.tag_filter;
    app.versions = exported.versions;
    app.cooldown_days = exported.cooldown_days;
    app.format = exported.format;
    app.prerelease = exported.prerelease;
    app.bins = exported.bins;
    app.track_only = exported.track_only;
    app.seen = exported.seen;
    app.validate()?;
    if store.apps.iter().any(|a| a.id == app.id || a.source == app.source) {
        return Err(Error::AlreadyTracked(app.id));
    }
    // Last, since a GPG key is stored on disk under the app's id.
    app.signature = match exported.signature {
        None => None,
        Some(ExportedSignature::Minisign { public_key }) => Some(Signature::Minisign {
            public_key: signature::parse_minisign_key(&public_key)?,
        }),
        Some(ExportedSignature::Gpg { keyring }) => {
            let bytes = STANDARD
                .decode(keyring.trim())
                .map_err(|e| Error::SignatureInvalid(format!("GPG key of {}: {e}", app.id)))?;
            let dir = store::cache_dir();
            fs::create_dir_all(&dir)?;
            let file = tempfile::NamedTempFile::new_in(dir)?;
            fs::write(file.path(), bytes)?;
            Some(Signature::Gpg {
                keyring: signature::store_gpg_key(&app.id, file.path())?,
            })
        }
        Some(ExportedSignature::Attestation) => Some(signature::resolve(
            &signature::SignatureInput::Attestation,
            &app.id,
            &app.source,
        )?),
    };
    Ok(app)
}

/// What a `forager://add?…` link asks to add: an address, as typed on the
/// Add page, and the settings to start from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddRequest {
    pub url: String,
    /// The kind of source, when the address alone does not say.
    pub forge: Option<String>,
    /// Download link pattern of a web page.
    pub pattern: Option<String>,
    pub id: Option<String>,
    pub asset_filter: Option<String>,
    pub tag_filter: Option<String>,
    pub versions: Option<String>,
    pub cooldown_days: u32,
    pub format: Option<Format>,
    pub bins: Vec<String>,
    pub prerelease: bool,
    pub track_only: bool,
}

impl AddRequest {
    /// The source the request names.
    pub fn source(&self) -> Result<Source> {
        let forge = self
            .forge
            .as_deref()
            .map(|f| f.parse::<Forge>().map_err(|_| Error::UnsupportedSource(f.to_string())))
            .transpose()?;
        match (forge, &self.pattern) {
            (Some(Forge::Html) | None, Some(pattern)) => Source::html(&self.url, pattern),
            (forge, _) => Source::from_url(&self.url, forge),
        }
    }
}

/// A link that opens Forager's Add page for `app`, with its settings.
pub fn link(app: &App) -> String {
    let mut url = Url::parse("forager://add").expect("a valid base");
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("url", &app.source.web_url());
        match &app.source {
            // Recognised from the address alone.
            Source::GitHub { .. } | Source::SourceForge { .. } | Source::SourceHut { .. } => {}
            Source::Html { pattern, .. } => {
                query.append_pair("forge", "html");
                query.append_pair("pattern", pattern);
            }
            source => {
                query.append_pair("forge", source.forge());
            }
        }
        if app.id != app.source.default_id() {
            query.append_pair("id", &app.id);
        }
        let optional = [
            ("filter", app.asset_filter.clone()),
            ("tag_filter", app.tag_filter.clone()),
            ("versions", app.versions.clone()),
            (
                "cooldown",
                (app.cooldown_days > 0).then(|| app.cooldown_days.to_string()),
            ),
            ("format", app.format.map(|f| f.to_string())),
        ];
        for (key, value) in optional {
            if let Some(value) = value {
                query.append_pair(key, &value);
            }
        }
        for bin in &app.bins {
            query.append_pair("bin", bin);
        }
        if app.prerelease {
            query.append_pair("prerelease", "1");
        }
        if app.track_only {
            query.append_pair("track_only", "1");
        }
    }
    url.to_string()
}

/// The web page "Add to Forager" badges link to. Sites like GitHub drop
/// `forager://` links, so it takes the same query and hands it on.
pub const WEB_ADD: &str = "https://aalmansadath.github.io/forager/add/";

/// The badge image, served next to that page.
pub const BADGE_IMAGE: &str = "https://aalmansadath.github.io/forager/badge.svg";

/// `link(app)` through the web page, for places that only allow https.
pub fn web_link(app: &App) -> String {
    let link = link(app);
    let query = link.split_once('?').map_or("", |(_, query)| query);
    format!("{WEB_ADD}?{query}")
}

/// Markdown for an "Add to Forager" badge, for a project's README.
pub fn badge(app: &App) -> String {
    format!("[![Add to Forager]({BADGE_IMAGE})]({})", web_link(app))
}

/// Reads a `forager://add?…` link, or the same through the web page.
/// Settings it does not know are ignored, so links from newer versions
/// still open.
pub fn parse_link(link: &str) -> Result<AddRequest> {
    let invalid = || Error::UnsupportedSource(link.to_string());
    let url = Url::parse(link.trim()).map_err(|_| invalid())?;
    // `forager://add?…` has `add` as its host; `forager:add?…` as its path.
    let action = url.host_str().unwrap_or_else(|| url.path().trim_matches('/'));
    let web = Url::parse(WEB_ADD).expect("a valid address");
    let is_web = url.origin() == web.origin() && url.path().trim_end_matches('/') == web.path().trim_end_matches('/');
    if !is_web && (url.scheme() != "forager" || action != "add") {
        return Err(invalid());
    }
    let mut request = AddRequest::default();
    let flag = |v: &str| matches!(v, "1" | "true" | "yes");
    for (key, value) in url.query_pairs() {
        let value = value.into_owned();
        match key.as_ref() {
            "url" => request.url = value,
            "forge" => request.forge = Some(value),
            "pattern" => request.pattern = Some(value),
            "id" => request.id = Some(value),
            "filter" => request.asset_filter = Some(value),
            "tag_filter" => request.tag_filter = Some(value),
            "versions" => request.versions = Some(value),
            "cooldown" => request.cooldown_days = value.parse().unwrap_or(0),
            "format" => request.format = value.parse().ok(),
            "bin" => request.bins.push(value),
            "prerelease" => request.prerelease = flag(&value),
            "track_only" => request.track_only = flag(&value),
            _ => {}
        }
    }
    if request.url.is_empty() {
        return Err(invalid());
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with(apps: Vec<App>) -> Store {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(&dir.path().join("apps.toml")).unwrap();
        for app in apps {
            store.add(app).unwrap();
        }
        store
    }

    fn github(repo: &str) -> App {
        App::new(
            Source::GitHub {
                owner: "o".into(),
                repo: repo.into(),
            },
            repo.into(),
        )
    }

    #[test]
    fn exports_settings_and_imports_them_elsewhere() {
        let mut tool = github("tool");
        tool.asset_filter = Some("musl".into());
        tool.versions = Some("<3".into());
        tool.cooldown_days = 2;
        tool.bins = vec!["bin/tool".into()];
        tool.skipped = Some("v2".into());
        tool.signature = Some(Signature::Attestation);
        tool.installed = Some(crate::model::Installed {
            tag: "v1".into(),
            format: Format::Tarball,
            asset: "tool.tar.gz".into(),
            package: None,
            path: Some("/home/me/.local/opt/tool".into()),
            links: Vec::new(),
            desktop: Vec::new(),
        });
        let mut watched = github("watched");
        watched.track_only = true;
        watched.seen = Some("v3".into());

        let text = export(&store_with(vec![tool, watched])).unwrap();
        assert!(text.starts_with("# Apps tracked by Forager"));
        assert!(!text.contains("installed") && !text.contains("skipped") && !text.contains("/home/me"));

        let mut other = store_with(vec![github("watched")]);
        let results = import(&mut other, &text).unwrap();
        assert!(results[0].1.is_ok());
        assert!(matches!(results[1].1, Err(Error::AlreadyTracked(_))));
        let tool = other.get("tool").unwrap();
        assert_eq!(tool.asset_filter.as_deref(), Some("musl"));
        assert_eq!(tool.bins, ["bin/tool"]);
        assert_eq!(tool.versions.as_deref(), Some("<3"));
        assert_eq!(tool.cooldown_days, 2);
        assert_eq!(tool.signature, Some(Signature::Attestation));
        assert!(tool.installed.is_none() && tool.skipped.is_none());
    }

    #[test]
    fn links_round_trip() {
        let mut app = App::new(
            Source::html("https://nodejs.org/dist/latest/", r"linux-x64\.tar\.xz$").unwrap(),
            "node".into(),
        );
        app.bins = vec!["bin/node".into(), "bin/npm".into()];
        app.format = Some(Format::Tarball);
        app.versions = Some(">=20, <21".into());
        app.cooldown_days = 3;
        app.track_only = true;
        let text = link(&app);
        assert!(text.starts_with("forager://add?url=https%3A%2F%2Fnodejs.org%2Fdist%2Flatest%2F&forge=html&pattern="));
        let request = parse_link(&text).unwrap();
        assert_eq!(request.id.as_deref(), Some("node"));
        assert_eq!(request.bins, ["bin/node", "bin/npm"]);
        assert_eq!(request.format, Some(Format::Tarball));
        assert_eq!(request.versions.as_deref(), Some(">=20, <21"));
        assert_eq!(request.cooldown_days, 3);
        assert!(request.track_only && !request.prerelease);
        assert_eq!(request.source().unwrap(), app.source);

        let plain = parse_link("forager://add?url=https://github.com/o/tool&unknown=1").unwrap();
        assert_eq!(plain.source().unwrap(), github("tool").source);
        assert_eq!(
            link(&github("tool")),
            "forager://add?url=https%3A%2F%2Fgithub.com%2Fo%2Ftool"
        );
        let web = web_link(&github("tool"));
        assert_eq!(
            web,
            "https://aalmansadath.github.io/forager/add/?url=https%3A%2F%2Fgithub.com%2Fo%2Ftool"
        );
        assert_eq!(parse_link(&web).unwrap().url, "https://github.com/o/tool");
        let request = parse_link(&web_link(&app)).unwrap();
        assert_eq!(request.source().unwrap(), app.source);
        assert!(badge(&github("tool")).starts_with("[![Add to Forager](https://aalmansadath.github.io/forager/badge.svg)](https://aalmansadath.github.io/forager/add/?url="));
        assert!(parse_link("https://aalmansadath.github.io/forager/other/?url=x").is_err());
        assert!(parse_link("forager://remove?url=x").is_err());
        assert!(parse_link("https://example.org/?url=x").is_err());
        assert!(parse_link("forager://add?id=x").is_err());
    }
}
