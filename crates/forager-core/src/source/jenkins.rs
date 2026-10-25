//! Jenkins jobs: each successful build with archived files is a release,
//! tagged `build-<number>`.

use serde::Deserialize;

use crate::error::Result;
use crate::http;
use crate::model::{Asset, Release};

#[derive(Deserialize)]
struct Job {
    #[serde(default)]
    builds: Vec<Build>,
}

#[derive(Deserialize)]
struct Build {
    number: u64,
    result: Option<String>,
    /// Milliseconds since the Unix epoch.
    timestamp: Option<u64>,
    url: String,
    #[serde(default)]
    artifacts: Vec<Artifact>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Artifact {
    file_name: String,
    relative_path: String,
}

/// `job` is the job's address, ending in `/`.
pub fn releases(job: &str) -> Result<Vec<Release>> {
    // `tree=builds[number,…,artifacts[fileName,relativePath]]{0,10}`, encoded.
    let url = format!(
        "{job}api/json?tree=builds%5Bnumber,result,timestamp,url,artifacts%5BfileName,relativePath%5D%5D%7B0,10%7D"
    );
    let key: String = job
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let body = http::get_cached(
        http::get(&url).header("Accept", "application/json"),
        &format!("jenkins/{key}.json"),
    )?;
    parse(&body)
}

pub(super) fn parse(body: &str) -> Result<Vec<Release>> {
    let job: Job = serde_json::from_str(body)?;
    Ok(job
        .builds
        .into_iter()
        .filter(|b| b.result.as_deref() == Some("SUCCESS") && !b.artifacts.is_empty())
        .map(|b| {
            let base = if b.url.ends_with('/') {
                b.url.clone()
            } else {
                format!("{}/", b.url)
            };
            Release {
                tag: format!("build-{}", b.number),
                name: Some(format!("#{}", b.number)),
                prerelease: false,
                published_at: b.timestamp.map(|ms| super::iso_from_unix(ms / 1000)),
                notes: None,
                assets: b
                    .artifacts
                    .into_iter()
                    .map(|a| Asset {
                        url: format!("{base}artifact/{}", a.relative_path.replace(' ', "%20")),
                        name: a.file_name,
                        size: 0,
                        digest: None,
                    })
                    .collect(),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_builds_with_files_are_releases() {
        let body = r#"{"_class": "hudson.model.FreeStyleProject", "builds": [
            {"number": 43, "result": null, "timestamp": 1792918219000, "url": "https://ci.example.org/job/tool/43/", "artifacts": []},
            {"number": 42, "result": "SUCCESS", "timestamp": 1792918219000, "url": "https://ci.example.org/job/tool/42/",
             "artifacts": [{"fileName": "tool-x86_64.AppImage", "relativePath": "out/tool-x86_64.AppImage"}]},
            {"number": 41, "result": "FAILURE", "timestamp": 1, "url": "https://ci.example.org/job/tool/41/",
             "artifacts": [{"fileName": "a", "relativePath": "a"}]}]}"#;
        let releases = parse(body).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].tag, "build-42");
        assert_eq!(
            releases[0].assets[0].url,
            "https://ci.example.org/job/tool/42/artifact/out/tool-x86_64.AppImage"
        );
        assert_eq!(releases[0].published_at.as_deref(), Some("2026-10-25T08:50:19Z"));
    }
}
