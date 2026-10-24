use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Http(#[from] ureq::Error),
    #[error("project not found: {0}")]
    ProjectNotFound(String),
    #[error("invalid response: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid config {path}: {source}")]
    Config { path: PathBuf, source: toml::de::Error },
    #[error("cannot serialize config: {0}")]
    ConfigWrite(#[from] toml::ser::Error),
    #[error("invalid asset filter: {0}")]
    Regex(#[from] regex::Error),
    #[error("unsupported source URL: {0}")]
    UnsupportedSource(String),
    #[error("unknown forge at {0}; say which with --forge gitlab, --forge forgejo, or --forge html --pattern <regex>")]
    UnknownForge(String),
    #[error("a web page source needs a pattern matching its download links")]
    PatternNeeded,
    #[error("invalid download pattern: {0}")]
    InvalidPattern(regex::Error),
    #[error("API rate limit reached{}; set a token (GITHUB_TOKEN for GitHub) to raise the limit", reset_hint(*.reset))]
    RateLimited {
        /// Unix time at which the limit resets.
        reset: Option<u64>,
    },
    #[error("no release found")]
    NoRelease,
    #[error("no asset in release {tag} matches this system")]
    NoMatchingAsset { tag: String },
    #[error("checksum mismatch for {asset}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        asset: String,
        expected: String,
        actual: String,
    },
    #[error("no package manager found for {0} packages")]
    NoPackageManager(&'static str),
    #[error("no home directory")]
    NoHome,
    #[error("archive: {0}")]
    Archive(String),
    #[error("{} already exists and was not created by forager", .0.display())]
    LinkConflict(PathBuf),
    #[error("installed, but the previous {format} install could not be removed: {source}")]
    PreviousNotRemoved {
        format: crate::model::Format,
        source: Box<Error>,
    },
    #[error("{0} has no signature, but the app requires one")]
    SignatureMissing(String),
    #[error("signature check failed: {0}")]
    SignatureInvalid(String),
    #[error("PackageKit: {details}")]
    PackageKit { code: u32, details: String },
    #[error("D-Bus: {0}")]
    DBus(#[from] zbus::Error),
    #[error("cancelled")]
    Cancelled,
    #[error("command failed: {0}")]
    CommandFailed(String),
    #[error("invalid app id '{0}': use letters, digits, '.', '_' and '-'")]
    InvalidId(String),
    #[error("app not tracked: {0}")]
    AppNotFound(String),
    #[error("app already tracked: {0}")]
    AlreadyTracked(String),
}

fn reset_hint(reset: Option<u64>) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    match reset {
        Some(reset) if reset > now => format!(" (resets in {} min)", (reset - now).div_ceil(60)),
        _ => String::new(),
    }
}
