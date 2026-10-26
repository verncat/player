//! Discovery metadata only. Installation still downloads and reviews a real package.
use super::model::{Author, Manifest, Package};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, path::Path, sync::Mutex, time::Duration};

const DEFAULT_URL: &str =
    "https://raw.githubusercontent.com/verncat/player/main/plugins/catalog.json";
const EMBEDDED: &str = include_str!("../../../plugins/catalog.json");
const MAX_BYTES: usize = 512 * 1024;
static CACHE_WRITE: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Catalog {
    pub schema_version: u32,
    pub updated_at: String,
    pub plugins: Vec<Entry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub repository: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_url: Option<String>,
    pub added_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    pub platforms: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub catalog: Catalog,
    pub source_url: String,
    pub source: &'static str,
    pub platform: &'static str,
    pub warning: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cached {
    source_url: String,
    catalog: Catalog,
}

fn valid_date(date: &str) -> bool {
    let b = date.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || b.iter()
            .enumerate()
            .any(|(i, b)| i != 4 && i != 7 && !b.is_ascii_digit())
    {
        return false;
    }
    let year: u32 = date[..4].parse().unwrap_or(0);
    let month: u32 = date[5..7].parse().unwrap_or(0);
    let day: u32 = date[8..].parse().unwrap_or(0);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    year > 0 && day > 0 && day <= days
}

impl Catalog {
    fn parse(content: &[u8]) -> Result<Self, String> {
        if content.len() > MAX_BYTES {
            return Err("Plugin catalog exceeds 512 KiB".into());
        }
        let catalog: Self =
            serde_json::from_slice(content).map_err(|e| format!("Invalid plugin catalog: {e}"))?;
        catalog.validate()?;
        Ok(catalog)
    }
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("Unsupported plugin catalog version".into());
        }
        if !valid_date(&self.updated_at) || self.plugins.len() > 256 {
            return Err("Invalid catalog date or too many catalog entries".into());
        }
        let mut ids = HashSet::new();
        for entry in &self.plugins {
            if !ids.insert(&entry.id)
                || !valid_date(&entry.added_at)
                || entry.added_at > self.updated_at
                || entry
                    .updated_at
                    .as_ref()
                    .is_some_and(|d| !valid_date(d) || d < &entry.added_at || d > &self.updated_at)
            {
                return Err(format!("Invalid or duplicate catalog entry: {}", entry.id));
            }
            super::remote::download_url(&entry.repository)?;
            if let Some(url) = &entry.install_url {
                super::remote::download_url(url)?;
            }
            if entry.tags.len() > 12
                || entry
                    .tags
                    .iter()
                    .any(|t| t.trim().is_empty() || t.len() > 40)
            {
                return Err("Invalid plugin catalog tags".into());
            }
            // Share package metadata rules, including links, images and platform names.
            Package {
                manifest: Manifest {
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                    description: entry.description.clone(),
                    version: entry.version.clone().unwrap_or_else(|| "0.0.0".into()),
                    api_version: 1,
                    author: entry.author.clone(),
                    image: entry.image.clone(),
                    homepage: None,
                    repository: Some(entry.repository.clone()),
                    android_native: None,
                    platforms: entry.platforms.clone(),
                    permissions: vec![],
                    contributions: vec![],
                },
                script: "catalog metadata".into(),
            }
            .validate(false)?;
        }
        Ok(())
    }
}

fn saved(root: &Path, source_url: &str) -> (Catalog, &'static str) {
    for name in ["catalog.json", "catalog.bak"] {
        let path = root.join(name);
        if !fs::metadata(&path).is_ok_and(|m| m.len() <= (MAX_BYTES * 2) as u64) {
            continue;
        }
        let cached = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Cached>(&bytes).ok());
        if let Some(cached) = cached {
            if cached.source_url == source_url && cached.catalog.validate().is_ok() {
                return (cached.catalog, "cache");
            }
        }
    }
    (
        Catalog::parse(EMBEDDED.as_bytes()).expect("validated embedded catalog"),
        "bundled",
    )
}

async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let mut response = super::remote::client()?
        .get(super::remote::download_url(url)?)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("Could not refresh plugin catalog: {}", e.without_url()))?;
    if !response.status().is_success() {
        return Err(format!(
            "Plugin catalog returned HTTP {}",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BYTES as u64)
    {
        return Err("Plugin catalog exceeds 512 KiB".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| e.without_url().to_string())?
    {
        if bytes.len() + chunk.len() > MAX_BYTES {
            return Err("Plugin catalog exceeds 512 KiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn load_with<F, Fut>(
    root: &Path,
    source_url: &str,
    refresh: bool,
    fetch: F,
) -> Result<Listing, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Vec<u8>, String>>,
{
    super::remote::download_url(source_url)?;
    let (mut catalog, mut source) = saved(root, source_url);
    let mut warning = None;
    if refresh {
        match fetch().await.and_then(|bytes| Catalog::parse(&bytes)) {
            Ok(downloaded) => {
                catalog = downloaded;
                source = "remote";
                let cache = Cached {
                    source_url: source_url.into(),
                    catalog: catalog.clone(),
                };
                let stored = CACHE_WRITE
                    .lock()
                    .map_err(|e| e.to_string())
                    .and_then(|_lock| {
                        super::store::write_atomic(
                            &root.join("catalog.json"),
                            &serde_json::to_vec(&cache).map_err(|e| e.to_string())?,
                        )
                    });
                if stored.is_err() {
                    warning =
                        Some("Catalog refreshed, but could not save it on this device.".into());
                }
            }
            Err(error) => {
                warning = Some(format!("{error}. Showing the available offline catalog."));
            }
        }
    }
    Ok(Listing {
        catalog,
        source_url: source_url.into(),
        source,
        platform: super::model::platform(),
        warning,
    })
}

pub async fn load(root: &Path, refresh: bool) -> Result<Listing, String> {
    let url = option_env!("PLAYER_PLUGIN_CATALOG_URL").unwrap_or(DEFAULT_URL);
    load_with(root, url, refresh, || fetch(url)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        serde_json::from_str(EMBEDDED).unwrap()
    }
    fn parse(value: serde_json::Value) -> Result<Catalog, String> {
        Catalog::parse(&serde_json::to_vec(&value).unwrap())
    }

    #[test]
    fn shipped_catalog_and_calendar_dates_are_valid() {
        let catalog = Catalog::parse(EMBEDDED.as_bytes()).unwrap();
        assert_eq!(catalog.plugins[0].id, "example.hello");
        assert!(valid_date("2024-02-29"));
        assert!(valid_date("2000-02-29"));
        for d in [
            "2025-02-29",
            "1900-02-29",
            "2026-04-31",
            "2026-00-01",
            "2026-10-00",
            "0000-01-01",
            "2026-1-01",
            "２０２６-10-27",
        ] {
            assert!(!valid_date(d), "accepted {d}");
        }
    }
    #[test]
    fn rejects_untrusted_or_ambiguous_catalog_metadata() {
        let mut value = fixture();
        value["schemaVersion"] = 2.into();
        assert!(parse(value).is_err());
        let mut value = fixture();
        let duplicate = value["plugins"][0].clone();
        value["plugins"].as_array_mut().unwrap().push(duplicate);
        assert!(parse(value).is_err());
        for (key, invalid) in [
            ("repository", "http://example.org/repo"),
            ("installUrl", "file:///tmp/package"),
            ("image", "javascript:alert(1)"),
            ("updatedAt", "2020-01-01"),
            ("id", "player.soulseek"),
        ] {
            let mut value = fixture();
            value["plugins"][0][key] = invalid.into();
            assert!(parse(value).is_err(), "accepted {key}");
        }
        let mut value = fixture();
        value["plugins"][0]["addedAt"] = "2027-01-01".into();
        assert!(parse(value).is_err());
        let mut value = fixture();
        value["plugins"][0]["platforms"] = serde_json::json!(["unknown"]);
        assert!(parse(value).is_err());
        assert!(Catalog::parse(&vec![b' '; MAX_BYTES + 1]).is_err());
        let mut value = fixture();
        let mut entry = value["plugins"][0].clone();
        entry.as_object_mut().unwrap().remove("image");
        value["plugins"] = serde_json::json!(vec![entry; 257]);
        assert!(parse(value).unwrap_err().contains("too many"));
        let mut value = fixture();
        value["plugins"][0]["script"] = "must not execute".into();
        assert!(parse(value).is_err());
    }
    #[test]
    fn refresh_persists_valid_catalog_and_keeps_it_when_offline_or_malformed() {
        let root = std::env::temp_dir().join(format!(
            "player-catalog-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let url = "https://example.org/catalog.json";
        let mut next = fixture();
        next["plugins"][0]["name"] = "Updated listing".into();
        let bytes = serde_json::to_vec(&next).unwrap();
        let run = |refresh, result| {
            tauri::async_runtime::block_on(load_with(&root, url, refresh, || {
                std::future::ready(result)
            }))
        };
        let fresh = run(true, Ok(bytes)).unwrap();
        assert_eq!(fresh.source, "remote");
        let offline = run(true, Err("offline".into())).unwrap();
        assert_eq!(offline.source, "cache");
        assert_eq!(offline.catalog.plugins[0].name, "Updated listing");
        assert!(offline.warning.is_some());
        let malformed = run(true, Ok(b"not json".to_vec())).unwrap();
        assert_eq!(malformed.source, "cache");
        let cached = run(false, Err("must not fetch".into())).unwrap();
        assert_eq!(cached.source, "cache");
        assert!(cached.warning.is_none());
        let other = tauri::async_runtime::block_on(load_with(
            &root,
            "https://other.org/catalog.json",
            false,
            || async { panic!("must not fetch") },
        ))
        .unwrap();
        assert_eq!(other.source, "bundled");
        let mut newer = fixture();
        newer["plugins"][0]["name"] = "Latest listing".into();
        run(true, Ok(serde_json::to_vec(&newer).unwrap())).unwrap();
        fs::write(root.join("catalog.json"), "broken").unwrap();
        let fallback = run(false, Err("must not fetch".into())).unwrap();
        assert_eq!(fallback.source, "cache");
        assert_eq!(fallback.catalog.plugins[0].name, "Updated listing");
        fs::write(root.join("catalog.bak"), "broken").unwrap();
        let fallback = run(false, Err("must not fetch".into())).unwrap();
        assert_eq!(fallback.source, "bundled");
        fs::remove_dir_all(root).unwrap();
    }
}
