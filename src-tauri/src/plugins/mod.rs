//! Player's runtime package registry and capability gateway.
pub mod android;
mod catalog;
#[cfg(feature = "discord-rpc")]
pub mod discord;
pub mod model;
pub mod native;
mod remote;
pub(crate) mod secrets;
#[cfg(feature = "soulseek")]
pub mod soulseek;
mod store;
use model::{InstallSource, Package, PluginInfo};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
pub use store::PluginState;
use tauri::{Emitter, Manager};

fn field<T: DeserializeOwned>(args: &Value, key: &str) -> Result<T, String> {
    serde_json::from_value(
        args.get(key)
            .cloned()
            .ok_or_else(|| format!("Missing {key}"))?,
    )
    .map_err(|e| format!("Invalid {key}: {e}"))
}
fn value(v: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

async fn read_secret(state: &PluginState, id: &str, name: &str) -> Result<Option<String>, String> {
    let cache = state.secrets.clone();
    let plugin_id = id.to_owned();
    let key = name.to_owned();
    let secret = tauri::async_runtime::spawn_blocking(move || cache.get(&plugin_id, &key))
        .await
        .map_err(|e| e.to_string())??;
    if state.has_secret(id, name)? != secret.is_some() {
        state.remember_secret(id, name, secret.is_some())?;
    }
    Ok(secret)
}

pub fn initialize(app: &tauri::AppHandle) -> Result<(), String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("plugin-system");
    let state = PluginState::new(root)?;
    #[cfg(feature = "soulseek")]
    {
        let cfg = app
            .state::<crate::library::LibraryState>()
            .get_device_settings()
            .map_err(|e| e.to_string())?;
        state.migrate_enabled("player.soulseek", cfg.soulseek_enabled)?;
        if state.settings("player.soulseek")?["revision"] == 0 {
            state.update_settings(
                "player.soulseek",
                0,
                json!({"username":cfg.soulseek_username}),
            )?;
        }
        if !cfg.soulseek_password.is_empty() {
            match state
                .secrets
                .get("player.soulseek", "password")
                .and_then(|existing| {
                    if existing.is_some() {
                        Ok(())
                    } else {
                        state
                            .secrets
                            .set("player.soulseek", "password", &cfg.soulseek_password)
                    }
                }) {
                Ok(()) => {
                    state.remember_secret("player.soulseek", "password", true)?;
                    app.state::<crate::library::LibraryState>()
                        .set_device_settings(
                            &cfg.emoji,
                            &cfg.device_name,
                            cfg.sync_enabled,
                            false,
                            "",
                            "",
                        )
                        .map_err(|e| e.to_string())?;
                }
                Err(error) => tracing::warn!("Soulseek credential migration deferred: {}", error),
            }
        }
        app.manage(soulseek::SoulseekState::new(
            state.root.join("staging/player.soulseek"),
        ));
    }
    #[cfg(feature = "discord-rpc")]
    app.manage(discord::DiscordPresenceState::default());
    app.manage(native::NativeRuntime::default());
    app.manage(state);
    Ok(())
}
pub fn emit_generation_event(
    app: &tauri::AppHandle,
    id: &str,
    generation: u64,
    topic: &str,
    payload: Value,
) {
    let state = app.state::<PluginState>();
    if state
        .record(id)
        .is_ok_and(|r| r.enabled && r.generation == generation)
    {
        let event = state.record_event(id, generation, topic, payload);
        let _ = app.emit("plugin-event", event);
    }
}
pub fn shutdown(app: &tauri::AppHandle) {
    native::shutdown(app);
    #[cfg(feature = "discord-rpc")]
    app.state::<discord::DiscordPresenceState>().shutdown();
    #[cfg(feature = "soulseek")]
    app.state::<soulseek::SoulseekState>().deactivate();
    let _ = app;
}
#[tauri::command]
pub fn plugins_diagnostics(state: tauri::State<'_, PluginState>) -> Vec<String> {
    state.warnings.clone()
}
#[tauri::command]
pub fn plugins_list(state: tauri::State<'_, PluginState>) -> Result<Vec<PluginInfo>, String> {
    state.list()
}
#[tauri::command]
pub fn plugins_preview(content: String) -> Result<Value, String> {
    let p = Package::parse(&content, false)?;
    Ok(json!({"manifest":p.manifest,"fingerprint":p.fingerprint()}))
}
#[tauri::command]
pub async fn plugins_download(
    url: String,
    expected_id: Option<String>,
) -> Result<remote::DownloadPreview, String> {
    let preview = remote::download(&url).await?;
    preview.verify_id(expected_id.as_deref())?;
    Ok(preview)
}
#[tauri::command]
pub async fn plugins_catalog() -> Result<catalog::Listing, String> {
    catalog::load().await
}
#[tauri::command]
pub fn plugins_install(
    content: String,
    fingerprint: String,
    source: Option<InstallSource>,
    state: tauri::State<'_, PluginState>,
) -> Result<PluginInfo, String> {
    state.install(&content, &fingerprint, source)
}
#[tauri::command]
pub fn plugins_uninstall(
    plugin_id: String,
    state: tauri::State<'_, PluginState>,
) -> Result<(), String> {
    if state.info(&plugin_id)?.enabled {
        return Err("Disable the plugin before uninstalling it".into());
    }
    for key in state.record(&plugin_id)?.secret_keys {
        state.secrets.delete(&plugin_id, &key)?;
    }
    state.uninstall(&plugin_id)
}
#[tauri::command]
pub fn plugin_load(
    plugin_id: String,
    state: tauri::State<'_, PluginState>,
) -> Result<Value, String> {
    let info = state.info(&plugin_id)?;
    if !info.enabled || !info.supported {
        return Err("Plugin disabled or unsupported".into());
    }
    Ok(json!({"info":info,"events":state.events(&plugin_id)}))
}
#[tauri::command]
pub fn plugins_migrate_enabled(
    plugin_id: String,
    enabled: bool,
    state: tauri::State<'_, PluginState>,
) -> Result<(), String> {
    state.migrate_enabled(&plugin_id, enabled)
}
#[tauri::command]
pub async fn plugins_set_enabled(
    plugin_id: String,
    enabled: bool,
    app: tauri::AppHandle,
    state: tauri::State<'_, PluginState>,
) -> Result<PluginInfo, String> {
    // Persist revocation first: a concurrent operation cannot obtain fresh capabilities.
    let previous_generation = state.record(&plugin_id)?.generation;
    let info = state.enabled(&plugin_id, enabled)?;
    if !enabled {
        let handle = app.clone();
        let id = plugin_id.clone();
        tauri::async_runtime::spawn_blocking(move || {
            native::stop(&handle, &id, previous_generation)
        })
        .await
        .map_err(|e| e.to_string())?;
        state.revoke_sources(&plugin_id);
        if app
            .state::<crate::playback::PlaybackState>()
            .resource_owner()
            .as_deref()
            == Some(&plugin_id)
        {
            app.state::<crate::playback::PlaybackState>().stop();
        }
        #[cfg(feature = "discord-rpc")]
        if plugin_id == "player.discord-rpc" {
            let _ = discord::discord_update_presence(false, None, app.state());
        }
        #[cfg(feature = "soulseek")]
        if plugin_id == "player.soulseek" {
            app.state::<soulseek::SoulseekState>().deactivate();
        }
        if cfg!(target_os = "android") && info.manifest.android_native.is_some() {
            let id = plugin_id.clone();
            let generation = info.generation;
            tauri::async_runtime::spawn_blocking(move || {
                android::mobile_call("deactivate", json!({"pluginId":id,"generation":generation}))
            })
            .await
            .map_err(|e| e.to_string())??;
        }
    }
    if previous_generation != info.generation {
        let cache = state.secrets.clone();
        let id = plugin_id.clone();
        tauri::async_runtime::spawn_blocking(move || cache.forget_plugin(&id))
            .await
            .map_err(|e| e.to_string())??;
    }
    let _ = app.emit("plugins-changed", ());
    Ok(info)
}

#[tauri::command]
pub async fn plugin_call(
    plugin_id: String,
    generation: u64,
    method: String,
    args: Value,
    app: tauri::AppHandle,
) -> Result<Value, String> {
    let state = app.state::<PluginState>();
    let permission = match method.as_str() {
        "settings.get" | "settings.update" => "settings",
        "secrets.has" | "secrets.set" | "secrets.delete" | "secrets.get" => "secrets",
        "library.import" => "library.import",
        "library.replace" => "library.replace",
        "http.request" | "http.download" => "network.http",
        m if m.starts_with("soulseek.") => "native.soulseek",
        m if m.starts_with("discord.") => "native.discord",
        _ => return Err("METHOD_NOT_FOUND".into()),
    };
    state.authorize(&plugin_id, generation, permission)?;
    let result = match method.as_str() {
        "settings.get" => state.settings(&plugin_id),
        "settings.update" => state.update_settings(
            &plugin_id,
            field(&args, "revision")?,
            field(&args, "values")?,
        ),
        "secrets.has" => value(state.has_secret(&plugin_id, &field::<String>(&args, "key")?)?),
        "secrets.get" => {
            value(read_secret(&state, &plugin_id, &field::<String>(&args, "key")?).await?)
        }
        "secrets.set" => {
            let key: String = field(&args, "key")?;
            let cache = state.secrets.clone();
            let id = plugin_id.clone();
            let name = key.clone();
            let secret: String = field(&args, "value")?;
            tauri::async_runtime::spawn_blocking(move || cache.set(&id, &name, &secret))
                .await
                .map_err(|e| e.to_string())??;
            state.remember_secret(&plugin_id, &key, true)?;
            Ok(Value::Null)
        }
        "secrets.delete" => {
            let key: String = field(&args, "key")?;
            let cache = state.secrets.clone();
            let id = plugin_id.clone();
            let name = key.clone();
            tauri::async_runtime::spawn_blocking(move || cache.delete(&id, &name))
                .await
                .map_err(|e| e.to_string())??;
            state.remember_secret(&plugin_id, &key, false)?;
            Ok(Value::Null)
        }
        #[cfg(feature = "discord-rpc")]
        "discord.availability" => value(discord::discord_availability()),
        #[cfg(feature = "discord-rpc")]
        "discord.update" => {
            discord::discord_update_presence(
                true,
                serde_json::from_value(args.get("playback").cloned().unwrap_or(Value::Null))
                    .map_err(|e| e.to_string())?,
                app.state(),
            )?;
            Ok(Value::Null)
        }
        #[cfg(feature = "soulseek")]
        m if m.starts_with("soulseek.") => {
            soulseek_call(&app, &plugin_id, generation, m, args).await
        }
        "http.request" => {
            let url: String = field(&args, "url")?;
            let parsed = reqwest::Url::parse(&url).map_err(|e| e.to_string())?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err("Only HTTP(S) URLs are supported".into());
            }
            let verb = args.get("method").and_then(Value::as_str).unwrap_or("GET");
            if !["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"].contains(&verb) {
                return Err("Unsupported HTTP method".into());
            }
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| e.to_string())?;
            let mut request = client.request(
                reqwest::Method::from_bytes(verb.as_bytes()).map_err(|e| e.to_string())?,
                parsed,
            );
            if let Some(headers) = args.get("headers").and_then(Value::as_object) {
                for (k, v) in headers {
                    request = request.header(k, v.as_str().ok_or("Header value must be a string")?);
                }
            }
            if let Some(body) = args.get("body").and_then(Value::as_str) {
                if body.len() > 1024 * 1024 {
                    return Err("HTTP request body exceeds 1 MiB".into());
                }
                request = request.body(body.to_owned());
            }
            let response = request.send().await.map_err(|e| e.to_string())?;
            let status = response.status().as_u16();
            let mut stream = response.bytes_stream();
            let mut bytes = Vec::new();
            use futures_util::StreamExt;
            while let Some(chunk) = stream.next().await {
                state.authorize(&plugin_id, generation, permission)?;
                let chunk = chunk.map_err(|e| e.to_string())?;
                if bytes.len() + chunk.len() > 1024 * 1024 {
                    return Err("HTTP response exceeds 1 MiB".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(json!({"status":status,"body":String::from_utf8_lossy(&bytes)}))
        }
        "http.download" => {
            use futures_util::StreamExt;
            use std::io::Write;
            let url: String = field(&args, "url")?;
            let parsed = reqwest::Url::parse(&url).map_err(|e| e.to_string())?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err("Only HTTP(S) URLs are supported".into());
            }
            let name: String = field(&args, "filename")?;
            if name.is_empty()
                || name.len() > 200
                || name != crate::import_paths::sanitize_path_segment(&name)
            {
                return Err("Invalid download filename".into());
            }
            let mut resource: crate::plugin_sources::ResourceIdentity = field(&args, "resource")?;
            resource.plugin_id = plugin_id.clone();
            let directory = state.root.join("staging").join(&plugin_id).join(format!(
                "download-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
            let path = directory.join(name);
            let result = async {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()
                    .map_err(|e| e.to_string())?;
                let response = client
                    .get(parsed)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?
                    .error_for_status()
                    .map_err(|e| e.to_string())?;
                let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
                let mut bytes = 0usize;
                let mut stream = response.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    state.authorize(&plugin_id, generation, "network.http")?;
                    let chunk = chunk.map_err(|e| e.to_string())?;
                    bytes += chunk.len();
                    if bytes > 64 * 1024 * 1024 {
                        return Err("Download exceeds 64 MiB".into());
                    }
                    file.write_all(&chunk).map_err(|e| e.to_string())?;
                }
                file.sync_all().map_err(|e| e.to_string())?;
                crate::plugin_sources::set_transfer_state(
                    &path,
                    crate::plugin_sources::TransferState::Completed,
                );
                let token =
                    state.register_source(&plugin_id, generation, path.clone(), resource)?;
                Ok(json!({"sourceToken":token,"bytes":bytes}))
            }
            .await;
            if result.is_err() {
                let _ = std::fs::remove_dir_all(&directory);
            }
            result
        }
        "library.import" | "library.replace" => {
            static FILE_OPERATIONS: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let _operation = FILE_OPERATIONS.lock().map_err(|e| e.to_string())?;
            state.authorize(&plugin_id, generation, permission)?;
            let token: String = field(&args, "sourceToken")?;
            let source = state.source(&plugin_id, generation, &token)?;
            if crate::plugin_sources::transfer_state(&source.path)
                != Some(crate::plugin_sources::TransferState::Completed)
            {
                return Err("Transfer has not completed".into());
            }
            let library = app.state::<crate::library::LibraryState>();
            // Work from a copy: a decoder may still own the original staging file.
            let copy_dir = state.root.join("imports").join(&plugin_id);
            std::fs::create_dir_all(&copy_dir).map_err(|e| e.to_string())?;
            let copy = copy_dir.join(source.path.file_name().ok_or("Source has no filename")?);
            std::fs::copy(&source.path, &copy).map_err(|e| e.to_string())?;
            if method == "library.replace" {
                let track = crate::library::replace_track_with_file(
                    field(&args, "trackId")?,
                    copy.to_string_lossy().into(),
                    app.clone(),
                    library,
                )?;
                app.state::<crate::playback::PlaybackState>().adopt_track(
                    &plugin_id,
                    &source.resource.resource_id,
                    track.id,
                );
                value(track)
            } else {
                let (artist, album) = crate::import_paths::extract_audio_metadata(&copy);
                let target_dir = crate::import_paths::build_metadata_based_path(
                    library.data_dir(),
                    artist.as_deref(),
                    album.as_deref(),
                );
                std::fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;
                let mut target = target_dir.join(copy.file_name().unwrap());
                if target.exists()
                    && crate::library::hash_file(&target) != crate::library::hash_file(&copy)
                {
                    let hash =
                        crate::library::hash_file(&copy).ok_or("Could not hash imported file")?;
                    target = target_dir.join(format!(
                        "{}-{}.{}",
                        copy.file_stem().unwrap_or_default().to_string_lossy(),
                        &hash[..12],
                        copy.extension().unwrap_or_default().to_string_lossy()
                    ));
                }
                if !target.exists() {
                    std::fs::copy(&copy, &target).map_err(|e| e.to_string())?;
                }
                let relative = target
                    .strip_prefix(library.data_dir())
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .into_owned();
                let mut track = crate::library::index_track_by_path(relative, library.clone())?;
                if let Some(t) = track.as_mut() {
                    if let Some(hash) = args
                        .get("coverHash")
                        .and_then(Value::as_str)
                        .or(source.resource.cover_hash.as_deref())
                    {
                        if hash.len() == 64
                            && hash.bytes().all(|b| b.is_ascii_hexdigit())
                            && crate::covers::is_stored(library.data_dir(), hash)
                            && t.cover_hash.is_none()
                        {
                            library
                                .conn()
                                .lock()
                                .unwrap()
                                .execute(
                                    "UPDATE tracks SET cover_hash=?1 WHERE id=?2",
                                    rusqlite::params![hash, t.id],
                                )
                                .map_err(|e| e.to_string())?;
                            t.cover_hash = Some(hash.into());
                        }
                    }
                }
                let _ = std::fs::remove_file(copy);
                if let Some(t) = &track {
                    app.state::<crate::playback::PlaybackState>().adopt_track(
                        &plugin_id,
                        &source.resource.resource_id,
                        t.id,
                    );
                }
                let _ = app.emit("library-changed", ());
                value(track)
            }
        }
        _ => Err("METHOD_NOT_FOUND".into()),
    }?;
    state.authorize(&plugin_id, generation, permission)?;
    Ok(result)
}

#[cfg(feature = "soulseek")]
async fn soulseek_call(
    app: &tauri::AppHandle,
    id: &str,
    generation: u64,
    method: &str,
    args: Value,
) -> Result<Value, String> {
    let library = app.state::<crate::library::LibraryState>();
    let state = app.state::<soulseek::SoulseekState>();
    let plugins = app.state::<PluginState>();
    let mut cfg = library.get_device_settings().map_err(|e| e.to_string())?;
    let settings = plugins.settings(id)?;
    cfg.soulseek_enabled = true;
    cfg.soulseek_username = settings["values"]["username"]
        .as_str()
        .unwrap_or(&cfg.soulseek_username)
        .into();
    if method == "soulseek.status" {
        return value(state.status(&cfg.soulseek_username, plugins.has_secret(id, "password")?));
    }
    if matches!(
        method,
        "soulseek.search" | "soulseek.download" | "soulseek.preview" | "soulseek.cover"
    ) {
        cfg.soulseek_password = read_secret(&plugins, id, "password")
            .await?
            .unwrap_or_default();
        plugins.authorize(id, generation, "native.soulseek")?;
        if cfg.soulseek_password.is_empty() {
            return Err("Save your Soulseek password in plugin settings first".into());
        }
        state.configure(cfg);
    }
    match method {
        "soulseek.search" => value(
            soulseek::soulseek_search(
                field(&args, "query")?,
                state,
                library,
                app.clone(),
                generation,
            )
            .await?,
        ),
        "soulseek.download" => value(
            soulseek::soulseek_download(
                field(&args, "request")?,
                app.clone(),
                state,
                library,
                generation,
            )
            .await?,
        ),
        "soulseek.preview" => value(
            soulseek::soulseek_preview(
                field(&args, "request")?,
                app.clone(),
                state,
                library,
                generation,
            )
            .await?,
        ),
        "soulseek.cancelPreview" => value(soulseek::soulseek_cancel_preview(
            field(&args, "request")?,
            state,
        )?),
        "soulseek.cover" => value(
            soulseek::soulseek_fetch_cover(
                field(&args, "request")?,
                state,
                library,
                app.clone(),
                generation,
            )
            .await?,
        ),
        "soulseek.prepare" => {
            let username: String = field(&args, "username")?;
            let filename: String = field(&args, "filename")?;
            let path = state.cache_path(&library, &username, &filename)?;
            if !path.is_file()
                || !path.canonicalize().map_err(|e| e.to_string())?.starts_with(
                    plugins
                        .root
                        .join("staging/player.soulseek")
                        .canonicalize()
                        .map_err(|e| e.to_string())?,
                )
            {
                return Err("Preview file is not ready".into());
            }
            let mut resource: crate::plugin_sources::ResourceIdentity = field(&args, "resource")?;
            resource.plugin_id = id.into();
            value(plugins.register_source(id, generation, path, resource)?)
        }
        _ => Err("METHOD_NOT_FOUND".into()),
    }
}

#[tauri::command]
pub fn plugin_play_source(
    plugin_id: String,
    generation: u64,
    source_token: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let state = app.state::<PluginState>();
    state.authorize(&plugin_id, generation, "playback.control")?;
    let source = state.source(&plugin_id, generation, &source_token)?;
    app.state::<crate::playback::PlaybackState>()
        .play_track_source(crate::library::TrackSource::GrowingFile {
            cache_path: source.path,
            resource: source.resource,
        })
}

#[tauri::command]
pub fn plugin_current_source(
    plugin_id: String,
    resource_id: String,
    app: tauri::AppHandle,
) -> Result<Value, String> {
    let state = app.state::<PluginState>();
    let record = state.record(&plugin_id)?;
    state.authorize(&plugin_id, record.generation, "playback.control")?;
    let (path, resource) = app
        .state::<crate::playback::PlaybackState>()
        .plugin_resource()
        .ok_or("Plugin source is no longer active")?;
    if resource.plugin_id != plugin_id || resource.resource_id != resource_id {
        return Err("Source is no longer active".into());
    }
    let token = state.register_source(&plugin_id, record.generation, path, resource)?;
    Ok(json!({"sourceToken":token,"generation":record.generation}))
}

#[tauri::command]
pub fn plugins_clear_cache(plugin_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<PluginState>();
    if state.info(&plugin_id)?.enabled {
        return Err("Disable the plugin before clearing its cache".into());
    }
    let path = state.root.join("staging").join(&plugin_id);
    if app
        .state::<crate::playback::PlaybackState>()
        .current_file_path()
        .is_some_and(|p| p.starts_with(&path))
    {
        return Err("Stop playback before clearing this cache".into());
    }
    if path.exists() {
        std::fs::remove_dir_all(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}
