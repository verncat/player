//! Credential values never enter the plugin registry or logs.
use std::{collections::BTreeMap, sync::Mutex};

/// Successful reads, missing values and refusals are shared within a plugin activation.
/// Holding the lock across provider access coalesces concurrent reads into one OS request.
#[derive(Default)]
pub struct SecretCache {
    values: Mutex<BTreeMap<String, Result<Option<String>, String>>>,
}
impl SecretCache {
    fn get_with(
        &self,
        key: String,
        read: impl FnOnce() -> Result<Option<String>, String>,
    ) -> Result<Option<String>, String> {
        let mut values = self
            .values
            .lock()
            .map_err(|_| "Credential cache is unavailable")?;
        values.entry(key).or_insert_with(read).clone()
    }
    fn update_with(
        &self,
        key: String,
        value: Option<String>,
        write: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let mut values = self
            .values
            .lock()
            .map_err(|_| "Credential cache is unavailable")?;
        write()?;
        values.insert(key, Ok(value));
        Ok(())
    }
    pub fn get(&self, id: &str, name: &str) -> Result<Option<String>, String> {
        self.get_with(key(id, name)?, || read(id, name))
    }
    pub fn set(&self, id: &str, name: &str, value: &str) -> Result<(), String> {
        if value.len() > 16384 {
            return Err("Secret too large".into());
        }
        self.update_with(key(id, name)?, Some(value.into()), || {
            write(id, name, value)
        })
    }
    pub fn delete(&self, id: &str, name: &str) -> Result<(), String> {
        self.update_with(key(id, name)?, None, || remove(id, name))
    }
    pub fn forget_plugin(&self, id: &str) -> Result<(), String> {
        let prefix = format!("{id}/");
        self.values
            .lock()
            .map_err(|_| "Credential cache is unavailable")?
            .retain(|key, _| !key.starts_with(&prefix));
        Ok(())
    }
}
#[cfg(target_os = "linux")]
use std::io::Write;
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};
fn key(id: &str, name: &str) -> Result<String, String> {
    if !super::model::valid_local_id(name) {
        return Err("Invalid secret key".into());
    }
    Ok(format!("{id}/{name}"))
}
#[cfg(target_os = "windows")]
fn wide(key: &str) -> Vec<u16> {
    format!("dev.verncat.player.plugins/{key}")
        .encode_utf16()
        .chain(Some(0))
        .collect()
}
fn write(id: &str, name: &str, value: &str) -> Result<(), String> {
    let key = key(id, name)?;
    if value.len() > 16384 {
        return Err("Secret too large".into());
    }
    #[cfg(target_os = "macos")]
    {
        return security_framework::passwords::set_generic_password(
            "dev.verncat.player.plugins",
            &key,
            value.as_bytes(),
        )
        .map_err(|e| format!("Keychain: {e}"));
    }
    #[cfg(target_os = "linux")]
    {
        let mut child = Command::new("secret-tool")
            .args([
                "store",
                "--label=Player plugin",
                "application",
                "dev.verncat.player",
                "key",
                &key,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| {
                "Install a Secret Service provider and secret-tool to save plugin credentials"
            })?;
        child
            .stdin
            .take()
            .ok_or("Secret Service stdin unavailable")?
            .write_all(value.as_bytes())
            .map_err(|e| e.to_string())?;
        return if child.wait().map_err(|e| e.to_string())?.success() {
            Ok(())
        } else {
            Err("Secret Service refused credential storage".into())
        };
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Security::Credentials::*;
        let mut target = wide(&key);
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: value.len() as u32,
            CredentialBlob: value.as_ptr() as *mut u8,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        return if unsafe { CredWriteW(&credential, 0) } != 0 {
            Ok(())
        } else {
            Err(format!(
                "Credential Manager: {}",
                std::io::Error::last_os_error()
            ))
        };
    }
    #[cfg(target_os = "android")]
    {
        return mobile_call("write", serde_json::json!({"key":key,"value":value})).map(|_| ());
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "windows",
        target_os = "android"
    )))]
    {
        let _ = (key, value);
        Err("Secure credential provider is unavailable on this platform".into())
    }
}
fn read(id: &str, name: &str) -> Result<Option<String>, String> {
    let key = key(id, name)?;
    #[cfg(target_os = "macos")]
    {
        match security_framework::passwords::get_generic_password(
            "dev.verncat.player.plugins",
            &key,
        ) {
            Ok(v) => return String::from_utf8(v).map(Some).map_err(|e| e.to_string()),
            Err(e) if e.code() == -25300 => return Ok(None),
            Err(e) => return Err(format!("Keychain: {e}")),
        }
    }
    #[cfg(target_os = "linux")]
    {
        let output = Command::new("secret-tool")
            .args(["lookup", "application", "dev.verncat.player", "key", &key])
            .stderr(Stdio::null())
            .output()
            .map_err(|_| "Secret Service is unavailable")?;
        return if output.status.success() {
            String::from_utf8(output.stdout)
                .map(|s| Some(s.trim_end_matches('\n').into()))
                .map_err(|e| e.to_string())
        } else {
            Ok(None)
        };
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Security::Credentials::*;
        let target = wide(&key);
        let mut credential = std::ptr::null_mut();
        if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } == 0 {
            let error = std::io::Error::last_os_error();
            return if error.raw_os_error() == Some(1168) {
                Ok(None)
            } else {
                Err(format!("Credential Manager: {error}"))
            };
        }
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (*credential).CredentialBlob,
                (*credential).CredentialBlobSize as usize,
            )
        }
        .to_vec();
        unsafe { CredFree(credential.cast()) };
        return String::from_utf8(bytes)
            .map(Some)
            .map_err(|e| e.to_string());
    }
    #[cfg(target_os = "android")]
    {
        return mobile_call("read", serde_json::json!({"key":key}))
            .and_then(|v| serde_json::from_value(v["value"].clone()).map_err(|e| e.to_string()));
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "windows",
        target_os = "android"
    )))]
    {
        let _ = key;
        Err("Secure credential provider is unavailable on this platform".into())
    }
}
fn remove(id: &str, name: &str) -> Result<(), String> {
    let key = key(id, name)?;
    #[cfg(target_os = "macos")]
    {
        return match security_framework::passwords::delete_generic_password(
            "dev.verncat.player.plugins",
            &key,
        ) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == -25300 => Ok(()),
            Err(e) => Err(format!("Keychain: {e}")),
        };
    }
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("secret-tool")
            .args(["clear", "application", "dev.verncat.player", "key", &key])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| "Secret Service is unavailable")?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Security::Credentials::*;
        let target = wide(&key);
        if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        return if e.raw_os_error() == Some(1168) {
            Ok(())
        } else {
            Err(format!("Credential Manager: {e}"))
        };
    }
    #[cfg(target_os = "android")]
    {
        return mobile_call("delete", serde_json::json!({"key":key})).map(|_| ());
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "windows",
        target_os = "android"
    )))]
    {
        let _ = key;
        Err("Secure credential provider is unavailable on this platform".into())
    }
}

#[cfg(target_os = "android")]
static MOBILE: std::sync::OnceLock<tauri::plugin::PluginHandle<tauri::Wry>> =
    std::sync::OnceLock::new();
#[cfg(target_os = "android")]
fn mobile_call(method: &str, args: serde_json::Value) -> Result<serde_json::Value, String> {
    MOBILE
        .get()
        .ok_or("Android credential provider not initialized")?
        .run_mobile_plugin(method, args)
        .map_err(|e| e.to_string())
}
#[cfg(target_os = "android")]
pub fn mobile_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("player-secrets")
        .setup(|_app, api| {
            let handle = api.register_android_plugin("dev.verncat.player", "PluginSecrets")?;
            let _ = MOBILE.set(handle);
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Barrier,
    };

    #[test]
    fn concurrent_reads_only_open_the_credential_provider_once() {
        let cache = Arc::new(SecretCache::default());
        let count = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let cache = cache.clone();
                let count = count.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    cache
                        .get_with("example.one/password".into(), || {
                            count.fetch_add(1, Ordering::SeqCst);
                            Ok(Some("test-only".into()))
                        })
                        .unwrap()
                })
            })
            .collect();
        for worker in workers {
            assert_eq!(worker.join().unwrap().as_deref(), Some("test-only"));
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn refusal_is_not_retried_automatically_and_reset_is_scoped_to_plugin() {
        let cache = SecretCache::default();
        let count = AtomicUsize::new(0);
        for _ in 0..5 {
            assert!(cache
                .get_with("example.one/password".into(), || {
                    count.fetch_add(1, Ordering::SeqCst);
                    Err("Access declined".into())
                })
                .is_err());
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
        cache
            .get_with("example.two/password".into(), || Ok(Some("other".into())))
            .unwrap();
        cache.forget_plugin("example.one").unwrap();
        assert_eq!(
            cache
                .get_with("example.one/password".into(), || {
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok(Some("retry".into()))
                })
                .unwrap()
                .as_deref(),
            Some("retry")
        );
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(
            cache
                .get_with("example.two/password".into(), || panic!(
                    "unrelated cache was cleared"
                ))
                .unwrap()
                .as_deref(),
            Some("other")
        );
    }

    #[test]
    fn writes_and_deletes_update_cache_only_after_success() {
        let cache = SecretCache::default();
        assert!(cache
            .set("example.one", "password", &"a".repeat(16385))
            .is_err());
        let key = "example.one/password".to_string();
        cache
            .get_with(key.clone(), || Err("Access declined".into()))
            .unwrap_err();
        cache
            .update_with(key.clone(), Some("saved".into()), || Ok(()))
            .unwrap();
        assert_eq!(
            cache
                .get_with(key.clone(), || panic!("save should update cache"))
                .unwrap()
                .as_deref(),
            Some("saved")
        );
        assert!(cache
            .update_with(key.clone(), Some("failed".into()), || Err(
                "write failed".into()
            ))
            .is_err());
        assert_eq!(
            cache
                .get_with(key.clone(), || panic!("failed write should keep old value"))
                .unwrap()
                .as_deref(),
            Some("saved")
        );
        cache.update_with(key.clone(), None, || Ok(())).unwrap();
        assert_eq!(
            cache
                .get_with(key, || panic!("delete should cache absence"))
                .unwrap(),
            None
        );
    }
}
