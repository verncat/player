use base64::{engine::general_purpose::STANDARD as B64, Engine};
use crate::library::{DeviceSettings, LibraryState};
use lofty::prelude::Accessor;
use lofty::probe::Probe;
use serde::{Deserialize, Serialize};
use soulseek_rs::types::Download;
use soulseek_rs::{Client, DownloadHandle, DownloadStatus, File};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "opus", "aac", "m4a", "wav", "wv", "ape", "alac", "aiff",
];

const COVER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];

const COVER_BASENAME_PRIORITY: &[&str] = &["cover", "folder", "front", "album", "art"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewTransferState {
    Active,
    Completed,
    Failed,
    TimedOut,
    Cancelled,
}

static PREVIEW_TRANSFERS: LazyLock<Mutex<HashMap<String, PreviewTransferState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn preview_transfer_state(path: &Path) -> Option<PreviewTransferState> {
    PREVIEW_TRANSFERS
        .lock()
        .unwrap()
        .get(&path.to_string_lossy().to_string())
        .copied()
}

fn set_preview_transfer_state(path: &Path, state: PreviewTransferState) {
    PREVIEW_TRANSFERS
        .lock()
        .unwrap()
        .insert(path.to_string_lossy().to_string(), state);
}

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
    inner: Arc<Mutex<SoulseekInner>>,
}

impl SoulseekState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SoulseekInner::default())),
        }
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
    let settings = library.get_device_settings().map_err(|e| e.to_string())?;
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
) -> Result<Vec<SoulseekSearchResult>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let settings = library.get_device_settings().map_err(|e| e.to_string())?;
    let Some(client) = state.maybe_client_for_settings(&settings) else {
        return Ok(Vec::new());
    };

    client.connect().await.map_err(|e| e.to_string())?;
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
) -> Result<String, String> {
    eprintln!(
        "[soulseek] download request: user={} remote={} size={} cover={:?} cover_size={:?}",
        request.username,
        request.filename,
        request.size,
        request.cover_filename,
        request.cover_size,
    );

    let settings = library.get_device_settings().map_err(|e| e.to_string())?;
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

    // Use temporary download directory for organizing files after download
    let data_dir = library.data_dir();
    let temp_download_root = data_dir.join(".soulseek-temp");
    let download_dir = build_download_directory(&temp_download_root, &request.username, &request.filename);
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
    tauri::async_runtime::spawn(monitor_download(
        app,
        "soulseek-download",
        transfer_id.clone(),
        download,
        handle,
        false,
        data_dir_pathbuf,
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
) -> Result<String, String> {
    eprintln!(
        "[soulseek] preview request: user={} remote={} size={}",
        request.username,
        request.filename,
        request.size,
    );

    let settings = library.get_device_settings().map_err(|e| e.to_string())?;
    let client = state.client_for_download(&settings)?;
    client.connect().await.map_err(|e| e.to_string())?;

    let preview_root = preview_cache_root(&library);
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
    tauri::async_runtime::spawn(monitor_download(
        app,
        "soulseek-preview",
        transfer_id.clone(),
        download,
        handle,
        true,
        data_dir_pathbuf,
    ));

    Ok(transfer_id)
}

#[tauri::command]
pub async fn soulseek_fetch_cover(
    request: SoulseekCoverRequest,
    state: tauri::State<'_, SoulseekState>,
    library: tauri::State<'_, LibraryState>,
) -> Result<Option<String>, String> {
    if request.cover_size == 0 {
        return Ok(None);
    }

    let settings = library.get_device_settings().map_err(|e| e.to_string())?;
    let client = state.client_for_download(&settings)?;
    client.connect().await.map_err(|e| e.to_string())?;

    let cache_root = library.data_dir().join(".soulseek-cover-cache");
    let download_dir = build_download_directory(&cache_root, &request.username, &request.cover_filename);
    fs::create_dir_all(&download_dir).map_err(|e| e.to_string())?;

    let local_path = download_dir.join(soulseek_basename(&request.cover_filename));
    let has_cached_cover = local_path
        .metadata()
        .map(|metadata| metadata.is_file() && metadata.len() == request.cover_size)
        .unwrap_or(false);

    if has_cached_cover {
        return encode_cover_as_data_url(&local_path, &request.cover_filename);
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

    while let Some(status) = handle.recv().await {
        match status {
            DownloadStatus::QueuedLocally
            | DownloadStatus::QueuedRemotely { .. }
            | DownloadStatus::InProgress { .. } => {}
            DownloadStatus::Completed => {
                return encode_cover_as_data_url(&local_path, &request.cover_filename);
            }
            DownloadStatus::Failed => {
                eprintln!(
                    "[soulseek] cover preview download failed: user={} cover={} destination_dir={}",
                    download.username,
                    download.filename,
                    download.download_directory
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

fn sanitize_path_segment(segment: &str) -> String {
    let sanitized: String = segment
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            value if value.is_control() => '_',
            value => value,
        })
        .collect();

    let sanitized = sanitized.trim().trim_matches('.').trim();
    if sanitized.is_empty() {
        "_".to_string()
    } else {
        sanitized.to_string()
    }
}

/// Builds a download path based on metadata (Artist/Album structure).
/// - If both artist and album exist: root/[Artist]/[Album]
/// - If only artist exists: root/[Artist]
/// - Otherwise: root
fn build_metadata_based_path(root: &Path, artist: Option<&str>, album: Option<&str>) -> PathBuf {
    let mut path = root.to_path_buf();
    
    if let Some(artist_name) = artist {
        let sanitized_artist = sanitize_path_segment(artist_name);
        path.push(&sanitized_artist);
        
        if let Some(album_name) = album {
            let sanitized_album = sanitize_path_segment(album_name);
            path.push(&sanitized_album);
        }
    }
    
    path
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

/// Extracts artist and album metadata from an audio file.
fn extract_audio_metadata(file_path: &Path) -> (Option<String>, Option<String>) {
    match Probe::open(file_path) {
        Ok(probe) => {
            match probe.read() {
                Ok(tagged) => {
                    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
                        let artist = tag.artist().as_deref().map(|s| s.to_owned());
                        let album = tag.album().as_deref().map(|s| s.to_owned());
                        return (artist, album);
                    }
                }
                Err(_) => {}
            }
        }
        Err(_) => {}
    }
    (None, None)
}

/// Reorganizes a downloaded file to its final location based on metadata.
/// Moves file from its temporary download location to a metadata-based path.
/// 
/// Path structure:
/// - If both artist and album exist: data_dir/[Artist]/[Album]/filename
/// - If only artist exists: data_dir/[Artist]/filename
/// - Otherwise: data_dir/filename
fn reorganize_downloaded_file(
    file_path: &Path,
    data_dir: &Path,
) -> Result<PathBuf, String> {
    if !file_path.exists() {
        return Err(format!("Downloaded file not found: {}", file_path.display()));
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

fn preview_cache_root(library: &LibraryState) -> PathBuf {
    library
        .data_dir()
        .parent()
        .unwrap_or_else(|| library.data_dir())
        .join(".soulseek-preview-cache")
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

    tauri::async_runtime::spawn(monitor_aux_download(download, handle));
    Ok(())
}

fn encode_cover_as_data_url(path: &Path, cover_filename: &str) -> Result<Option<String>, String> {
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };

    if data.is_empty() {
        return Ok(None);
    }

    Ok(Some(format!(
        "data:{};base64,{}",
        cover_mime_from_filename(cover_filename),
        B64.encode(data)
    )))
}

fn cover_mime_from_filename(cover_filename: &str) -> &'static str {
    match Path::new(cover_filename)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "image/jpeg",
    }
}

async fn monitor_aux_download(download: Download, mut handle: DownloadHandle) {
    while let Some(status) = handle.recv().await {
        match status {
            DownloadStatus::QueuedLocally => {
                eprintln!(
                    "[soulseek] sidecar cover queued locally: user={} cover={}",
                    download.username,
                    download.filename
                );
            }
            DownloadStatus::QueuedRemotely { place } => {
                eprintln!(
                    "[soulseek] sidecar cover queued remotely: user={} cover={} place={:?}",
                    download.username,
                    download.filename,
                    place
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
                    download.username,
                    download.filename,
                    download.download_directory
                );
                break;
            }
            DownloadStatus::TimedOut => {
                eprintln!(
                    "[soulseek] sidecar cover timed out: user={} cover={} destination_dir={}",
                    download.username,
                    download.filename,
                    download.download_directory
                );
                break;
            }
            DownloadStatus::Cancelled => {
                eprintln!(
                    "[soulseek] sidecar cover cancelled: user={} cover={} destination_dir={}",
                    download.username,
                    download.filename,
                    download.download_directory
                );
                break;
            }
        }
    }

    eprintln!(
        "[soulseek] sidecar cover monitor ended: user={} cover={} destination_dir={}",
        download.username,
        download.filename,
        download.download_directory
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
                    transfer_id,
                    download.username,
                    download.filename
                );
                event.state = "queued_local".to_string();
                if is_preview {
                    set_preview_transfer_state(Path::new(&local_path), PreviewTransferState::Active);
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
                    set_preview_transfer_state(Path::new(&local_path), PreviewTransferState::Active);
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
                    set_preview_transfer_state(Path::new(&local_path), PreviewTransferState::Active);
                }
                false
            }
            DownloadStatus::Completed => {
                eprintln!(
                    "[soulseek] download completed: transfer_id={} user={} remote={} local_path={}",
                    transfer_id,
                    download.username,
                    download.filename,
                    local_path
                );
                event.state = "completed".to_string();
                event.bytes_downloaded = Some(download.size);

                // Reorganize file based on metadata (not for previews)
                if !is_preview {
                    let file_path = Path::new(&local_path);
                    match reorganize_downloaded_file(file_path, &data_dir) {
                        Ok(final_path) => {
                            let final_path_str = final_path.to_string_lossy().to_string();
                            event.local_path = Some(final_path_str);
                            eprintln!(
                                "[soulseek] file reorganized successfully: {} -> {}",
                                local_path, final_path.display()
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
                    set_preview_transfer_state(Path::new(&local_path), PreviewTransferState::Completed);
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
                    set_preview_transfer_state(Path::new(&local_path), PreviewTransferState::Failed);
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
                    set_preview_transfer_state(Path::new(&local_path), PreviewTransferState::TimedOut);
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
                    set_preview_transfer_state(Path::new(&local_path), PreviewTransferState::Cancelled);
                }
                true
            }
        };

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

    eprintln!(
        "[soulseek] download monitor ended: transfer_id={} user={} remote={} destination_dir={}",
        transfer_id,
        download.username,
        download.filename,
        download.download_directory
    );
}