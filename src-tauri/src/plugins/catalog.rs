//! Discovery metadata only. Installation still downloads and reviews a real package.
use super::model::{Author, Manifest, API_VERSION};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, time::Duration};

const DEFAULT_URL: &str =
    "https://raw.githubusercontent.com/verncat/player-plugins/main/catalog.json";
const MAX_BYTES: usize = 512 * 1024;

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
    pub platform: &'static str,
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
        if self.schema_version != 2 {
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
            Manifest {
                id: entry.id.clone(),
                name: entry.name.clone(),
                description: entry.description.clone(),
                version: entry.version.clone().unwrap_or_else(|| "0.0.0".into()),
                api_version: API_VERSION,
                author: entry.author.clone(),
                image: entry.image.clone(),
                homepage: None,
                repository: Some(entry.repository.clone()),
                android_native: None,
                platforms: entry.platforms.clone(),
                permissions: vec![],
                contributions: vec![],
            }
            .validate(false)?;
        }
        Ok(())
    }
}

async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let mut response = super::remote::client()?
        .get(super::remote::download_url(url)?)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("Could not load plugin catalog: {}", e.without_url()))?;
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

async fn load_with<F, Fut>(source_url: &str, fetch: F) -> Result<Listing, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Vec<u8>, String>>,
{
    super::remote::download_url(source_url)?;
    let catalog = Catalog::parse(&fetch().await?)?;
    Ok(Listing {
        catalog,
        platform: super::model::platform(),
    })
}

pub async fn load() -> Result<Listing, String> {
    let url = option_env!("PLAYER_PLUGIN_CATALOG_URL").unwrap_or(DEFAULT_URL);
    load_with(url, || fetch(url)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        serde_json::json!({
            "schemaVersion": 2,
            "updatedAt": "2026-10-27",
            "plugins": [{
                "id": "example.hello", "name": "Hello Player", "description": "Test entry",
                "repository": "https://github.com/example/hello",
                "addedAt": "2026-10-27", "platforms": ["linux"], "tags": []
            }]
        })
    }
    fn parse(value: serde_json::Value) -> Result<Catalog, String> {
        Catalog::parse(&serde_json::to_vec(&value).unwrap())
    }

    #[test]
    fn catalog_and_calendar_dates_are_valid() {
        let catalog = parse(fixture()).unwrap();
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
    fn published_catalog_from_pinned_plugin_repository_is_valid() {
        let catalog = Catalog::parse(include_bytes!("../../../plugins/catalog.json")).unwrap();
        assert_eq!(catalog.schema_version, 2);
    }
    #[test]
    fn rejects_untrusted_or_ambiguous_catalog_metadata() {
        let mut value = fixture();
        value["schemaVersion"] = 3.into();
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
    fn each_load_fetches_a_new_catalog() {
        let mut calls = 0;
        for name in ["First response", "New response"] {
            let mut value = fixture();
            value["plugins"][0]["name"] = name.into();
            let bytes = serde_json::to_vec(&value).unwrap();
            let listing = tauri::async_runtime::block_on(load_with(
                "https://example.org/catalog.json",
                || {
                    calls += 1;
                    std::future::ready(Ok(bytes))
                },
            ))
            .unwrap();
            assert_eq!(listing.catalog.plugins[0].name, name);
            let json = serde_json::to_value(&listing).unwrap();
            assert_eq!(json.as_object().unwrap().len(), 2);
            assert!(json.get("catalog").is_some());
            assert!(json.get("platform").is_some());
        }
        assert_eq!(calls, 2);
    }
    #[test]
    fn failed_or_invalid_downloads_return_errors_without_a_fallback() {
        let run = |response| {
            tauri::async_runtime::block_on(load_with("https://example.org/catalog.json", || {
                std::future::ready(response)
            }))
        };
        // A previous successful request must not turn a later failure into a saved list.
        assert!(run(Ok(serde_json::to_vec(&fixture()).unwrap())).is_ok());
        for error in ["Network unavailable", "Plugin catalog returned HTTP 404"] {
            assert_eq!(run(Err(error.into())).unwrap_err(), error);
        }
        assert!(run(Ok(b"not json".to_vec()))
            .unwrap_err()
            .contains("Invalid plugin catalog"));
        let mut invalid = fixture();
        invalid["schemaVersion"] = 3.into();
        assert!(run(Ok(serde_json::to_vec(&invalid).unwrap())).is_err());
    }
    #[test]
    fn empty_remote_catalog_stays_empty_and_unsafe_urls_are_rejected_before_fetch() {
        let empty =
            serde_json::json!({ "schemaVersion": 2, "updatedAt": "2026-10-27", "plugins": [] });
        let listing = tauri::async_runtime::block_on(load_with(
            "https://example.org/catalog.json",
            || async { Ok(serde_json::to_vec(&empty).unwrap()) },
        ))
        .unwrap();
        assert!(listing.catalog.plugins.is_empty());
        let invalid =
            tauri::async_runtime::block_on(load_with("file:///tmp/catalog.json", || async {
                panic!("must not fetch unsafe URLs")
            }));
        assert!(invalid.is_err());
    }
}
