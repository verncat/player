use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const API_VERSION: u32 = 1;
pub const MAX_PACKAGE_BYTES: usize = 4 * 1024 * 1024;
pub const PERMISSIONS: &[&str] = &[
    "ui",
    "playback.read",
    "playback.control",
    "library.import",
    "library.replace",
    "settings",
    "secrets",
    "network.http",
    "native.discord",
    "native.soulseek",
    "native.android",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api_version: u32,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android_native: Option<AndroidNative>,
    pub platforms: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub contributions: Vec<Contribution>,
}

/// An installed APK pinned during discovery. This is unrestricted native code.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidNative {
    pub package_name: String,
    pub entry_class: String,
    pub certificate_sha256: String,
    pub apk_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Author {
    Name(String),
    Details(AuthorDetails),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorDetails {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

pub fn web_url(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "Invalid website URL")?;
    if value.len() > 2048
        || !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Links must be HTTP(S) URLs without embedded credentials".into());
    }
    Ok(url)
}

fn validate_image(value: &str) -> Result<(), String> {
    if !value.starts_with("data:") {
        return web_url(value).map(|_| ());
    }
    use base64::{engine::general_purpose::STANDARD, Engine};
    let (header, encoded) = value.split_once(',').ok_or("Invalid plugin image")?;
    let format = match header {
        "data:image/png;base64" => image::ImageFormat::Png,
        "data:image/jpeg;base64" => image::ImageFormat::Jpeg,
        "data:image/webp;base64" => image::ImageFormat::WebP,
        "data:image/gif;base64" => image::ImageFormat::Gif,
        _ => return Err("Plugin image must be PNG, JPEG, WebP or GIF".into()),
    };
    if encoded.len() > (256 * 1024_usize).div_ceil(3) * 4 {
        return Err("Embedded plugin image exceeds 256 KiB".into());
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| "Invalid plugin image base64")?;
    if bytes.len() > 256 * 1024 || image::guess_format(&bytes).ok() != Some(format) {
        return Err("Invalid plugin image content or size".into());
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstallSource {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

impl InstallSource {
    pub fn validate(&self) -> Result<(), String> {
        if web_url(&self.url)?.scheme() != "https" {
            return Err("Plugin downloads require HTTPS".into());
        }
        if let Some(repository) = &self.repository {
            web_url(repository)?;
        }
        if let Some(revision) = &self.revision {
            if self.repository.is_none()
                || !matches!(revision.len(), 40 | 64)
                || !revision.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("Invalid source revision".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Contribution {
    pub id: String,
    pub kind: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub order: i32,
    #[serde(default)]
    pub slot: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub manifest: Manifest,
    pub script: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub enabled: bool,
    pub generation: u64,
    pub settings_revision: u64,
    #[serde(default)]
    pub settings: Value,
    #[serde(default)]
    pub migrated: bool,
    #[serde(default)]
    pub secret_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_source: Option<InstallSource>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub manifest: Manifest,
    pub builtin: bool,
    pub supported: bool,
    pub enabled: bool,
    pub generation: u64,
    pub migrated: bool,
    pub fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_source: Option<InstallSource>,
}

pub fn valid_id(id: &str) -> bool {
    id.len() <= 100
        && id.contains('.')
        && id.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}
pub fn valid_local_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
pub fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unsupported"
    }
}
impl Package {
    pub fn parse(content: &str, builtin: bool) -> Result<Self, String> {
        if content.len() > MAX_PACKAGE_BYTES {
            return Err("Plugin package exceeds 4 MiB".into());
        }
        let package: Self =
            serde_json::from_str(content).map_err(|e| format!("Invalid plugin package: {e}"))?;
        package.validate(builtin)?;
        Ok(package)
    }
    pub fn validate(&self, builtin: bool) -> Result<(), String> {
        let m = &self.manifest;
        if let Some(image) = &m.image {
            validate_image(image)?;
        }
        if let Some(author) = &m.author {
            let name = match author {
                Author::Name(name) => name,
                Author::Details(details) => {
                    if let Some(url) = &details.url {
                        web_url(url)?;
                    }
                    &details.name
                }
            };
            if name.trim().is_empty() || name.len() > 200 {
                return Err("Invalid author name".into());
            }
        }
        for url in [&m.homepage, &m.repository].into_iter().flatten() {
            web_url(url)?;
        }
        if !valid_id(&m.id) || (!builtin && m.id.starts_with("player.")) {
            return Err("Invalid or reserved plugin id".into());
        }
        if m.api_version != API_VERSION {
            return Err("Incompatible plugin API version".into());
        }
        if m.version.split('.').count() != 3
            || m.version.split('.').any(|v| v.parse::<u32>().is_err())
        {
            return Err("Version must be major.minor.patch".into());
        }
        if m.name.trim().is_empty()
            || m.name.len() > 100
            || m.description.len() > 2000
            || self.script.trim().is_empty()
        {
            return Err("Invalid plugin name, description or script".into());
        }
        if m.platforms.is_empty()
            || m.platforms
                .iter()
                .any(|p| !["macos", "windows", "linux", "android"].contains(&p.as_str()))
        {
            return Err("Invalid plugin platforms".into());
        }
        if m.permissions.iter().any(|p| {
            !PERMISSIONS.contains(&p.as_str())
                || (!builtin && p.starts_with("native.") && p != "native.android")
        }) {
            return Err("Unknown or reserved plugin permission".into());
        }
        let native_permission = m.permissions.iter().any(|p| p == "native.android");
        if native_permission != m.android_native.is_some() {
            return Err("native.android requires an androidNative binding".into());
        }
        if let Some(native) = &m.android_native {
            let valid_java_name = |s: &str| {
                s.len() <= 240
                    && s.contains('.')
                    && s.split('.').all(|part| {
                        !part.is_empty()
                            && part.bytes().enumerate().all(|(i, b)| {
                                b.is_ascii_alphabetic()
                                    || b == b'_'
                                    || (i > 0 && b.is_ascii_digit())
                            })
                    })
            };
            if m.platforms != ["android"]
                || !valid_java_name(&native.package_name)
                || !valid_java_name(&native.entry_class)
                || !native
                    .entry_class
                    .starts_with(&format!("{}.", native.package_name))
                || [&native.certificate_sha256, &native.apk_sha256]
                    .iter()
                    .any(|hash| {
                        hash.len() != 64
                            || !hash
                                .bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    })
            {
                return Err("Invalid Android native binding".into());
            }
        }
        if m.contributions.len() > 32 {
            return Err("Too many plugin contributions".into());
        }
        let mut ids = std::collections::HashSet::new();
        for c in &m.contributions {
            if !valid_local_id(&c.id)
                || !ids.insert(&c.id)
                || c.label.trim().is_empty()
                || c.label.len() > 100
                || c.description.len() > 2000
            {
                return Err("Invalid or duplicate contribution".into());
            }
            if ![
                "page",
                "settings",
                "search",
                "track-action",
                "player-widget",
            ]
            .contains(&c.kind.as_str())
            {
                return Err("Unknown contribution kind".into());
            }
            if c.kind == "player-widget"
                && !matches!(c.slot.as_deref(), Some("player.actions" | "player.status"))
            {
                return Err("Invalid player slot".into());
            }
        }
        if !m.contributions.is_empty() && !m.permissions.iter().any(|p| p == "ui") {
            return Err("UI contributions require ui permission".into());
        }
        Ok(())
    }
    pub fn fingerprint(&self) -> String {
        blake3::hash(&serde_json::to_vec(self).expect("serializable package"))
            .to_hex()
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn package() -> Package {
        Package {
            manifest: Manifest {
                id: "example.hello".into(),
                name: "Hello".into(),
                version: "1.0.0".into(),
                api_version: 1,
                description: String::new(),
                image: None,
                author: None,
                homepage: None,
                repository: None,
                android_native: None,
                platforms: vec!["linux".into()],
                permissions: vec![],
                contributions: vec![],
            },
            script: "registerPlugin({})".into(),
        }
    }
    #[test]
    fn old_manifests_do_not_gain_null_metadata_fields() {
        let p = package();
        let json = serde_json::to_value(&p).unwrap();
        for key in ["image", "author", "homepage", "repository", "androidNative"] {
            assert!(json["manifest"].get(key).is_none());
        }
        let roundtrip = Package::parse(&serde_json::to_string(&p).unwrap(), false).unwrap();
        assert_eq!(p.fingerprint(), roundtrip.fingerprint());
    }
    #[test]
    fn accepts_author_forms_and_rejects_unsafe_links() {
        let mut p = package();
        p.manifest.author = Some(Author::Name("Example author".into()));
        assert!(p.validate(false).is_ok());
        p.manifest.author = Some(Author::Details(AuthorDetails {
            name: "Author".into(),
            url: Some("https://example.org/author".into()),
        }));
        p.manifest.repository = Some("https://github.com/example/plugin".into());
        assert!(p.validate(false).is_ok());
        p.manifest.homepage = Some("javascript:alert(1)".into());
        assert!(p.validate(false).is_err());
        assert!(web_url("https://user:password@example.org").is_err());
    }
    #[test]
    fn plugin_image_supports_links_and_embedded_raster_images() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let mut p = package();
        p.manifest.image = Some("https://example.org/icon.png".into());
        assert!(p.validate(false).is_ok());
        p.manifest.image = Some(format!(
            "data:image/png;base64,{}",
            STANDARD.encode(include_bytes!("../../icons/32x32.png"))
        ));
        assert!(p.validate(false).is_ok());
        let encoded = serde_json::to_string(&p).unwrap();
        assert_eq!(
            Package::parse(&encoded, false).unwrap().fingerprint(),
            p.fingerprint()
        );
        for invalid in [
            "javascript:alert(1)",
            "file:///tmp/icon.png",
            "icon.png",
            "",
            "https://user:password@example.org/icon.png",
            "data:image/png;base64,invalid",
            "data:image/png;base64,aGVsbG8=",
            "data:image/svg+xml;base64,PHN2Zy8+",
        ] {
            p.manifest.image = Some(invalid.into());
            assert!(p.validate(false).is_err(), "accepted {invalid}");
        }
        p.manifest.image = Some(format!("data:image/png;base64,{}", "A".repeat(350_000)));
        assert!(p.validate(false).is_err());
    }
    #[test]
    fn shipped_example_is_an_installable_external_package() {
        let p = Package {
            manifest: serde_json::from_str(include_str!(
                "../../../plugins/examples/hello/plugin.json"
            ))
            .unwrap(),
            script: include_str!("../../../plugins/examples/hello/main.js").into(),
        };
        assert!(p.validate(false).is_ok());
        let encoded = serde_json::to_string(&p).unwrap();
        assert_eq!(
            Package::parse(&encoded, false).unwrap().fingerprint(),
            p.fingerprint()
        );
    }
    #[test]
    fn rejects_reserved_ids_and_native_permissions() {
        let mut p = package();
        assert!(p.validate(false).is_ok());
        p.manifest.id = "player.soulseek".into();
        assert!(p.validate(false).is_err());
        p.manifest.id = "example.hello".into();
        p.manifest.permissions.push("native.soulseek".into());
        assert!(p.validate(false).is_err());
    }
    #[test]
    fn android_native_requires_permission_platform_and_pinned_identity() {
        let mut p = package();
        p.manifest.android_native = Some(AndroidNative {
            package_name: "org.example.plugin".into(),
            entry_class: "org.example.plugin.Main".into(),
            certificate_sha256: "a".repeat(64),
            apk_sha256: "b".repeat(64),
        });
        assert!(p.validate(false).is_err());
        p.manifest.permissions.push("native.android".into());
        assert!(p.validate(false).is_err());
        p.manifest.platforms = vec!["android".into()];
        assert!(p.validate(false).is_ok());
        let valid = p.clone();
        p.manifest.android_native.as_mut().unwrap().entry_class = "org.other.Main".into();
        assert!(p.validate(false).is_err());
        p = valid.clone();
        p.manifest.android_native.as_mut().unwrap().apk_sha256 = "x".repeat(64);
        assert!(p.validate(false).is_err());
        p = valid;
        p.manifest.android_native = None;
        assert!(p.validate(false).is_err());
    }
    #[test]
    fn rejects_path_ids_incompatible_api_and_duplicates() {
        assert!(!valid_id("../escape"));
        let mut p = package();
        p.manifest.api_version = 2;
        assert!(p.validate(false).is_err());
        p.manifest.api_version = 1;
        p.manifest.permissions.push("ui".into());
        let c = Contribution {
            id: "test".into(),
            kind: "page".into(),
            label: "Test".into(),
            description: String::new(),
            icon: None,
            order: 0,
            slot: None,
        };
        p.manifest.contributions = vec![c.clone(), c];
        assert!(p.validate(false).is_err());
    }
}
