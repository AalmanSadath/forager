use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http: {0}")]
    Http(#[from] ureq::Error),
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
    #[error("command failed: {0}")]
    CommandFailed(String),
    #[error("app not tracked: {0}")]
    AppNotFound(String),
    #[error("app already tracked: {0}")]
    AlreadyTracked(String),
}
