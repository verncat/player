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
    inner: Mutex<Registry>,
    builtins: BTreeMap<String, Package>,
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
                    if m.len() > 132 * 1024 * 1024 {
                        Err("Registry is too large".into())
                    } else {
                        fs::read(&path).map_err(|e| e.to_string())
                    }
                })
                .and_then(|bytes| {
                    serde_json::from_slice::<Registry>(&bytes).map_err(|e| e.to_string())
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
        let mut builtins = BTreeMap::new();
        #[cfg(feature = "discord-rpc")]
        {
            let p = Package {
                manifest: serde_json::from_str(include_str!(
                    "../../../plugins/discord-rpc/plugin.json"
                ))
                .map_err(|e| e.to_string())?,
                script: include_str!("../../../plugins/discord-rpc/main.js").into(),
            };
            p.validate(true)?;
            builtins.insert(p.manifest.id.clone(), p);
        }
        #[cfg(feature = "soulseek")]
        {
            let p = Package {
                manifest: serde_json::from_str(include_str!(
                    "../../../plugins/soulseek/plugin.json"
                ))
                .map_err(|e| e.to_string())?,
                script: include_str!("../../../plugins/soulseek/main.js").into(),
            };
            p.validate(true)?;
            builtins.insert(p.manifest.id.clone(), p);
        }
        inner.packages.retain(|id, p| {
            let valid = p.validate(false).is_ok() && id == &p.manifest.id;
            if !valid {
                warnings.push(format!("Ignored invalid installed package {id}"));
            }
            valid
        });
        for record in inner.records.values_mut() {
            if record.enabled {
                record.generation = record.generation.wrapping_add(1);
            }
        }
        Ok(Self {
            root,
            warnings,
            inner: Mutex::new(inner),
            builtins,
            sources: Mutex::new(BTreeMap::new()),
            events: Mutex::new(BTreeMap::new()),
            sequence: std::sync::atomic::AtomicU64::new(1),
        })
    }
    fn change<T>(&self, f: impl FnOnce(&mut Registry) -> Result<T, String>) -> Result<T, String> {
        let mut guard = self.inner.lock().map_err(|e| e.to_string())?;
        let mut next = guard.clone();
        let result = f(&mut next)?;
        write_atomic(
            &self.root.join("registry.json"),
            &serde_json::to_vec(&next).map_err(|e| e.to_string())?,
        )?;
        *guard = next;
        Ok(result)
    }
    pub fn package(&self, id: &str) -> Result<Package, String> {
        self.builtins
            .get(id)
            .cloned()
            .or_else(|| self.inner.lock().ok()?.packages.get(id).cloned())
            .ok_or_else(|| "Plugin not found".into())
    }
    pub fn record(&self, id: &str) -> Result<Record, String> {
        self.package(id)?;
        Ok(self
            .inner
            .lock()
            .map_err(|e| e.to_string())?
            .records
            .get(id)
            .cloned()
            .unwrap_or_default())
    }
    pub fn info(&self, id: &str) -> Result<PluginInfo, String> {
        let p = self.package(id)?;
        let record = self.record(id)?;
        Ok(PluginInfo {
            supported: p.manifest.platforms.iter().any(|p| p == platform()),
            fingerprint: p.fingerprint(),
            manifest: p.manifest,
            builtin: self.builtins.contains_key(id),
            enabled: record.enabled,
            generation: record.generation,
            migrated: record.migrated,
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
        ids.extend(self.builtins.keys().cloned());
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
            e.migrated = true;
            e.generation = e.generation.wrapping_add(1);
            Ok(())
        })?;
        self.info(id)
    }
    pub fn migrate_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        if self.package(id).is_err() {
            return Ok(());
        }
        self.change(|r| {
            let e = r.records.entry(id.into()).or_default();
            if !e.migrated {
                e.enabled = enabled;
                e.migrated = true;
                e.generation += 1;
            }
            Ok(())
        })
    }
    pub fn authorize(&self, id: &str, generation: u64, permission: &str) -> Result<(), String> {
        let p = self.package(id)?;
        let r = self.record(id)?;
        if !r.enabled || r.generation != generation {
            return Err("DISABLED: plugin activation has ended".into());
        }
        if !p.manifest.platforms.iter().any(|p| p == platform()) {
            return Err("UNSUPPORTED".into());
        }
        if !p.manifest.permissions.iter().any(|p| p == permission) {
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
        let p = Package::parse(content, false)?;
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
        if self.builtins.contains_key(id) {
            return Err("Built-in plugins can be disabled, not uninstalled".into());
        }
        self.change(|r| {
            if r.records.get(id).is_some_and(|r| r.enabled) {
                return Err("Disable the plugin before uninstalling it".into());
            }
            r.packages.remove(id);
            r.records.remove(id);
            Ok(())
        })
    }
    pub fn remember_secret(&self, id: &str, key: &str, present: bool) -> Result<(), String> {
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
        let p=Package::parse(r#"{"manifest":{"id":"example.test","name":"Test","version":"1.0.0","apiVersion":1,"platforms":["macos","linux","windows"],"permissions":["settings"]},"script":"registerPlugin({})"}"#,false).unwrap();
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
}
