use crate::library::{DeviceSettings, LibraryState};
use serde::{Deserialize, Serialize};
use soulseek_rs::types::Download;
use soulseek_rs::{Client, DownloadCanceller, DownloadHandle, DownloadStatus, File};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Manager;

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "opus", "aac", "m4a", "wav", "wv", "ape", "alac", "aiff",
];

const COVER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];

const COVER_BASENAME_PRIORITY: &[&str] = &["cover", "folder", "front", "album", "art"];

use crate::import_paths::{
    build_metadata_based_path, extract_audio_metadata, sanitize_path_segment,
};
use crate::plugin_sources::{
    set_transfer_state as set_preview_transfer_state, TransferState as PreviewTransferState,
};

#[derive(Clone, PartialEq, Eq)]
struct SoulseekConfig {
    username: String,
    password: String,
}

#[derive(Default)]
struct SoulseekInner {
    config: Option<SoulseekConfig>,
    client: Option<Client>,
}

pub struct SoulseekState {
    config: Mutex<Option<DeviceSettings>>,
    cache_root: PathBuf,
    epoch: AtomicU64,
    preview_paths: Mutex<HashMap<String, PathBuf>>,
    inner: Arc<Mutex<SoulseekInner>>,
    preview_cancellers: Arc<Mutex<HashMap<String, DownloadCanceller>>>,
}

impl SoulseekState {
    pub fn new(cache_root: PathBuf) -> Self {
        Self {
            cache_root,
            config: Mutex::new(None),
            epoch: AtomicU64::new(0),
            preview_paths: Mutex::new(HashMap::new()),
            inner: Arc::new(Mutex::new(SoulseekInner::default())),
            preview_cancellers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn configure(&self, settings: DeviceSettings) {
        *self.config.lock().unwrap() = Some(settings);
    }
    pub fn deactivate(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        for (_, cancel) in self.preview_cancellers.lock().unwrap().drain() {
            cancel.cancel();
        }
        self.inner.lock().unwrap().client = None;
        if let Some(config) = self.config.lock().unwrap().as_mut() {
            config.soulseek_enabled = false;
        }
    }
    fn settings(&self, library: &LibraryState) -> Result<DeviceSettings, String> {
        if let Some(config) = self.config.lock().unwrap().clone() {
            Ok(config)
        } else {
            library.get_device_settings().map_err(|e| e.to_string())
        }
    }
    fn check_epoch(&self, epoch: u64) -> Result<(), String> {
        if self.epoch.load(Ordering::SeqCst) != epoch {
            Err("CANCELLED: Soulseek activation ended".into())
        } else {
            Ok(())
        }
    }
    pub fn cache_path(
        &self,
        _library: &LibraryState,
        username: &str,
        filename: &str,
    ) -> Result<PathBuf, String> {
        self.preview_paths
            .lock()
            .unwrap()
            .get(&Self::preview_key(username, filename))
            .cloned()
            .ok_or("Preview has not been downloaded".into())
    }
    fn preview_key(username: &str, filename: &str) -> String {
        format!("{}\u{0}{}", username, filename)
    }

    fn register_preview_canceller(
        &self,
        username: &str,
        filename: &str,
        transfer_id: &str,
        canceler: DownloadCanceller,
        epoch: u64,
    ) -> Result<(), String> {
        let mut registry = self.preview_cancellers.lock().unwrap();
        if let Err(error) = self.check_epoch(epoch) {
            canceler.cancel();
            return Err(error);
        }
        registry.insert(
            format!(
                "{}\u{0}{}",
                Self::preview_key(username, filename),
                transfer_id
            ),
            canceler,
        );
        Ok(())
    }
    fn cancellation_scope(
        &self,
        username: &str,
        filename: &str,
        canceler: DownloadCanceller,
        epoch: u64,
    ) -> Result<CancellationScope, String> {
        let id = format!(
            "aux-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        self.register_preview_canceller(username, filename, &id, canceler.clone(), epoch)?;
        Ok(CancellationScope {
            registry: self.preview_canceller_registry(),
            key: format!("{}\u{0}{}", Self::preview_key(username, filename), id),
            canceler,
        })
    }

    fn take_preview_canceller(&self, username: &str, filename: &str) -> Option<DownloadCanceller> {
        let prefix = format!("{}\u{0}", Self::preview_key(username, filename));
        let mut map = self.preview_cancellers.lock().unwrap();
        let key = map.keys().find(|k| k.starts_with(&prefix)).cloned()?;
        map.remove(&key)
    }

    fn preview_canceller_registry(&self) -> Arc<Mutex<HashMap<String, DownloadCanceller>>> {
        Arc::clone(&self.preview_cancellers)
    }

    fn desired_config(settings: &DeviceSettings) -> Option<SoulseekConfig> {
        if !settings.soulseek_enabled {
            return None;
        }

        let username = settings.soulseek_username.trim().to_string();
        let password = settings.soulseek_password.clone();
        if username.is_empty() || password.is_empty() {
            return None;
        }

        Some(SoulseekConfig { username, password })
    }

    fn sync_client_config(&self, desired: Option<SoulseekConfig>) -> Option<Client> {
        let mut inner = self.inner.lock().unwrap();
        if inner.config != desired {
            inner.client = None;
            inner.config = desired.clone();
        }

        if inner.client.is_none() {
            if let Some(config) = desired {
                inner.client = Some(Client::new(config.username, config.password));
            }
        }

        inner.client.clone()
    }

    fn maybe_client_for_settings(&self, settings: &DeviceSettings) -> Option<Client> {
        self.sync_client_config(Self::desired_config(settings))
    }

    fn client_for_download(&self, settings: &DeviceSettings) -> Result<Client, String> {
        self.maybe_client_for_settings(settings).ok_or_else(|| {
            if settings.soulseek_enabled {
                "Soulseek username and password are required".to_string()
            } else {
                "Soulseek is disabled in settings".to_string()
            }
        })
    }
}

struct CancellationScope {
    registry: Arc<Mutex<HashMap<String, DownloadCanceller>>>,
    key: String,
    canceler: DownloadCanceller,
}
impl Drop for CancellationScope {
    fn drop(&mut self) {
        self.canceler.cancel();
        self.registry.lock().unwrap().remove(&self.key);
    }
}
fn validate_remote_filename(filename: &str) -> Result<(), String> {
    let basename = soulseek_basename(filename);
    if filename.len() > 8192
        || filename.contains('\0')
        || basename != sanitize_path_segment(basename)
    {
        return Err("Invalid remote filename".into());
    }
    Ok(())
}
fn assert_generation(app: &tauri::AppHandle, generation: u64) -> Result<(), String> {
    app.state::<crate::plugins::PluginState>().authorize(
        "player.soulseek",
        generation,
        "native.soulseek",
    )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SoulseekStatus {
    pub enabled: bool,
    pub configured: bool,
    pub username: Option<String>,
    pub active_session: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SoulseekSearchResult {
    pub username: String,
    pub filename: String,
    pub basename: String,
    pub cover_filename: Option<String>,
    pub cover_size: Option<u64>,
    pub size: u64,
    pub bitrate: Option<u32>,
    pub duration: Option<u32>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u32>,
    pub vbr: Option<bool>,
    pub peer_speed: u32,
    pub free_upload_slots: u8,
    pub extension: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SoulseekDownloadRequest {
    pub username: String,
    pub filename: String,
    pub cover_filename: Option<String>,
    pub cover_size: Option<u64>,
    pub size: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SoulseekCoverRequest {
    pub username: String,
    pub cover_filename: String,
    pub cover_size: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SoulseekPromotePreviewRequest {
    pub local_path: String,
    pub cover_filename: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SoulseekCancelPreviewRequest {
    pub username: String,
    pub filename: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SoulseekPlayPreviewRequest {
    pub username: String,
    pub filename: String,
    pub size: u64,
}

struct PendingSearchFile {
    result: SoulseekSearchResult,
    parent: Option<String>,
}

struct PendingCoverFile {
    filename: String,
    size: u64,
    priority: usize,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SoulseekDownloadEvent {
    transfer_id: String,
    username: String,
    filename: String,
    basename: String,
    state: String,
    bytes_downloaded: Option<u64>,
    total_bytes: Option<u64>,
    speed_bytes_per_sec: Option<f64>,
    queue_position: Option<u32>,
    local_path: Option<String>,
    error: Option<String>,
}

#[tauri::command]
pub fn soulseek_get_status(
    state: tauri::State<'_, SoulseekState>,
    library: tauri::State<'_, LibraryState>,
) -> Result<SoulseekStatus, String> {
    let settings = state.settings(&library)?;
    let configured = SoulseekState::desired_config(&settings).is_some();
    let active_session = state.maybe_client_for_settings(&settings).is_some();
    let username = if settings.soulseek_username.trim().is_empty() {
        None
    } else {
        Some(settings.soulseek_username.trim().to_string())
    };

    Ok(SoulseekStatus {
        enabled: settings.soulseek_enabled,
        configured,
        username,
        active_session,
    })
}

#[tauri::command]
pub async fn soulseek_search(
    query: String,
    state: tauri::State<'_, SoulseekState>,
    library: tauri::State<'_, LibraryState>,
    app: tauri::AppHandle,
    generation: u64,
) -> Result<Vec<SoulseekSearchResult>, String> {
    assert_generation(&app, generation)?;
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let epoch = state.epoch.load(Ordering::SeqCst);
    let settings = state.settings(&library)?;
    let Some(client) = state.maybe_client_for_settings(&settings) else {
        return Ok(Vec::new());
    };

    client.connect().await.map_err(|e| e.to_string())?;
    state.check_epoch(epoch)?;
    assert_generation(&app, generation)?;
    let results = client
        .search(query, Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    Ok(flatten_search_results(results))
}

#[tauri::command]
pub async fn soulseek_download(
    request: SoulseekDownloadRequest,
    app: tauri::AppHandle,
    state: tauri::State<'_, SoulseekState>,
    library: tauri::State<'_, LibraryState>,
    generation: u64,
) -> Result<String, String> {
    assert_generation(&app, generation)?;
    validate_remote_filename(&request.filename)?;
    if let Some(cover) = &request.cover_filename {
        validate_remote_filename(cover)?;
    }

    eprintln!(
        "[soulseek] download request: user={} remote={} size={} cover={:?} cover_size={:?}",
        request.username,
        request.filename,
        request.size,
        request.cover_filename,
        request.cover_size,
    );

    let epoch = state.epoch.load(Ordering::SeqCst);
    let settings = state.settings(&library)?;
    let client = state.client_for_download(&settings)?;
    eprintln!(
        "[soulseek] connecting download session: user={} remote={}",
        request.username, request.filename
    );
    client.connect().await.map_err(|e| {
        eprintln!(
            "[soulseek] connect failed: user={} remote={} error={}",
            request.username, request.filename, e
        );
        e.to_string()
    })?;

    state.check_epoch(epoch)?;
    assert_generation(&app, generation)?;
    // Use temporary download directory for organizing files after download
    let temp_download_root = state.cache_root.join("downloads");
    let download_dir =
        build_download_directory(&temp_download_root, &request.username, &request.filename);
    eprintln!(
        "[soulseek] resolved local download dir: user={} remote={} local_dir={}",
        request.username,
        request.filename,
        download_dir.display()
    );
    std::fs::create_dir_all(&download_dir).map_err(|e| {
        eprintln!(
            "[soulseek] failed to create local download dir: user={} remote={} local_dir={} error={}",
            request.username,
            request.filename,
            download_dir.display(),
            e
        );
        e.to_string()
    })?;

    let soulseek_path = soulseek_rs::SoulseekPath::from_wire(request.filename.clone());
    eprintln!(
        "[soulseek] queueing main download: user={} remote={} local_dir={}",
        request.username,
        request.filename,
        download_dir.display()
    );
    let (download, handle) = client
        .download(
            soulseek_path,
            request.username.clone(),
            request.size,
            download_dir.to_string_lossy().to_string(),
            Some(Duration::from_secs(30)),
            Some(Duration::from_secs(600)),
        )
        .map_err(|e| {
            eprintln!(
                "[soulseek] client.download failed: user={} remote={} size={} local_dir={} error={}",
                request.username,
                request.filename,
                request.size,
                download_dir.display(),
                e
            );
            e.to_string()
        })?;

    if let (Some(cover_filename), Some(cover_size)) =
        (request.cover_filename.as_ref(), request.cover_size)
    {
        if cover_size > 0 && cover_filename != &request.filename {
            eprintln!(
                "[soulseek] queueing sidecar cover: user={} remote={} cover={} cover_size={} local_dir={}",
                request.username,
                request.filename,
                cover_filename,
                cover_size,
                download_dir.display()
            );
            if let Err(error) = queue_sidecar_cover_download(
                &client,
                &request.username,
                cover_filename,
                cover_size,
                &download_dir,
                &state,
                epoch,
            ) {
                eprintln!(
                    "[soulseek] failed to queue sidecar cover for {}: {}",
                    cover_filename, error
                );
            }
        }
    }

    let transfer_id = format!(
        "{}-{}",
        request.username,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );

    let data_dir_pathbuf = library.data_dir().to_path_buf();
    state.register_preview_canceller(
        &request.username,
        &request.filename,
        &transfer_id,
        handle.cancel_handle(),
        epoch,
    )?;
    let cancel_registry = state.preview_canceller_registry();
    tauri::async_runtime::spawn(monitor_download(
        app.clone(),
        "soulseek-download",
        transfer_id.clone(),
        download,
        handle,
        false,
        data_dir_pathbuf,
        request
            .cover_filename
            .as_ref()
            .filter(|cover| *cover != &request.filename)
            .cloned(),
        Some(cancel_registry),
        generation,
    ));

    eprintln!(
        "[soulseek] main download queued: transfer_id={} user={} remote={} local_dir={}",
        transfer_id,
        request.username,
        request.filename,
        download_dir.display()
    );

    Ok(transfer_id)
}

#[tauri::command]
pub async fn soulseek_preview(
    request: SoulseekDownloadRequest,
    app: tauri::AppHandle,
    state: tauri::State<'_, SoulseekState>,
    library: tauri::State<'_, LibraryState>,
    generation: u64,
) -> Result<String, String> {
    assert_generation(&app, generation)?;
    validate_remote_filename(&request.filename)?;
    if let Some(cover) = &request.cover_filename {
        validate_remote_filename(cover)?;
    }

    eprintln!(
        "[soulseek] preview request: user={} remote={} size={}",
        request.username, request.filename, request.size,
    );

    let epoch = state.epoch.load(Ordering::SeqCst);
    let settings = state.settings(&library)?;
    let client = state.client_for_download(&settings)?;
    client.connect().await.map_err(|e| e.to_string())?;
    state.check_epoch(epoch)?;
    assert_generation(&app, generation)?;

    let preview_root = state.cache_root.join("previews").join(format!(
        "{}-{}",
        epoch,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let preview_dir = build_download_directory(&preview_root, &request.username, &request.filename);
    fs::create_dir_all(&preview_dir).map_err(|e| e.to_string())?;

    let local_path = preview_dir.join(soulseek_basename(&request.filename));
    if local_path.exists() {
        let _ = fs::remove_file(&local_path);
    }

    let soulseek_path = soulseek_rs::SoulseekPath::from_wire(request.filename.clone());
    let (download, handle) = client
        .download(
            soulseek_path,
            request.username.clone(),
            request.size,
            preview_dir.to_string_lossy().to_string(),
            Some(Duration::from_secs(30)),
            Some(Duration::from_secs(600)),
        )
        .map_err(|e| e.to_string())?;

    if let (Some(cover_filename), Some(cover_size)) =
        (request.cover_filename.as_ref(), request.cover_size)
    {
        if cover_size > 0 && cover_filename != &request.filename {
            if let Err(error) = queue_sidecar_cover_download(
                &client,
                &request.username,
                cover_filename,
                cover_size,
                &preview_dir,
                &state,
                epoch,
            ) {
                eprintln!(
                    "[soulseek] failed to queue preview sidecar cover for {}: {}",
                    cover_filename, error
                );
            }
        }
    }

    state.preview_paths.lock().unwrap().insert(
        SoulseekState::preview_key(&request.username, &request.filename),
        local_path.clone(),
    );
    set_preview_transfer_state(&local_path, PreviewTransferState::Active);

    let transfer_id = format!(
        "preview-{}-{}",
        request.username,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );

    let data_dir_pathbuf = library.data_dir().to_path_buf();
    let preview_cancel_registry = state.preview_canceller_registry();
    let preview_canceller = handle.cancel_handle();
    state.register_preview_canceller(
        &request.username,
        &request.filename,
        &transfer_id,
        preview_canceller,
        epoch,
    )?;
    tauri::async_runtime::spawn(monitor_download(
        app.clone(),
        "soulseek-preview",
        transfer_id.clone(),
        download,
        handle,
        true,
        data_dir_pathbuf,
        request
            .cover_filename
            .as_ref()
            .filter(|cover| *cover != &request.filename)
            .cloned(),
        Some(preview_cancel_registry),
        generation,
    ));

    Ok(transfer_id)
}

#[tauri::command]
pub fn soulseek_cancel_preview(
    request: SoulseekCancelPreviewRequest,
    state: tauri::State<'_, SoulseekState>,
) -> Result<bool, String> {
    let mut cancelled = false;
    while let Some(canceller) = state.take_preview_canceller(&request.username, &request.filename) {
        canceller.cancel();
        cancelled = true;
    }
    Ok(cancelled)
}

#[tauri::command]
pub fn soulseek_play_preview(
    request: SoulseekPlayPreviewRequest,
    library: tauri::State<'_, LibraryState>,
    playback: tauri::State<'_, crate::playback::PlaybackState>,
) -> Result<(), String> {
    let cache_path = preview_cache_path(&library, &request.username, &request.filename);
    if !cache_path.exists() {
        return Err(format!(
            "Soulseek preview cache does not exist yet: {}",
            cache_path.display()
        ));
    }

    playback.play_track_source(crate::library::TrackSource::GrowingFile {
        resource: crate::plugin_sources::ResourceIdentity {
            plugin_id: "player.soulseek".into(),
            resource_id: serde_json::to_string(&(request.username, request.filename)).unwrap(),
            title: "Soulseek preview".into(),
            artist: String::new(),
            album: String::new(),
            duration_seconds: None,
            cover_hash: None,
        },
        cache_path,
    })
}

#[tauri::command]
pub fn soulseek_promote_preview(
    request: SoulseekPromotePreviewRequest,
    app: tauri::AppHandle,
    library: tauri::State<'_, LibraryState>,
) -> Result<String, String> {
    use tauri::Emitter;

    let preview_path = PathBuf::from(&request.local_path);
    if preview_path.starts_with(library.data_dir())
        && !preview_path.starts_with(preview_cache_root(&library))
        && preview_path.exists()
    {
        let _ = app.emit("library-changed", ());
        return Ok(request.local_path);
    }

    if !preview_path.exists() {
        return Err(format!(
            "Preview file not found: {}",
            preview_path.display()
        ));
    }

    let source_dir = preview_path.parent().map(Path::to_path_buf);
    let staging_copy = preview_path.with_extension(format!(
        "import-{}",
        preview_path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
    ));
    fs::copy(&preview_path, &staging_copy).map_err(|e| e.to_string())?;
    let final_path = reorganize_downloaded_file(&staging_copy, library.data_dir())?;

    if let (Some(source_dir), Some(target_dir)) = (source_dir.as_deref(), final_path.parent()) {
        move_downloaded_sidecar_cover(source_dir, target_dir, request.cover_filename.as_deref())?;
    }

    {
        let conn = library.conn();
        let conn = conn.lock().unwrap();
        crate::library::index_file(&conn, library.data_dir(), &final_path)
            .map_err(|e| e.to_string())?;
    }

    let _ = app.emit("library-changed", ());
    Ok(final_path.to_string_lossy().to_string())
}

/// Downloads a search result's cover image and returns its hash in the
/// cover store (see covers.rs).
#[tauri::command]
pub async fn soulseek_fetch_cover(
    request: SoulseekCoverRequest,
    state: tauri::State<'_, SoulseekState>,
    library: tauri::State<'_, LibraryState>,
    app: tauri::AppHandle,
    generation: u64,
) -> Result<Option<String>, String> {
    assert_generation(&app, generation)?;
    validate_remote_filename(&request.cover_filename)?;
    if request.cover_size > 32 * 1024 * 1024 {
        return Err("Cover exceeds 32 MiB".into());
    }

    if request.cover_size == 0 {
        return Ok(None);
    }

    let epoch = state.epoch.load(Ordering::SeqCst);
    let settings = state.settings(&library)?;
    let client = state.client_for_download(&settings)?;
    client.connect().await.map_err(|e| e.to_string())?;
    state.check_epoch(epoch)?;
    assert_generation(&app, generation)?;

    let cache_root = state.cache_root.join("covers");
    let download_dir =
        build_download_directory(&cache_root, &request.username, &request.cover_filename);
    fs::create_dir_all(&download_dir).map_err(|e| e.to_string())?;

    let local_path = download_dir.join(soulseek_basename(&request.cover_filename));
    let has_cached_cover = local_path
        .metadata()
        .map(|metadata| metadata.is_file() && metadata.len() == request.cover_size)
        .unwrap_or(false);

    if has_cached_cover {
        return store_cover_preview(local_path, library.data_dir().to_path_buf()).await;
    }

    if local_path.exists() {
        let _ = fs::remove_file(&local_path);
    }

    let soulseek_path = soulseek_rs::SoulseekPath::from_wire(request.cover_filename.clone());
    let (download, mut handle) = client
        .download(
            soulseek_path,
            request.username.clone(),
            request.cover_size,
            download_dir.to_string_lossy().to_string(),
            Some(Duration::from_secs(30)),
            Some(Duration::from_secs(600)),
        )
        .map_err(|e| e.to_string())?;

    let _scope = state.cancellation_scope(
        &request.username,
        &request.cover_filename,
        handle.cancel_handle(),
        epoch,
    )?;
    while let Some(status) = handle.recv().await {
        match status {
            DownloadStatus::QueuedLocally
            | DownloadStatus::QueuedRemotely { .. }
            | DownloadStatus::InProgress { .. } => {}
            DownloadStatus::Completed => {
                return store_cover_preview(local_path, library.data_dir().to_path_buf()).await;
            }
            DownloadStatus::Failed => {
                eprintln!(
                    "[soulseek] cover preview download failed: user={} cover={} destination_dir={}",
                    download.username, download.filename, download.download_directory
                );
                return Ok(None);
            }
            DownloadStatus::TimedOut => {
                eprintln!(
                    "[soulseek] cover preview download timed out: user={} cover={} destination_dir={}",
                    download.username,
                    download.filename,
                    download.download_directory
                );
                return Ok(None);
            }
            DownloadStatus::Cancelled => {
                return Ok(None);
            }
        }
    }

    Ok(None)
}

fn flatten_search_results(results: Vec<soulseek_rs::SearchResult>) -> Vec<SoulseekSearchResult> {
    let mut audio_files = Vec::new();
    let mut covers: HashMap<(String, String), PendingCoverFile> = HashMap::new();

    for result in results {
        let free_upload_slots = result.slots;
        let peer_speed = result.speed;
        for file in result.files {
            let mapped = map_search_file(file, free_upload_slots, peer_speed);
            if let (Some(parent), Some(priority)) = (
                mapped.parent.as_ref(),
                cover_file_priority(&mapped.result.basename, mapped.result.extension.as_deref()),
            ) {
                let key = (mapped.result.username.clone(), parent.clone());
                let candidate = PendingCoverFile {
                    filename: mapped.result.filename.clone(),
                    size: mapped.result.size,
                    priority,
                };
                match covers.get(&key) {
                    Some(current) if current.priority <= candidate.priority => {}
                    _ => {
                        covers.insert(key, candidate);
                    }
                }
                continue;
            }

            if is_audio_result(&mapped.result) {
                audio_files.push(mapped);
            }
        }
    }

    let mut files = Vec::with_capacity(audio_files.len());
    for mut mapped in audio_files {
        if let Some(parent) = mapped.parent.as_ref() {
            if let Some(cover) = covers.get(&(mapped.result.username.clone(), parent.clone())) {
                mapped.result.cover_filename = Some(cover.filename.clone());
                mapped.result.cover_size = Some(cover.size);
            }
        }
        files.push(mapped.result);
    }

    files.sort_by(|left, right| {
        right
            .free_upload_slots
            .cmp(&left.free_upload_slots)
            .then_with(|| right.peer_speed.cmp(&left.peer_speed))
            .then_with(|| left.basename.cmp(&right.basename))
            .then_with(|| left.username.cmp(&right.username))
    });

    files
}

fn map_search_file(file: File, free_upload_slots: u8, peer_speed: u32) -> PendingSearchFile {
    let basename = file.name.filename().to_string();
    let filename = file.name.to_string();
    let extension = Path::new(&basename)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());

    PendingSearchFile {
        parent: soulseek_parent(&filename),
        result: SoulseekSearchResult {
            username: file.username,
            filename,
            basename,
            cover_filename: None,
            cover_size: None,
            size: file.size,
            bitrate: file.attributes.bitrate,
            duration: file.attributes.duration,
            sample_rate: file.attributes.sample_rate,
            bit_depth: file.attributes.bit_depth,
            vbr: file.attributes.vbr,
            peer_speed,
            free_upload_slots,
            extension,
        },
    }
}

fn is_audio_result(result: &SoulseekSearchResult) -> bool {
    result
        .extension
        .as_deref()
        .map(|extension| AUDIO_EXTENSIONS.contains(&extension))
        .unwrap_or(false)
        || result.bitrate.is_some()
        || result.duration.is_some()
        || result.sample_rate.is_some()
}

fn cover_file_priority(basename: &str, extension: Option<&str>) -> Option<usize> {
    let extension = extension?;
    if !COVER_EXTENSIONS.contains(&extension) {
        return None;
    }

    let stem = Path::new(basename)
        .file_stem()
        .and_then(|value| value.to_str())?
        .to_ascii_lowercase();

    COVER_BASENAME_PRIORITY
        .iter()
        .position(|candidate| *candidate == stem)
}

fn soulseek_parent(filename: &str) -> Option<String> {
    let mut segments: Vec<&str> = filename
        .split(['\\', '/'])
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.len() <= 1 {
        return None;
    }
    segments.pop();
    Some(segments.join("/"))
}

fn soulseek_basename(filename: &str) -> &str {
    filename
        .rsplit(['\\', '/'])
        .next()
        .filter(|segment| !segment.is_empty())
        .unwrap_or(filename)
}

fn build_download_directory(root: &Path, username: &str, remote_filename: &str) -> PathBuf {
    let mut directory = root.join(sanitize_path_segment(username));
    let mut segments: Vec<&str> = remote_filename
        .split(['\\', '/'])
        .filter(|segment| !segment.is_empty())
        .collect();

    if !segments.is_empty() {
        segments.pop();
    }

    for segment in segments {
        directory = directory.join(sanitize_path_segment(segment));
    }

    directory
}

/// Reorganizes a downloaded file to its final location based on metadata.
/// Moves file from its temporary download location to a metadata-based path.
///
/// Path structure:
/// - If both artist and album exist: data_dir/[Artist]/[Album]/filename
/// - If only artist exists: data_dir/[Artist]/filename
/// - Otherwise: data_dir/filename
fn reorganize_downloaded_file(file_path: &Path, data_dir: &Path) -> Result<PathBuf, String> {
    if !file_path.exists() {
        return Err(format!(
            "Downloaded file not found: {}",
            file_path.display()
        ));
    }

    // Extract metadata
    let (artist, album) = extract_audio_metadata(file_path);

    // Build target directory
    let target_dir = build_metadata_based_path(data_dir, artist.as_deref(), album.as_deref());

    // Ensure target directory exists
    fs::create_dir_all(&target_dir)
        .map_err(|e| format!("Failed to create target directory: {}", e))?;

    // Get filename
    let filename = file_path
        .file_name()
        .ok_or_else(|| "Failed to get filename".to_string())?;

    // Build final path
    let final_path = target_dir.join(&filename);

    // If file already exists at target, skip
    if final_path.exists() {
        eprintln!(
            "[soulseek] file already exists at target location, removing download duplicate: {} -> {}",
            file_path.display(),
            final_path.display()
        );
        let _ = fs::remove_file(file_path);
        return Ok(final_path);
    }

    // Move file to final location
    fs::rename(file_path, &final_path)
        .map_err(|e| format!("Failed to move file to final location: {}", e))?;

    eprintln!(
        "[soulseek] reorganized downloaded file: {} -> {} (artist={}, album={})",
        file_path.display(),
        final_path.display(),
        artist.as_deref().unwrap_or(""),
        album.as_deref().unwrap_or("")
    );

    Ok(final_path)
}

fn move_downloaded_aux_file(source_path: &Path, target_path: &Path) -> Result<(), String> {
    if !source_path.exists() {
        return Ok(());
    }

    if target_path.exists() {
        let _ = fs::remove_file(source_path);
        return Ok(());
    }

    match fs::rename(source_path, target_path) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(source_path, target_path).map_err(|e| {
                format!(
                    "Failed to copy auxiliary download file {} -> {}: {}",
                    source_path.display(),
                    target_path.display(),
                    e
                )
            })?;
            fs::remove_file(source_path).map_err(|e| {
                format!(
                    "Failed to remove original auxiliary download file {}: {}",
                    source_path.display(),
                    e
                )
            })?;
            Ok(())
        }
    }
}

fn move_downloaded_sidecar_cover(
    source_dir: &Path,
    target_dir: &Path,
    cover_filename: Option<&str>,
) -> Result<(), String> {
    let Some(cover_filename) = cover_filename else {
        return Ok(());
    };

    let cover_basename = soulseek_basename(cover_filename);
    let source_path = source_dir.join(cover_basename);
    let target_path = target_dir.join(cover_basename);
    move_downloaded_aux_file(&source_path, &target_path)
}

fn preview_cache_root(library: &LibraryState) -> PathBuf {
    library
        .data_dir()
        .parent()
        .unwrap_or_else(|| library.data_dir())
        .join(".soulseek-preview-cache")
}

fn preview_cache_path(library: &LibraryState, username: &str, filename: &str) -> PathBuf {
    let preview_root = preview_cache_root(library);
    let preview_dir = build_download_directory(&preview_root, username, filename);
    preview_dir.join(soulseek_basename(filename))
}

fn resolved_download_path(download: &Download) -> String {
    Path::new(&download.download_directory)
        .join(download.filename.filename())
        .to_string_lossy()
        .to_string()
}

fn queue_sidecar_cover_download(
    client: &Client,
    username: &str,
    cover_filename: &str,
    cover_size: u64,
    download_dir: &Path,
    state: &SoulseekState,
    epoch: u64,
) -> Result<(), String> {
    let destination = download_dir.join(soulseek_basename(cover_filename));
    if destination.exists() {
        eprintln!(
            "[soulseek] skipping sidecar cover download because file already exists: user={} cover={} destination={}",
            username,
            cover_filename,
            destination.display()
        );
        return Ok(());
    }

    let soulseek_path = soulseek_rs::SoulseekPath::from_wire(cover_filename.to_string());
    let (download, handle) = client
        .download(
            soulseek_path,
            username.to_string(),
            cover_size,
            download_dir.to_string_lossy().to_string(),
            Some(Duration::from_secs(30)),
            Some(Duration::from_secs(600)),
        )
        .map_err(|e| {
            eprintln!(
                "[soulseek] client.download failed for sidecar cover: user={} cover={} cover_size={} local_dir={} error={}",
                username,
                cover_filename,
                cover_size,
                download_dir.display(),
                e
            );
            e.to_string()
        })?;

    eprintln!(
        "[soulseek] sidecar cover queued: user={} cover={} destination={}",
        username,
        cover_filename,
        destination.display()
    );

    let scope =
        state.cancellation_scope(username, cover_filename, handle.cancel_handle(), epoch)?;
    tauri::async_runtime::spawn(async move {
        let _scope = scope;
        monitor_aux_download(download, handle).await;
    });
    Ok(())
}

/// Puts a downloaded cover preview into the cover store and returns its hash.
async fn store_cover_preview(path: PathBuf, data_dir: PathBuf) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || match fs::read(&path) {
        Ok(data) => Ok(crate::covers::store(&data_dir, &data)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    })
    .await
    .map_err(|error| error.to_string())?
}

async fn monitor_aux_download(download: Download, mut handle: DownloadHandle) {
    while let Some(status) = handle.recv().await {
        match status {
            DownloadStatus::QueuedLocally => {
                eprintln!(
                    "[soulseek] sidecar cover queued locally: user={} cover={}",
                    download.username, download.filename
                );
            }
            DownloadStatus::QueuedRemotely { place } => {
                eprintln!(
                    "[soulseek] sidecar cover queued remotely: user={} cover={} place={:?}",
                    download.username, download.filename, place
                );
            }
            DownloadStatus::InProgress {
                bytes_downloaded,
                total_bytes,
                speed_bytes_per_sec,
            } => {
                eprintln!(
                    "[soulseek] sidecar cover progress: user={} cover={} downloaded={} total={} speed={:.1}",
                    download.username,
                    download.filename,
                    bytes_downloaded,
                    total_bytes,
                    speed_bytes_per_sec
                );
            }
            DownloadStatus::Completed => {
                eprintln!(
                    "[soulseek] sidecar cover completed: user={} cover={} destination={}",
                    download.username,
                    download.filename,
                    Path::new(&download.download_directory)
                        .join(download.filename.filename())
                        .display()
                );
                break;
            }
            DownloadStatus::Failed => {
                eprintln!(
                    "[soulseek] sidecar cover failed: user={} cover={} destination_dir={}",
                    download.username, download.filename, download.download_directory
                );
                break;
            }
            DownloadStatus::TimedOut => {
                eprintln!(
                    "[soulseek] sidecar cover timed out: user={} cover={} destination_dir={}",
                    download.username, download.filename, download.download_directory
                );
                break;
            }
            DownloadStatus::Cancelled => {
                eprintln!(
                    "[soulseek] sidecar cover cancelled: user={} cover={} destination_dir={}",
                    download.username, download.filename, download.download_directory
                );
                break;
            }
        }
    }

    eprintln!(
        "[soulseek] sidecar cover monitor ended: user={} cover={} destination_dir={}",
        download.username, download.filename, download.download_directory
    );
}

async fn monitor_download(
    app: tauri::AppHandle,
    event_name: &'static str,
    transfer_id: String,
    download: Download,
    mut handle: DownloadHandle,
    is_preview: bool,
    data_dir: PathBuf,
    sidecar_cover_filename: Option<String>,
    preview_cancel_registry: Option<Arc<Mutex<HashMap<String, DownloadCanceller>>>>,
    generation: u64,
) {
    use tauri::Emitter;

    let local_path = resolved_download_path(&download);

    eprintln!(
        "[soulseek] download monitor started: transfer_id={} user={} remote={} destination_dir={} size={}",
        transfer_id,
        download.username,
        download.filename,
        download.download_directory,
        download.size
    );

    let mut last_progress = std::time::Instant::now() - Duration::from_secs(1);
    while let Some(status) = handle.recv().await {
        let basename = download.filename.filename().to_string();
        let filename = download.filename.to_string();
        let mut event = SoulseekDownloadEvent {
            transfer_id: transfer_id.clone(),
            username: download.username.clone(),
            filename,
            basename,
            state: String::new(),
            bytes_downloaded: None,
            total_bytes: Some(download.size),
            speed_bytes_per_sec: None,
            queue_position: None,
            local_path: Some(local_path.clone()),
            error: None,
        };

        let terminal = match status {
            DownloadStatus::QueuedLocally => {
                eprintln!(
                    "[soulseek] download status queued locally: transfer_id={} user={} remote={}",
                    transfer_id, download.username, download.filename
                );
                event.state = "queued_local".to_string();
                if is_preview {
                    set_preview_transfer_state(
                        Path::new(&local_path),
                        PreviewTransferState::Active,
                    );
                }
                false
            }
            DownloadStatus::QueuedRemotely { place } => {
                eprintln!(
                    "[soulseek] download status queued remotely: transfer_id={} user={} remote={} place={:?}",
                    transfer_id,
                    download.username,
                    download.filename,
                    place
                );
                event.state = "queued_remote".to_string();
                event.queue_position = place;
                if is_preview {
                    set_preview_transfer_state(
                        Path::new(&local_path),
                        PreviewTransferState::Active,
                    );
                }
                false
            }
            DownloadStatus::InProgress {
                bytes_downloaded,
                total_bytes,
                speed_bytes_per_sec,
            } => {
                eprintln!(
                    "[soulseek] download progress: transfer_id={} user={} remote={} downloaded={} total={} speed={:.1}",
                    transfer_id,
                    download.username,
                    download.filename,
                    bytes_downloaded,
                    total_bytes,
                    speed_bytes_per_sec
                );
                event.state = "progress".to_string();
                event.bytes_downloaded = Some(bytes_downloaded);
                event.total_bytes = Some(total_bytes);
                event.speed_bytes_per_sec = Some(speed_bytes_per_sec);
                if is_preview {
                    set_preview_transfer_state(
                        Path::new(&local_path),
                        PreviewTransferState::Active,
                    );
                }
                false
            }
            DownloadStatus::Completed => {
                eprintln!(
                    "[soulseek] download completed: transfer_id={} user={} remote={} local_path={}",
                    transfer_id, download.username, download.filename, local_path
                );
                event.state = "completed".to_string();
                event.bytes_downloaded = Some(download.size);

                // Reorganize file based on metadata (not for previews)
                if !is_preview {
                    let file_path = Path::new(&local_path);
                    match reorganize_downloaded_file(file_path, &data_dir) {
                        Ok(final_path) => {
                            if let (Some(source_dir), Some(target_dir)) =
                                (file_path.parent(), final_path.parent())
                            {
                                if let Err(error) = move_downloaded_sidecar_cover(
                                    source_dir,
                                    target_dir,
                                    sidecar_cover_filename.as_deref(),
                                ) {
                                    eprintln!(
                                        "[soulseek] failed to move sidecar cover for {}: {}",
                                        local_path, error
                                    );
                                }
                            }
                            let final_path_str = final_path.to_string_lossy().to_string();
                            event.local_path = Some(final_path_str);
                            eprintln!(
                                "[soulseek] file reorganized successfully: {} -> {}",
                                local_path,
                                final_path.display()
                            );
                        }
                        Err(e) => {
                            eprintln!(
                                "[soulseek] failed to reorganize file: {} - {}",
                                local_path, e
                            );
                        }
                    }
                }

                if is_preview {
                    set_preview_transfer_state(
                        Path::new(&local_path),
                        PreviewTransferState::Completed,
                    );
                }
                true
            }
            DownloadStatus::Failed => {
                eprintln!(
                    "[soulseek] download failed: transfer_id={} user={} remote={} destination_dir={}",
                    transfer_id,
                    download.username,
                    download.filename,
                    download.download_directory
                );
                event.state = "failed".to_string();
                event.error = Some("Download failed".to_string());
                if is_preview {
                    set_preview_transfer_state(
                        Path::new(&local_path),
                        PreviewTransferState::Failed,
                    );
                }
                true
            }
            DownloadStatus::TimedOut => {
                eprintln!(
                    "[soulseek] download timed out: transfer_id={} user={} remote={} destination_dir={}",
                    transfer_id,
                    download.username,
                    download.filename,
                    download.download_directory
                );
                event.state = "timed_out".to_string();
                event.error = Some("Download timed out".to_string());
                if is_preview {
                    set_preview_transfer_state(
                        Path::new(&local_path),
                        PreviewTransferState::TimedOut,
                    );
                }
                true
            }
            DownloadStatus::Cancelled => {
                eprintln!(
                    "[soulseek] download cancelled: transfer_id={} user={} remote={} destination_dir={}",
                    transfer_id,
                    download.username,
                    download.filename,
                    download.download_directory
                );
                event.state = "cancelled".to_string();
                event.error = Some("Download cancelled".to_string());
                if is_preview {
                    set_preview_transfer_state(
                        Path::new(&local_path),
                        PreviewTransferState::Cancelled,
                    );
                }
                true
            }
        };

        if event.state == "progress" && last_progress.elapsed() < Duration::from_millis(200) {
            continue;
        }
        if event.state == "progress" {
            last_progress = std::time::Instant::now();
        }
        crate::plugins::emit_generation_event(
            &app,
            "player.soulseek",
            generation,
            event_name,
            serde_json::to_value(&event).unwrap_or_default(),
        );
        if let Err(error) = app.emit(event_name, event) {
            eprintln!(
                "[soulseek] failed to emit download event: transfer_id={} user={} remote={} error={}",
                transfer_id,
                download.username,
                download.filename,
                error
            );
        }
        if terminal {
            break;
        }
    }

    if is_preview
        && crate::plugin_sources::transfer_state(Path::new(&local_path))
            == Some(PreviewTransferState::Active)
    {
        set_preview_transfer_state(Path::new(&local_path), PreviewTransferState::Failed);
        crate::plugins::emit_generation_event(
            &app,
            "player.soulseek",
            generation,
            event_name,
            serde_json::json!({"transferId":transfer_id,"username":download.username,"filename":download.filename.to_string(),"state":"failed","error":"Transfer ended unexpectedly"}),
        );
    }
    eprintln!(
        "[soulseek] download monitor ended: transfer_id={} user={} remote={} destination_dir={}",
        transfer_id, download.username, download.filename, download.download_directory
    );

    {
        if let Some(registry) = preview_cancel_registry {
            registry.lock().unwrap().remove(&format!(
                "{}\u{0}{}",
                SoulseekState::preview_key(&download.username, &download.filename.to_string()),
                transfer_id
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_basenames_cannot_escape_staging() {
        assert!(validate_remote_filename("Artist\\Album\\song.flac").is_ok());
        assert!(validate_remote_filename("../..").is_err());
        assert!(validate_remote_filename("C:escape.mp3").is_err());
        assert!(validate_remote_filename("bad\0.mp3").is_err());
    }
}
