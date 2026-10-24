use semver::Version;

/// Parses a release tag leniently: `v1.2`, `release-1.2.3` and `1.2.3-rc.1`
/// all yield a version. Tags with trailing text that is not a valid semver
/// prerelease or build suffix (e.g. `1.2.3.4`) yield `None`.
pub fn parse_tag(tag: &str) -> Option<Version> {
    let start = tag.find(|c: char| c.is_ascii_digit())?;
    let s = &tag[start..];
    if let Ok(v) = Version::parse(s) {
        return Some(v);
    }

    let core_end = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let (core, rest) = s.split_at(core_end);
    let mut parts: Vec<&str> = core.trim_end_matches('.').split('.').collect();
    if parts.len() > 3 {
        return None;
    }
    parts.resize(3, "0");
    Version::parse(&format!("{}{rest}", parts.join("."))).ok()
}

/// Whether `latest` is newer than `installed`. Falls back to "tags differ"
/// when either tag is not a recognisable version.
pub fn is_newer(installed: &str, latest: &str) -> bool {
    match (parse_tag(installed), parse_tag(latest)) {
        (Some(a), Some(b)) => b > a,
        _ => installed != latest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_tags() {
        assert_eq!(parse_tag("v1.2.3"), Some(Version::new(1, 2, 3)));
        assert_eq!(parse_tag("release-1.2"), Some(Version::new(1, 2, 0)));
        assert_eq!(parse_tag("2"), Some(Version::new(2, 0, 0)));
        assert_eq!(parse_tag("v1.0.0-rc.1").unwrap().pre.as_str(), "rc.1");
        assert_eq!(parse_tag("1.2.3.4"), None);
        assert_eq!(parse_tag("nightly"), None);
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("v1.2.3", "v1.10.0"));
        assert!(!is_newer("v1.2.3", "v1.2.3"));
        assert!(!is_newer("v2.0.0", "v1.9.9"));
        assert!(is_newer("v1.0.0-rc.1", "v1.0.0"));
        assert!(is_newer("nightly-a", "nightly-b"));
    }
}
