use reqwest::header::{ACCEPT, USER_AGENT};
use serde::Deserialize;
use std::cmp::Ordering;
use std::time::Duration;

#[derive(Clone, Copy)]
pub struct EmbeddedChangelogEntry {
    pub subject: &'static str,
    pub short_hash: &'static str,
    pub committed_at: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/about_build.rs"));

#[derive(serde::Serialize)]
pub struct AboutInfo {
    pub current_version: String,
    pub build_commit: Option<String>,
    pub release_repo: Option<String>,
    pub changelog: Vec<AboutChangelogEntry>,
}

#[derive(serde::Serialize)]
pub struct AboutChangelogEntry {
    pub subject: String,
    pub short_hash: String,
    pub committed_at: String,
}

#[derive(serde::Serialize)]
pub struct AboutUpdateStatus {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub has_update: bool,
    pub release_url: Option<String>,
    pub checked_repo: Option<String>,
    pub message: String,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: Option<String>,
}

#[tauri::command]
pub fn about_info() -> AboutInfo {
    AboutInfo {
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        build_commit: BUILD_GIT_COMMIT.map(str::to_string),
        release_repo: BUILD_GITHUB_REPOSITORY.map(str::to_string),
        changelog: BUILD_CHANGELOG
            .iter()
            .map(|entry| AboutChangelogEntry {
                subject: entry.subject.to_string(),
                short_hash: entry.short_hash.to_string(),
                committed_at: entry.committed_at.to_string(),
            })
            .collect(),
    }
}

#[tauri::command]
pub fn about_check_updates() -> Result<AboutUpdateStatus, String> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let checked_repo = BUILD_GITHUB_REPOSITORY.map(str::to_string);
    let Some(repo_slug) = BUILD_GITHUB_REPOSITORY else {
        return Ok(AboutUpdateStatus {
            current_version,
            latest_version: None,
            has_update: false,
            release_url: None,
            checked_repo,
            message: "GitHub repository was not detected at build time.".to_string(),
        });
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|err| format!("build http client: {err}"))?;
    let url = format!("https://api.github.com/repos/{repo_slug}/releases/latest");

    let response = client
        .get(url)
        .header(USER_AGENT, format!("player/{}", env!("CARGO_PKG_VERSION")))
        .header(ACCEPT, "application/vnd.github+json")
        .send()
        .map_err(|err| format!("request latest release: {err}"))?;

    if !response.status().is_success() {
        return Err(format!("GitHub releases returned {}", response.status()));
    }

    let release: GithubRelease = response
        .json()
        .map_err(|err| format!("decode latest release: {err}"))?;
    let latest_version = normalize_version_tag(&release.tag_name);
    let has_update = compare_versions(&latest_version, &current_version) == Ordering::Greater;
    let message = if has_update {
        format!("Update available: {latest_version}")
    } else {
        "You already have the latest release.".to_string()
    };

    Ok(AboutUpdateStatus {
        current_version,
        latest_version: Some(latest_version),
        has_update,
        release_url: release.html_url,
        checked_repo,
        message,
    })
}

fn normalize_version_tag(value: &str) -> String {
    value.trim().trim_start_matches(['v', 'V']).to_string()
}

fn compare_versions(left: &str, right: &str) -> Ordering {
    let left_parts = parse_version_parts(left);
    let right_parts = parse_version_parts(right);
    let max_len = left_parts.len().max(right_parts.len());

    for index in 0..max_len {
        let lhs = *left_parts.get(index).unwrap_or(&0);
        let rhs = *right_parts.get(index).unwrap_or(&0);
        match lhs.cmp(&rhs) {
            Ordering::Equal => continue,
            non_equal => return non_equal,
        }
    }

    Ordering::Equal
}

fn parse_version_parts(value: &str) -> Vec<u64> {
    let cleaned = normalize_version_tag(value);
    let core = cleaned.split_once('+').map_or(cleaned.as_str(), |(head, _)| head);
    let core = core.split_once('-').map_or(core, |(head, _)| head);
    let mut parts: Vec<u64> = core
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect();

    if parts.is_empty() {
        parts.push(0);
    }

    parts
}