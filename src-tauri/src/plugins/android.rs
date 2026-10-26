//! APK extensions: JavaScript UI uses the existing worker; native calls stay pinned to one APK.
use super::model::Package;
use serde_json::{json, Value};

#[cfg(target_os = "android")]
static MOBILE: std::sync::OnceLock<tauri::plugin::PluginHandle<tauri::Wry>> =
    std::sync::OnceLock::new();

pub fn mobile_call(method: &str, args: Value) -> Result<Value, String> {
    #[cfg(target_os = "android")]
    {
        MOBILE
            .get()
            .ok_or("Android extension loader is not initialized")?
            .run_mobile_plugin(method, args)
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (method, args);
        Err("APK extensions require Android".into())
    }
}

#[tauri::command]
pub async fn plugins_android_discover() -> Result<Value, String> {
    if !cfg!(target_os = "android") {
        return Ok(json!({"supported":false,"plugins":[],"warnings":[]}));
    }
    tauri::async_runtime::spawn_blocking(|| {
        let found = mobile_call("discover", json!({}))?;
        let mut plugins = Vec::new();
        let mut warnings = found["warnings"].as_array().cloned().unwrap_or_default();
        for candidate in found["packages"].as_array().into_iter().flatten() {
            let content = candidate.as_str().ok_or("Invalid APK discovery response")?;
            match Package::parse(content, false) {
                Ok(package) if package.manifest.android_native.is_some() => plugins.push(json!({
                    "content":content,"manifest":package.manifest,"fingerprint":package.fingerprint()
                })),
                Ok(_) => warnings.push(json!("APK is missing its native binding")),
                Err(error) => warnings.push(json!(error)),
            }
        }
        Ok(json!({"supported":true,"plugins":plugins,"warnings":warnings}))
    }).await.map_err(|e| e.to_string())?
}

#[cfg(target_os = "android")]
pub fn mobile_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("player-extensions")
        .setup(|_app, api| {
            let handle = api.register_android_plugin("dev.verncat.player", "ApkPlugins")?;
            let _ = MOBILE.set(handle);
            Ok(())
        })
        .build()
}
