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
    pub platform: String,
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
    pub release_notes: Option<String>,
    pub asset_url: Option<String>,
    pub checked_repo: Option<String>,
    pub message: String,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: Option<String>,
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[tauri::command]
pub fn about_info() -> AboutInfo {
    AboutInfo {
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        platform: std::env::consts::OS.to_string(),
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
            release_notes: None,
            asset_url: None,
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
    let asset_url = if has_update {
        find_platform_asset(&release.assets)
    } else {
        None
    };
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
        release_notes: release.body,
        asset_url,
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

fn find_platform_asset(assets: &[GithubAsset]) -> Option<String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let patterns: &[&str] = match (os, arch) {
        ("macos", "aarch64") => &["aarch64.dmg"],
        ("macos", _) => &["x64.dmg", "x86_64.dmg", "universal.dmg"],
        ("windows", "aarch64") => &["arm64-setup.exe", "aarch64-setup.exe"],
        ("windows", _) => &["x64-setup.exe", "x86_64-setup.exe", "x64.msi", "setup.exe"],
        ("linux", "aarch64") => &["aarch64.AppImage", "arm64.AppImage"],
        ("linux", _) => &["amd64.AppImage", "x86_64.AppImage", "x64.AppImage"],
        _ => &[],
    };

    for asset in assets {
        let lower = asset.name.to_lowercase();
        for pattern in patterns {
            if lower.ends_with(pattern) {
                return Some(asset.browser_download_url.clone());
            }
        }
    }

    // fallback: first asset matching OS extension
    let ext = match os {
        "macos" => ".dmg",
        "windows" => ".exe",
        "linux" => ".AppImage",
        _ => return None,
    };
    assets
        .iter()
        .find(|a| a.name.to_lowercase().ends_with(ext))
        .map(|a| a.browser_download_url.clone())
}

#[tauri::command]
pub async fn about_do_update(app: tauri::AppHandle, asset_url: String) -> Result<String, String> {
    use futures_util::StreamExt;
    use std::io::Write;
    use tauri::{Emitter, Manager};
    use tauri_plugin_opener::OpenerExt;

    let filename = asset_url
        .split('/')
        .last()
        .filter(|s| !s.is_empty())
        .unwrap_or("player-update")
        .to_string();

    let downloads_dir = app
        .path()
        .download_dir()
        .map_err(|e| format!("Cannot locate Downloads folder: {e}"))?;

    let dest = downloads_dir.join(&filename);

    let response = reqwest::Client::new()
        .get(&asset_url)
        .header(
            reqwest::header::USER_AGENT,
            format!("player/{}", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .await
        .map_err(|e| format!("Download failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("Server returned {}", response.status()));
    }

    let total = response.content_length();
    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    let mut file =
        std::fs::File::create(&dest).map_err(|e| format!("Create file failed: {e}"))?;

    let mut last_percent: i32 = -1;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download error: {e}"))?;
        downloaded += chunk.len() as u64;
        file.write_all(&chunk).map_err(|e| format!("Write failed: {e}"))?;

        let percent: Option<u8> = total
            .filter(|&t| t > 0)
            .map(|t| ((downloaded * 100) / t).min(100) as u8);
        let pct_now = percent.map(|p| p as i32).unwrap_or(-1);
        if pct_now != last_percent {
            last_percent = pct_now;
            let _ = app.emit(
                "about-update-progress",
                serde_json::json!({
                    "downloaded": downloaded,
                    "total": total,
                    "percent": percent
                }),
            );
        }
    }

    // On Linux, AppImage needs to be executable
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)
            .map_err(|e| format!("Stat failed: {e}"))?
            .permissions();
        perms.set_mode(perms.mode() | 0o755);
        std::fs::set_permissions(&dest, perms).map_err(|e| format!("chmod failed: {e}"))?;
    }

    let dest_str = dest.to_string_lossy().to_string();

    #[cfg(not(target_os = "android"))]
    app.opener()
        .open_path(&dest_str, None::<&str>)
        .map_err(|e| format!("Open failed: {e}"))?;

    Ok(dest_str)
}
