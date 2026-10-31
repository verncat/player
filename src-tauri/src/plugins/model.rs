use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const API_VERSION: u32 = 2;
pub const MAX_PACKAGE_BYTES: usize = 96 * 1024 * 1024;
pub const PERMISSIONS: &[&str] = &[
    "ui",
    "playback.read",
    "playback.control",
    "library.import",
    "library.replace",
    "settings",
    "secrets",
    "network.http",
    "storage",
    "media.source",
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_interval_ms: Option<u64>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_in_menu: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub manifest: Manifest,
    pub native: NativePackage,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum NativePackage {
    Library {
        artifacts: std::collections::BTreeMap<String, NativeArtifact>,
    },
    AndroidApk,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeArtifact {
    pub data: String,
    pub sha256: String,
}
impl NativeArtifact {
    pub fn decode(&self) -> Result<Vec<u8>, String> {
        use base64::{engine::general_purpose::STANDARD, Engine};
        use sha2::{Digest, Sha256};
        if self.data.len() > 90 * 1024 * 1024 {
            return Err("Native library exceeds 64 MiB".into());
        }
        let bytes = STANDARD
            .decode(&self.data)
            .map_err(|_| "Invalid native library encoding")?;
        if bytes.is_empty() || bytes.len() > 64 * 1024 * 1024 {
            return Err("Invalid native library size".into());
        }
        if format!("{:x}", Sha256::digest(&bytes)) != self.sha256 {
            return Err("Native library SHA-256 mismatch".into());
        }
        Ok(bytes)
    }
}
pub fn native_target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
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
    pub secret_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_source: Option<InstallSource>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub manifest: Manifest,
    pub supported: bool,
    pub enabled: bool,
    pub generation: u64,
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
    pub fn parse(content: &str) -> Result<Self, String> {
        if content.len() > MAX_PACKAGE_BYTES {
            return Err("Plugin package exceeds 96 MiB".into());
        }
        let package: Self =
            serde_json::from_str(content).map_err(|e| format!("Invalid plugin package: {e}"))?;
        package.validate()?;
        Ok(package)
    }
    pub fn validate(&self) -> Result<(), String> {
        match &self.native {
            NativePackage::AndroidApk if self.manifest.android_native.is_some() => {}
            NativePackage::Library { artifacts }
                if !artifacts.is_empty() && self.manifest.android_native.is_none() =>
            {
                if artifacts.len() > 8 {
                    return Err("Too many native artifacts".into());
                }
                for (target, artifact) in artifacts {
                    if ![
                        "macos-aarch64",
                        "macos-x86_64",
                        "windows-x86_64",
                        "windows-aarch64",
                        "linux-x86_64",
                        "linux-aarch64",
                    ]
                    .contains(&target.as_str())
                    {
                        return Err("Unsupported native target".into());
                    }
                    let os = target.split('-').next().unwrap_or("");
                    if !self.manifest.platforms.iter().any(|p| p == os) {
                        return Err("Native artifact OS is absent from manifest platforms".into());
                    }
                    artifact.decode()?;
                }
            }
            _ => {
                return Err(
                    "Only native library packages or installed Android APKs are supported".into(),
                )
            }
        }
        self.manifest.validate()
    }
    pub fn fingerprint(&self) -> String {
        blake3::hash(&serde_json::to_vec(self).expect("serializable package"))
            .to_hex()
            .to_string()
    }
}

impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        let m = self;
        if m.poll_interval_ms
            .is_some_and(|interval| !(50..=60_000).contains(&interval))
        {
            return Err("Polling interval must be 50–60000 ms".into());
        }
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
        if !valid_id(&m.id) {
            return Err("Invalid plugin id".into());
        }
        if m.api_version != API_VERSION {
            return Err("Incompatible plugin API version".into());
        }
        if m.version.split('.').count() != 3
            || m.version.split('.').any(|v| v.parse::<u32>().is_err())
        {
            return Err("Version must be major.minor.patch".into());
        }
        if m.name.trim().is_empty() || m.name.len() > 100 || m.description.len() > 2000 {
            return Err("Invalid plugin name or description".into());
        }
        if m.platforms.is_empty()
            || m.platforms
                .iter()
                .any(|p| !["macos", "windows", "linux", "android"].contains(&p.as_str()))
        {
            return Err("Invalid plugin platforms".into());
        }
        if m.permissions
            .iter()
            .any(|p| !PERMISSIONS.contains(&p.as_str()))
        {
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
            if c.show_in_menu.is_some() && c.kind != "page" {
                return Err("showInMenu is only valid for page contributions".into());
            }
        }
        if !m.contributions.is_empty() && !m.permissions.iter().any(|p| p == "ui") {
            return Err("UI contributions require ui permission".into());
        }
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn fixture_native() -> NativePackage {
    NativePackage::Library {
        artifacts: [
            "macos-aarch64",
            "macos-x86_64",
            "linux-x86_64",
            "linux-aarch64",
            "windows-x86_64",
            "windows-aarch64",
        ]
        .into_iter()
        .map(|target| {
            (
                target.into(),
                NativeArtifact {
                    data: "YQ==".into(),
                    sha256: "ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb"
                        .into(),
                },
            )
        })
        .collect(),
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
                api_version: API_VERSION,
                description: String::new(),
                image: None,
                author: None,
                homepage: None,
                repository: None,
                android_native: None,
                poll_interval_ms: None,
                platforms: vec!["linux".into(), "macos".into(), "windows".into()],
                permissions: vec![],
                contributions: vec![],
            },
            native: fixture_native(),
        }
    }
    #[test]
    fn polling_interval_is_optional_and_bounded() {
        let mut p = package();
        for interval in [0, 49, 60_001, u64::MAX] {
            p.manifest.poll_interval_ms = Some(interval);
            assert!(p.validate().is_err());
        }
        for interval in [50, 250, 60_000] {
            p.manifest.poll_interval_ms = Some(interval);
            p.validate().unwrap();
        }
    }
    #[test]
    fn old_manifests_do_not_gain_null_metadata_fields() {
        let p = package();
        let json = serde_json::to_value(&p).unwrap();
        for key in ["image", "author", "homepage", "repository", "androidNative"] {
            assert!(json["manifest"].get(key).is_none());
        }
        let roundtrip = Package::parse(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p.fingerprint(), roundtrip.fingerprint());
    }
    #[test]
    fn accepts_author_forms_and_rejects_unsafe_links() {
        let mut p = package();
        p.manifest.author = Some(Author::Name("Example author".into()));
        assert!(p.validate().is_ok());
        p.manifest.author = Some(Author::Details(AuthorDetails {
            name: "Author".into(),
            url: Some("https://example.org/author".into()),
        }));
        p.manifest.repository = Some("https://github.com/example/plugin".into());
        assert!(p.validate().is_ok());
        p.manifest.homepage = Some("javascript:alert(1)".into());
        assert!(p.validate().is_err());
        assert!(web_url("https://user:password@example.org").is_err());
    }
    #[test]
    fn plugin_image_supports_links_and_embedded_raster_images() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let mut p = package();
        p.manifest.image = Some("https://example.org/icon.png".into());
        assert!(p.validate().is_ok());
        p.manifest.image = Some(format!(
            "data:image/png;base64,{}",
            STANDARD.encode(include_bytes!("../../icons/32x32.png"))
        ));
        assert!(p.validate().is_ok());
        let encoded = serde_json::to_string(&p).unwrap();
        assert_eq!(
            Package::parse(&encoded).unwrap().fingerprint(),
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
            assert!(p.validate().is_err(), "accepted {invalid}");
        }
        p.manifest.image = Some(format!("data:image/png;base64,{}", "A".repeat(350_000)));
        assert!(p.validate().is_err());
    }
    #[test]
    fn shipped_example_is_an_installable_external_package() {
        let p = Package {
            manifest: serde_json::from_str(include_str!(
                "../../../plugins/examples/hello/plugin.json"
            ))
            .unwrap(),
            native: fixture_native(),
        };
        assert!(p.validate().is_ok());
        let encoded = serde_json::to_string(&p).unwrap();
        assert_eq!(
            Package::parse(&encoded).unwrap().fingerprint(),
            p.fingerprint()
        );
    }
    #[test]
    fn official_plugin_ids_are_installable_and_provider_host_permissions_are_removed() {
        let mut p = package();
        assert!(p.validate().is_ok());
        p.manifest.id = "player.soulseek".into();
        assert!(p.validate().is_ok());
        p.manifest.id = "example.hello".into();
        p.manifest.permissions.push("native.soulseek".into());
        assert!(p.validate().is_err());
    }
    #[test]
    fn android_native_requires_permission_platform_and_pinned_identity() {
        let mut p = package();
        p.native = NativePackage::AndroidApk;
        p.manifest.android_native = Some(AndroidNative {
            package_name: "org.example.plugin".into(),
            entry_class: "org.example.plugin.Main".into(),
            certificate_sha256: "a".repeat(64),
            apk_sha256: "b".repeat(64),
        });
        assert!(p.validate().is_err());
        p.manifest.permissions.push("native.android".into());
        assert!(p.validate().is_err());
        p.manifest.platforms = vec!["android".into()];
        assert!(p.validate().is_ok());
        let valid = p.clone();
        p.manifest.android_native.as_mut().unwrap().entry_class = "org.other.Main".into();
        assert!(p.validate().is_err());
        p = valid.clone();
        p.manifest.android_native.as_mut().unwrap().apk_sha256 = "x".repeat(64);
        assert!(p.validate().is_err());
        p = valid;
        p.manifest.android_native = None;
        assert!(p.validate().is_err());
    }
    #[test]
    fn page_menu_visibility_is_optional_and_roundtrips() {
        let mut p = package();
        p.manifest.permissions.push("ui".into());
        p.manifest.contributions = vec![serde_json::from_value(serde_json::json!({
            "id": "details", "kind": "page", "label": "Details"
        }))
        .unwrap()];
        let json = serde_json::to_value(&p).unwrap();
        assert!(json["manifest"]["contributions"][0]
            .get("showInMenu")
            .is_none());
        let legacy = Package::parse(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p.fingerprint(), legacy.fingerprint());
        let legacy_fingerprint = p.fingerprint();
        for visible in [false, true] {
            p.manifest.contributions[0].show_in_menu = Some(visible);
            let json = serde_json::to_value(&p).unwrap();
            assert_eq!(json["manifest"]["contributions"][0]["showInMenu"], visible);
            let parsed = Package::parse(&serde_json::to_string(&p).unwrap()).unwrap();
            assert_eq!(parsed.manifest.contributions[0].show_in_menu, Some(visible));
            assert_eq!(p.fingerprint(), parsed.fingerprint());
            assert_ne!(p.fingerprint(), legacy_fingerprint);
        }
    }
    #[test]
    fn rejects_menu_visibility_on_other_contributions_and_non_booleans() {
        let mut p = package();
        p.manifest.permissions.push("ui".into());
        p.manifest.contributions = vec![serde_json::from_value(serde_json::json!({
            "id": "details", "kind": "page", "label": "Details", "showInMenu": false
        }))
        .unwrap()];
        for kind in ["settings", "search", "track-action", "player-widget"] {
            p.manifest.contributions[0].kind = kind.into();
            p.manifest.contributions[0].slot = Some("player.status".into());
            assert!(p.validate().unwrap_err().contains("showInMenu"));
        }
        let mut json = serde_json::to_value(&p).unwrap();
        json["manifest"]["contributions"][0]["kind"] = "page".into();
        json["manifest"]["contributions"][0]["showInMenu"] = "false".into();
        assert!(Package::parse(&json.to_string()).is_err());
    }
    #[test]
    fn rejects_path_ids_incompatible_api_and_duplicates() {
        assert!(!valid_id("../escape"));
        let mut p = package();
        p.manifest.api_version = 1;
        assert!(p.validate().is_err());
        p.manifest.api_version = API_VERSION;
        p.manifest.permissions.push("ui".into());
        let c = Contribution {
            id: "test".into(),
            kind: "page".into(),
            label: "Test".into(),
            description: String::new(),
            icon: None,
            order: 0,
            slot: None,
            show_in_menu: None,
        };
        p.manifest.contributions = vec![c.clone(), c];
        assert!(p.validate().is_err());
    }
    #[test]
    fn native_payloads_are_hashed_and_legacy_scripts_are_rejected() {
        let mut p = package();
        if let NativePackage::Library { artifacts } = &mut p.native {
            artifacts.values_mut().next().unwrap().sha256 = "0".repeat(64);
        }
        assert!(p.validate().unwrap_err().contains("SHA-256"));
        let legacy = serde_json::json!({"manifest":package().manifest,"script":"legacy"});
        assert!(Package::parse(&legacy.to_string()).is_err());
        let mut p = package();
        p.manifest.api_version = 1;
        assert!(p.validate().is_err());
    }
}
