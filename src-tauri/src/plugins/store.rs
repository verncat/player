use super::model::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Clone, Default, Serialize, Deserialize)]
struct Registry {
    #[serde(default)]
    packages: BTreeMap<String, Package>,
    #[serde(default)]
    records: BTreeMap<String, Record>,
}
#[derive(Clone)]
pub struct SourceGrant {
    pub plugin_id: String,
    pub generation: u64,
    pub path: PathBuf,
    pub resource: crate::plugin_sources::ResourceIdentity,
}
pub struct PluginState {
    pub root: PathBuf,
    pub warnings: Vec<String>,
    pub(super) secrets: std::sync::Arc<super::secrets::SecretCache>,
    inner: Mutex<Registry>,
    sources: Mutex<BTreeMap<String, SourceGrant>>,
    events: Mutex<BTreeMap<String, Value>>,
    sequence: std::sync::atomic::AtomicU64,
}

pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp).map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    // Windows rename cannot replace the destination. The backup also permits recovery.
    let backup = path.with_extension("bak");
    if path.exists() {
        if backup.exists() {
            fs::remove_file(&backup).map_err(|e| e.to_string())?;
        }
        fs::rename(path, &backup).map_err(|e| e.to_string())?;
    }
    if let Err(error) = fs::rename(&tmp, path) {
        if backup.exists() {
            let _ = fs::rename(&backup, path);
        }
        return Err(error.to_string());
    }
    Ok(())
}
impl PluginState {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let mut warnings = Vec::new();
        let mut inner = Registry::default();
        for name in ["registry.json", "registry.bak"] {
            let path = root.join(name);
            if !path.exists() {
                continue;
            }
            let loaded = fs::metadata(&path)
                .map_err(|e| e.to_string())
                .and_then(|m| {
                    if m.len() > 512 * 1024 * 1024 {
                        Err("Registry is too large".into())
                    } else {
                        fs::read(&path).map_err(|e| e.to_string())
                    }
                })
                .and_then(|bytes| {
                    let mut value: Value = serde_json::from_slice(&bytes).map_err(|e|e.to_string())?;
                    let mut removed=Vec::new();
                    if let Some(packages)=value["packages"].as_object_mut() {
                        packages.retain(|id,p| {
                            let native=p.get("native").is_some();
                            if !native {removed.push(id.clone());warnings.push(format!("Removed legacy JavaScript plugin {id}; install a native version"));}
                            native
                        });
                    }
                    for id in removed {
                        if let Some(record)=value["records"].get_mut(&id) {record["enabled"]=json!(false);}
                    }
                    serde_json::from_value::<Registry>(value).map_err(|e|e.to_string())
                });
            match loaded {
                Ok(registry) => {
                    inner = registry;
                    break;
                }
                Err(error) => {
                    warnings.push(format!(
                        "Could not read {name}: {error}. The previous registry was preserved."
                    ));
                    let copy = root.join(format!(
                        "{name}.corrupt-{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos()
                    ));
                    let _ = fs::copy(path, copy);
                }
            }
        }
        inner.packages.retain(|id, p| {
            let valid = p.validate().is_ok() && id == &p.manifest.id;
            if !valid {
                warnings.push(format!("Ignored invalid installed package {id}"));
            }
            valid
        });
        for (id, record) in inner.records.iter_mut() {
            if !inner.packages.contains_key(id) {
                record.enabled = false;
            }
            if record.enabled {
                record.generation = record.generation.wrapping_add(1);
            }
        }
        Ok(Self {
            root,
            warnings,
            secrets: std::sync::Arc::new(super::secrets::SecretCache::default()),
            inner: Mutex::new(inner),
            sources: Mutex::new(BTreeMap::new()),
            events: Mutex::new(BTreeMap::new()),
            sequence: std::sync::atomic::AtomicU64::new(1),
        })
    }
    fn change<T>(&self, f: impl FnOnce(&mut Registry) -> Result<T, String>) -> Result<T, String> {
        let mut guard = self.inner.lock().map_err(|e| e.to_string())?;
        let mut next = guard.clone();
        let result = f(&mut next)?;
        let bytes = serde_json::to_vec(&next).map_err(|e| e.to_string())?;
        if bytes.len() > 512 * 1024 * 1024 {
            return Err("Installed native packages exceed the registry size limit".into());
        }
        write_atomic(&self.root.join("registry.json"), &bytes)?;
        *guard = next;
        Ok(result)
    }
    pub fn package(&self, id: &str) -> Result<Package, String> {
        self.inner
            .lock()
            .map_err(|e| e.to_string())?
            .packages
            .get(id)
            .cloned()
            .ok_or_else(|| "Plugin not found".into())
    }
    pub fn manifest(&self, id: &str) -> Result<Manifest, String> {
        self.inner
            .lock()
            .map_err(|e| e.to_string())?
            .packages
            .get(id)
            .map(|p| p.manifest.clone())
            .ok_or_else(|| "Plugin not found".into())
    }
    pub fn record(&self, id: &str) -> Result<Record, String> {
        let registry = self.inner.lock().map_err(|e| e.to_string())?;
        if !registry.packages.contains_key(id) {
            return Err("Plugin not found".into());
        }
        Ok(registry.records.get(id).cloned().unwrap_or_default())
    }
    pub fn info(&self, id: &str) -> Result<PluginInfo, String> {
        let p = self.package(id)?;
        let record = self.record(id)?;
        Ok(PluginInfo {
            supported: p.manifest.platforms.iter().any(|p| p == platform())
                && match &p.native {
                    NativePackage::Library { artifacts } => {
                        artifacts.contains_key(&native_target())
                    }
                    NativePackage::AndroidApk => cfg!(target_os = "android"),
                },
            fingerprint: p.fingerprint(),
            manifest: p.manifest,
            enabled: record.enabled,
            generation: record.generation,
            install_source: record.install_source,
        })
    }
    pub fn list(&self) -> Result<Vec<PluginInfo>, String> {
        let mut ids: Vec<_> = self
            .inner
            .lock()
            .map_err(|e| e.to_string())?
            .packages
            .keys()
            .cloned()
            .collect();
        ids.sort();
        ids.dedup();
        ids.iter().map(|id| self.info(id)).collect()
    }
    pub fn enabled(&self, id: &str, enabled: bool) -> Result<PluginInfo, String> {
        if enabled && !self.info(id)?.supported {
            return Err("Plugin is not supported on this platform".into());
        }
        if self.record(id)?.enabled == enabled {
            return self.info(id);
        }
        self.change(|r| {
            let e = r.records.entry(id.into()).or_default();
            e.enabled = enabled;
            e.generation = e.generation.wrapping_add(1);
            Ok(())
        })?;
        self.info(id)
    }
    pub fn authorize(&self, id: &str, generation: u64, permission: &str) -> Result<(), String> {
        let manifest = self.manifest(id)?;
        let r = self.record(id)?;
        if !r.enabled || r.generation != generation {
            return Err("DISABLED: plugin activation has ended".into());
        }
        if !manifest.platforms.iter().any(|p| p == platform()) {
            return Err("UNSUPPORTED".into());
        }
        if !manifest.permissions.iter().any(|p| p == permission) {
            return Err(format!("PERMISSION_DENIED: {permission}"));
        }
        Ok(())
    }
    pub fn settings(&self, id: &str) -> Result<Value, String> {
        let r = self.record(id)?;
        Ok(
            json!({"revision":r.settings_revision,"values":if r.settings.is_object(){r.settings}else{json!({})}}),
        )
    }
    pub fn update_settings(&self, id: &str, revision: u64, values: Value) -> Result<Value, String> {
        self.package(id)?;
        if !values.is_object()
            || serde_json::to_vec(&values)
                .map_err(|e| e.to_string())?
                .len()
                > 65536
        {
            return Err("Invalid settings (maximum 64 KiB)".into());
        }
        self.change(|r| {
            let e = r.records.entry(id.into()).or_default();
            if e.settings_revision != revision {
                return Err("CONFLICT: settings changed; reload them first".into());
            }
            e.settings = values;
            e.settings_revision += 1;
            Ok(())
        })?;
        self.settings(id)
    }
    pub fn install(
        &self,
        content: &str,
        fingerprint: &str,
        source: Option<InstallSource>,
    ) -> Result<PluginInfo, String> {
        if let Some(source) = &source {
            source.validate()?;
        }
        let p = Package::parse(content)?;
        if p.fingerprint() != fingerprint {
            return Err("Package fingerprint changed".into());
        }
        let id = p.manifest.id.clone();
        self.change(|r| {
            if r.records.get(&id).is_some_and(|r| r.enabled) {
                return Err("Disable the plugin before updating it".into());
            }
            if r.packages.len() >= 32 && !r.packages.contains_key(&id) {
                return Err("At most 32 installed plugins".into());
            }
            r.packages.insert(id.clone(), p);
            r.records.entry(id.clone()).or_default().install_source = source;
            Ok(())
        })?;
        self.info(&id)
    }
    pub fn uninstall(&self, id: &str) -> Result<(), String> {
        self.change(|r| {
            if r.records.get(id).is_some_and(|r| r.enabled) {
                return Err("Disable the plugin before uninstalling it".into());
            }
            r.packages.remove(id);
            r.records.remove(id);
            Ok(())
        })
    }
    pub fn has_secret(&self, id: &str, key: &str) -> Result<bool, String> {
        if !valid_local_id(key) {
            return Err("Invalid secret key".into());
        }
        Ok(self.record(id)?.secret_keys.iter().any(|k| k == key))
    }
    pub fn remember_secret(&self, id: &str, key: &str, present: bool) -> Result<(), String> {
        if !valid_local_id(key) {
            return Err("Invalid secret key".into());
        }
        self.package(id)?;
        self.change(|r| {
            let record = r.records.entry(id.into()).or_default();
            record.secret_keys.retain(|k| k != key);
            if present {
                record.secret_keys.push(key.into());
            }
            Ok(())
        })
    }
    pub fn register_source(
        &self,
        id: &str,
        generation: u64,
        path: PathBuf,
        resource: crate::plugin_sources::ResourceIdentity,
    ) -> Result<String, String> {
        if self.authorize(id, generation, "playback.control").is_err()
            && self.authorize(id, generation, "library.import").is_err()
            && self.authorize(id, generation, "library.replace").is_err()
        {
            return Err("PERMISSION_DENIED: source access".into());
        }
        if resource.plugin_id != id
            || resource
                .duration_seconds
                .is_some_and(|n| !n.is_finite() || n < 0.0)
            || resource.resource_id.is_empty()
            || resource.resource_id.len() > 4000
            || resource.title.len() > 2000
            || resource.artist.len() > 2000
            || resource.album.len() > 2000
        {
            return Err("Invalid resource metadata".into());
        }
        let token = format!(
            "{}-{}",
            std::process::id(),
            self.sequence
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let mut sources = self.sources.lock().map_err(|e| e.to_string())?;
        if sources.len() >= 1000 {
            return Err("Too many prepared plugin sources".into());
        }
        sources.insert(
            token.clone(),
            SourceGrant {
                plugin_id: id.into(),
                generation,
                path,
                resource,
            },
        );
        Ok(token)
    }
    pub fn source(&self, id: &str, generation: u64, token: &str) -> Result<SourceGrant, String> {
        let s = self
            .sources
            .lock()
            .map_err(|e| e.to_string())?
            .get(token)
            .cloned()
            .ok_or("Source expired")?;
        if s.plugin_id != id || s.generation != generation {
            return Err("Source belongs to another activation".into());
        }
        Ok(s)
    }
    pub fn revoke_sources(&self, id: &str) {
        self.sources
            .lock()
            .unwrap()
            .retain(|_, s| s.plugin_id != id);
    }
    pub fn record_event(&self, id: &str, generation: u64, topic: &str, payload: Value) -> Value {
        let revision = self
            .sequence
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let event = json!({"pluginId":id,"generation":generation,"revision":revision,"topic":topic,"payload":payload});
        let key = format!(
            "{id}/{topic}/{}",
            payload
                .get("transferId")
                .and_then(Value::as_str)
                .unwrap_or("state")
        );
        let mut events = self.events.lock().unwrap();
        if events.len() >= 512 && !events.contains_key(&key) {
            if let Some(key) = events.keys().next().cloned() {
                events.remove(&key);
            }
        }
        events.insert(key, event.clone());
        event
    }
    pub fn events(&self, id: &str) -> Vec<Value> {
        self.events
            .lock()
            .unwrap()
            .values()
            .filter(|e| e["pluginId"] == id)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fresh_player_contains_no_preinstalled_provider_packages() {
        let root = std::env::temp_dir().join(format!("player-no-bundled-{}", std::process::id()));
        let state = PluginState::new(root.clone()).unwrap();
        assert!(state.list().unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn corrupt_primary_registry_preserves_backup_and_does_not_prevent_startup() {
        let root =
            std::env::temp_dir().join(format!("player-plugin-corrupt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("registry.json"), b"broken").unwrap();
        fs::write(
            root.join("registry.bak"),
            b"{\"packages\":{},\"records\":{}}",
        )
        .unwrap();
        let state = PluginState::new(root.clone()).unwrap();
        assert!(!state.warnings.is_empty());
        assert!(fs::read_dir(&root).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("corrupt-")));
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn persistence_revisions_and_stale_generation() {
        let root = std::env::temp_dir().join(format!("player-plugin-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let state = PluginState::new(root.clone()).unwrap();
        let mut p = Package {
            manifest: serde_json::from_str(include_str!(
                "../../../plugins/examples/hello/plugin.json"
            ))
            .unwrap(),
            native: fixture_native(),
        };
        p.manifest.id = "example.test".into();
        p.manifest.permissions = vec!["settings".into()];
        p.manifest.contributions.clear();
        let source = InstallSource {
            url: "https://github.com/example/plugin".into(),
            repository: Some("https://github.com/example/plugin".into()),
            revision: Some("a".repeat(40)),
        };
        state
            .install(
                &serde_json::to_string(&p).unwrap(),
                &p.fingerprint(),
                Some(source),
            )
            .unwrap();
        let info = state.enabled("example.test", true).unwrap();
        assert!(!state.has_secret("example.test", "password").unwrap());
        state
            .remember_secret("example.test", "password", true)
            .unwrap();
        assert!(state.has_secret("example.test", "password").unwrap());
        assert!(!state.has_secret("example.test", "another").unwrap());
        assert!(state.has_secret("example.test", "../invalid").is_err());
        state
            .remember_secret("example.test", "password", false)
            .unwrap();
        assert!(!state.has_secret("example.test", "password").unwrap());
        assert!(state
            .authorize("example.test", info.generation, "settings")
            .is_ok());
        assert!(state
            .authorize("example.test", info.generation, "playback.control")
            .is_err());
        state
            .update_settings("example.test", 0, json!({"a":1}))
            .unwrap();
        assert!(state.update_settings("example.test", 0, json!({})).is_err());
        state.enabled("example.test", false).unwrap();
        assert!(state
            .authorize("example.test", info.generation, "settings")
            .is_err());
        drop(state);
        let loaded = PluginState::new(root.clone()).unwrap();
        assert_eq!(loaded.settings("example.test").unwrap()["values"]["a"], 1);
        assert_eq!(
            loaded
                .info("example.test")
                .unwrap()
                .install_source
                .unwrap()
                .revision,
            Some("a".repeat(40))
        );
        loaded
            .install(&serde_json::to_string(&p).unwrap(), &p.fingerprint(), None)
            .unwrap();
        assert!(loaded
            .info("example.test")
            .unwrap()
            .install_source
            .is_none());
        assert_eq!(loaded.settings("example.test").unwrap()["values"]["a"], 1);
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn removing_script_packages_preserves_settings_and_allows_native_replacement() {
        let root = std::env::temp_dir().join(format!(
            "player-plugin-native-migration-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("registry.json"),json!({"packages":{"example.hello":{"manifest":{"id":"example.hello","apiVersion":1},"script":"legacy"}},"records":{"example.hello":{"enabled":true,"generation":3,"settingsRevision":4,"settings":{"greeting":"Keep me"},"secretKeys":["token"]}}}).to_string()).unwrap();
        let state = PluginState::new(root.clone()).unwrap();
        assert!(state.package("example.hello").is_err());
        assert!(state.warnings.iter().any(|w| w.contains("example.hello")));
        let p = Package {
            manifest: serde_json::from_str(include_str!(
                "../../../plugins/examples/hello/plugin.json"
            ))
            .unwrap(),
            native: fixture_native(),
        };
        let installed = state
            .install(&serde_json::to_string(&p).unwrap(), &p.fingerprint(), None)
            .unwrap();
        assert!(!installed.enabled);
        assert_eq!(
            state.settings("example.hello").unwrap()["values"]["greeting"],
            "Keep me"
        );
        assert!(state.has_secret("example.hello", "token").unwrap());
        let _ = fs::remove_dir_all(root);
    }
}
