//! Public HTTPS package loading. Downloading never evaluates plugin code.
use super::model::{web_url, InstallSource, Manifest, Package, MAX_PACKAGE_BYTES};
use futures_util::StreamExt;
use reqwest::Url;
use serde::Serialize;
use serde_json::Value;
use std::{
    future::Future,
    time::{Duration, Instant},
};

#[derive(Debug, Serialize)]
pub struct DownloadPreview {
    pub content: String,
    pub manifest: Manifest,
    pub fingerprint: String,
    pub source: InstallSource,
}

impl DownloadPreview {
    pub fn verify_id(&self, expected: Option<&str>) -> Result<(), String> {
        if expected.is_some_and(|id| id != self.manifest.id) {
            return Err("Downloaded plugin ID does not match the selected catalog entry".into());
        }
        Ok(())
    }
}

struct Fetched {
    status: u16,
    body: String,
}

pub(super) fn download_url(input: &str) -> Result<Url, String> {
    let url = web_url(input.trim())?;
    if url.scheme() != "https" {
        return Err("Plugin downloads require an HTTPS URL".into());
    }
    Ok(url)
}

pub(super) fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!(
            "Player-Plugin-Installer/",
            env!("CARGO_PKG_VERSION")
        ))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("Too many download redirects")
            } else if download_url(attempt.url().as_str()).is_err() {
                attempt.error("Download redirect must use HTTPS without credentials")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| e.to_string())
}

pub async fn download(input: &str) -> Result<DownloadPreview, String> {
    let started = Instant::now();
    let client = client()?;
    resolve_with(input, &|url: Url| {
        let client = client.clone();
        async move {
            let remaining = Duration::from_secs(30)
                .checked_sub(started.elapsed())
                .filter(|d| !d.is_zero())
                .ok_or("Plugin download timed out")?;
            let mut request = client.get(url.clone()).timeout(remaining);
            if url.host_str() == Some("api.github.com") {
                request = request
                    .header("Accept", "application/vnd.github+json")
                    .header("X-GitHub-Api-Version", "2022-11-28");
            }
            let response = request
                .send()
                .await
                .map_err(|e| format!("Could not download plugin: {}", e.without_url()))?;
            if response
                .content_length()
                .is_some_and(|n| n > MAX_PACKAGE_BYTES as u64)
            {
                return Err("Download exceeds 4 MiB".into());
            }
            let status = response.status().as_u16();
            let mut bytes = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| format!("Download failed: {}", e.without_url()))?;
                if bytes.len() + chunk.len() > MAX_PACKAGE_BYTES {
                    return Err("Download exceeds 4 MiB".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(Fetched {
                status,
                body: String::from_utf8(bytes).map_err(|_| "Plugin files must be UTF-8")?,
            })
        }
    })
    .await
}

struct Resolver<'a, F> {
    fetch: &'a F,
    requests: usize,
    bytes: usize,
}
impl<F, Fut> Resolver<'_, F>
where
    F: Fn(Url) -> Fut + Sync,
    Fut: Future<Output = Result<Fetched, String>> + Send,
{
    async fn get(&mut self, url: Url) -> Result<Fetched, String> {
        download_url(url.as_str())?;
        self.requests += 1;
        if self.requests > 24 {
            return Err("Too many requests while resolving repository; use a direct plugin.json or package link".into());
        }
        let response = (self.fetch)(url).await?;
        self.bytes += response.body.len();
        if response.body.len() > MAX_PACKAGE_BYTES || self.bytes > MAX_PACKAGE_BYTES * 3 {
            return Err("Plugin download size limit exceeded".into());
        }
        Ok(response)
    }
    async fn text(&mut self, url: Url) -> Result<String, String> {
        let response = self.get(url).await?;
        if response.status != 200 {
            return Err(http_error(&response));
        }
        Ok(response.body)
    }
    async fn revision(
        &mut self,
        owner: &str,
        repo: &str,
        reference: &str,
    ) -> Result<Option<String>, String> {
        let mut url = github_api(owner, repo, &["commits"]);
        url.query_pairs_mut()
            .append_pair("sha", reference)
            .append_pair("per_page", "1");
        let response = self.get(url).await?;
        if matches!(response.status, 404 | 409 | 422) {
            return Ok(None);
        }
        if response.status != 200 {
            return Err(http_error(&response));
        }
        let data: Value =
            serde_json::from_str(&response.body).map_err(|_| "Invalid GitHub commit response")?;
        let sha = data
            .get(0)
            .and_then(|v| v.get("sha"))
            .and_then(Value::as_str)
            .ok_or("GitHub did not return a commit")?;
        if !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid GitHub commit id".into());
        }
        Ok(Some(sha.into()))
    }
    async fn package_at(&mut self, url: Url) -> Result<Package, String> {
        let text = self.text(url.clone()).await?;
        let json: Value = serde_json::from_str(&text)
            .map_err(|_| "Link did not return a JSON plugin package or manifest")?;
        if json.get("manifest").is_some() {
            return Package::parse(&text);
        }
        if !url.path().ends_with("/plugin.json") {
            return Err(
                "Link must point to a .player-plugin package, plugin.json, or a GitHub repository"
                    .into(),
            );
        }
        Err("Native installation requires a built .player-plugin package, not source files".into())
    }
}

fn github_api(owner: &str, repo: &str, tail: &[&str]) -> Url {
    let mut url = Url::parse("https://api.github.com").unwrap();
    {
        let mut path = url.path_segments_mut().unwrap();
        path.push("repos").push(owner).push(repo);
        for part in tail {
            path.push(part);
        }
    }
    url
}

fn http_error(response: &Fetched) -> String {
    match response.status {
        403|429=>"Download was refused or the public GitHub API rate limit was reached. Try a direct package or raw .player-plugin link.".into(),
        404=>"Plugin files were not found. Use a public repository directory containing plugin.player-plugin, or a direct .player-plugin link.".into(),
        status=>format!("Plugin download returned HTTP {status}"),
    }
}

async fn resolve_with<F, Fut>(input: &str, fetch: &F) -> Result<DownloadPreview, String>
where
    F: Fn(Url) -> Fut + Sync,
    Fut: Future<Output = Result<Fetched, String>> + Send,
{
    let mut input = download_url(input)?;
    input.set_fragment(None);
    let mut resolver = Resolver {
        fetch,
        requests: 0,
        bytes: 0,
    };
    let mut source = InstallSource {
        url: input.to_string(),
        repository: None,
        revision: None,
    };
    let parts: Vec<String> = input
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .map(|s| {
            percent_encoding::percent_decode_str(s)
                .decode_utf8()
                .map(|s| s.into_owned())
                .map_err(|_| "Invalid URL path".to_string())
        })
        .collect::<Result<_, _>>()?;
    let github = matches!(input.host_str(), Some("github.com" | "www.github.com"))
        && parts.len() >= 2
        && (parts.len() == 2 || matches!(parts.get(2).map(String::as_str), Some("tree" | "blob")));
    let package_url = if github {
        let owner = &parts[0];
        let repo = parts[1].strip_suffix(".git").unwrap_or(&parts[1]);
        if owner.is_empty()
            || repo.is_empty()
            || [owner.as_str(), repo].iter().any(|s| {
                !s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            })
        {
            return Err("Invalid GitHub repository URL".into());
        }
        let (revision, path) = if parts.len() == 2 {
            let explicit = input
                .query_pairs()
                .find(|(k, _)| k == "ref")
                .map(|(_, v)| v.into_owned());
            let reference = if let Some(reference) = explicit {
                reference
            } else {
                let text = resolver.text(github_api(owner, repo, &[])).await?;
                let data: Value = serde_json::from_str(&text)
                    .map_err(|_| "Invalid GitHub repository response")?;
                data["default_branch"]
                    .as_str()
                    .ok_or("Repository has no default branch")?
                    .into()
            };
            (
                resolver
                    .revision(owner, repo, &reference)
                    .await?
                    .ok_or("Public repository or branch not found")?,
                Vec::new(),
            )
        } else {
            let tail = &parts[3..];
            if tail.is_empty() || tail.len() > 16 {
                return Err("Invalid GitHub tree/blob URL".into());
            }
            // A branch can contain '/'. Resolve the longest valid ref, not just the first segment.
            let mut resolved = None;
            for split in (1..=tail.len()).rev() {
                if let Some(revision) = resolver
                    .revision(owner, repo, &tail[..split].join("/"))
                    .await?
                {
                    resolved = Some((revision, tail[split..].to_vec()));
                    break;
                }
            }
            resolved.ok_or("Public repository branch or commit not found")?
        };
        let mut url = Url::parse("https://raw.githubusercontent.com").unwrap();
        {
            let mut segments = url.path_segments_mut().unwrap();
            segments.push(owner).push(repo).push(&revision);
            for part in &path {
                segments.push(part);
            }
            if parts.get(2).map(String::as_str) != Some("blob") {
                segments.push("plugin.player-plugin");
            }
        }
        source.repository = Some(format!("https://github.com/{owner}/{repo}"));
        source.revision = Some(revision);
        url
    } else if input.path().ends_with('/') {
        input
            .join("plugin.player-plugin")
            .map_err(|e| e.to_string())?
    } else {
        input
    };
    let package = resolver.package_at(package_url).await?;
    let content = serde_json::to_string(&package).map_err(|e| e.to_string())?;
    // Validate the complete package before showing its native-code approval.
    let package = Package::parse(&content)?;
    source.validate()?;
    Ok(DownloadPreview {
        fingerprint: package.fingerprint(),
        manifest: package.manifest,
        content,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn content() -> String {
        serde_json::to_string(&Package {
            manifest: serde_json::from_str(include_str!(
                "../../../plugins/examples/hello/plugin.json"
            ))
            .unwrap(),
            native: super::super::model::fixture_native(),
        })
        .unwrap()
    }
    fn response(status: u16, body: impl Into<String>) -> Result<Fetched, String> {
        Ok(Fetched {
            status,
            body: body.into(),
        })
    }
    fn run<F, Fut>(url: &str, fetch: &F) -> Result<DownloadPreview, String>
    where
        F: Fn(Url) -> Fut + Sync,
        Fut: Future<Output = Result<Fetched, String>> + Send,
    {
        tauri::async_runtime::block_on(resolve_with(url, fetch))
    }
    #[test]
    fn downloads_validate_native_hash_and_expected_identity() {
        let preview = run("https://example.org/hello.player-plugin", &|_| {
            std::future::ready(response(200, content()))
        })
        .unwrap();
        assert!(preview.verify_id(Some("example.hello")).is_ok());
        assert!(preview.verify_id(Some("other.plugin")).is_err());
        let broken = content().replace("ca978112", "00000000");
        assert!(run("https://example.org/hello.player-plugin", &|_| {
            std::future::ready(response(200, broken.clone()))
        })
        .is_err());
    }
    #[test]
    fn github_root_pins_native_package_to_commit() {
        let sha = "a".repeat(40);
        let seen = std::sync::Mutex::new(vec![]);
        let preview = run("https://github.com/owner/plugin", &|url: Url| {
            seen.lock().unwrap().push(url.to_string());
            let result = if url.path().ends_with("/commits") {
                response(200, serde_json::json!([{"sha":sha}]).to_string())
            } else if url.host_str() == Some("api.github.com") {
                response(200, r#"{"default_branch":"main"}"#)
            } else {
                assert!(url
                    .path()
                    .ends_with(&format!("/{sha}/plugin.player-plugin")));
                response(200, content())
            };
            std::future::ready(result)
        })
        .unwrap();
        assert_eq!(preview.source.revision, Some(sha));
        assert_eq!(seen.lock().unwrap().len(), 3);
    }
    #[test]
    fn resolves_branch_with_slashes_and_native_subdirectory() {
        let sha = "b".repeat(40);
        let preview = run(
            "https://github.com/owner/plugin/tree/feature/audio/examples/hello",
            &|url: Url| {
                let result = if url.host_str() == Some("api.github.com") {
                    if url
                        .query_pairs()
                        .any(|(k, v)| k == "sha" && v == "feature/audio")
                    {
                        response(200, serde_json::json!([{"sha":sha}]).to_string())
                    } else {
                        response(404, "")
                    }
                } else {
                    assert!(url
                        .path()
                        .ends_with(&format!("/{sha}/examples/hello/plugin.player-plugin")));
                    response(200, content())
                };
                std::future::ready(result)
            },
        )
        .unwrap();
        assert_eq!(preview.manifest.id, "example.hello");
    }
    #[test]
    fn rejects_script_packages_and_unbuilt_manifests() {
        for body in [
            serde_json::json!({"manifest":{},"script":"legacy"}).to_string(),
            include_str!("../../../plugins/examples/hello/plugin.json").into(),
        ] {
            assert!(
                run("https://example.org/plugin.json", &|_| std::future::ready(
                    response(200, body.clone())
                ))
                .is_err()
            );
        }
    }
    #[test]
    fn rejects_insecure_urls_and_network_errors() {
        for url in [
            "http://example.org/plugin",
            "file:///tmp/plugin",
            "https://user:pass@example.org/plugin",
        ] {
            assert!(download_url(url).is_err());
        }
        for status in [404, 403, 429] {
            assert!(run("https://example.org/plugin.player-plugin", &|_| {
                std::future::ready(response(status, ""))
            })
            .is_err());
        }
    }
    #[test]
    fn rejects_oversized_downloads_before_parsing() {
        assert!(run("https://example.org/plugin.player-plugin", &|_| {
            std::future::ready(response(200, "x".repeat(MAX_PACKAGE_BYTES + 1)))
        })
        .is_err());
    }
}
