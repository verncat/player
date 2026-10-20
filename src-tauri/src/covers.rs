//! Cover image store. Each distinct cover is kept once, as small resized
//! copies named after the blake3 hash of the original image, and served to the
//! webview over the `cover://` URI scheme. Originals aren't stored: the
//! full-size image is read from the audio file again when needed
//! (`library::full_cover`).

use std::collections::HashSet;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageReader};
use tauri::http::{header, Request, Response, StatusCode};
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};

use crate::library::LibraryState;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Must match COVER_SCHEME in src/utils/covers.ts.
pub const SCHEME: &str = "cover";

/// Files younger than this are never pruned: a cover that is being stored
/// right now isn't referenced by its track yet.
const PRUNE_MIN_AGE: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Copy)]
enum CoverSize {
    Small,
    Medium,
}

impl CoverSize {
    const ALL: [CoverSize; 2] = [CoverSize::Small, CoverSize::Medium];

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "small" => Some(Self::Small),
            "medium" => Some(Self::Medium),
            _ => None,
        }
    }

    fn max_px(self) -> u32 {
        match self {
            Self::Small => 128,
            Self::Medium => 512,
        }
    }

    fn jpeg_quality(self) -> u8 {
        match self {
            Self::Small => 80,
            Self::Medium => 85,
        }
    }
}

pub(crate) fn store_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(".covers")
}

pub(crate) fn hash(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

fn is_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn resized_path(data_dir: &Path, hash: &str, size: CoverSize) -> PathBuf {
    store_dir(data_dir).join(format!("{hash}-{}.jpg", size.max_px()))
}

fn svg_path(data_dir: &Path, hash: &str) -> PathBuf {
    store_dir(data_dir).join(format!("{hash}.svg"))
}

fn is_svg(data: &[u8]) -> bool {
    let start = &data[..data.len().min(256)];
    let text = String::from_utf8_lossy(start);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    text.starts_with("<svg") || (text.starts_with("<?xml") && text.contains("<svg"))
}

fn mime_type(data: &[u8]) -> &'static str {
    if is_svg(data) {
        return "image/svg+xml";
    }
    image::guess_format(data)
        .map(|format| format.to_mime_type())
        .unwrap_or("application/octet-stream")
}

pub(crate) fn is_stored(data_dir: &Path, hash: &str) -> bool {
    svg_path(data_dir, hash).exists()
        || CoverSize::ALL
            .iter()
            .all(|size| resized_path(data_dir, hash, *size).exists())
}

pub(crate) fn store(data_dir: &Path, data: &[u8]) -> Option<String> {
    if data.is_empty() {
        return None;
    }
    let hash = hash(data);
    if is_stored(data_dir, &hash) {
        return Some(hash);
    }
    match write_sizes(data_dir, &hash, data) {
        Ok(()) => Some(hash),
        Err(error) => {
            tracing::warn!(
                target: "player_lib::covers",
                %hash,
                bytes = data.len(),
                %error,
                "couldn't store cover"
            );
            None
        }
    }
}

#[cfg_attr(
    feature = "tracy",
    tracing::instrument(
        name = "covers.write_sizes",
        target = "player_lib::profile",
        skip(data_dir, data),
        fields(bytes = data.len())
    )
)]
fn write_sizes(data_dir: &Path, hash: &str, data: &[u8]) -> Result<(), BoxError> {
    std::fs::create_dir_all(store_dir(data_dir))?;
    if is_svg(data) {
        return write_atomically(&svg_path(data_dir, hash), data);
    }

    let image = ImageReader::new(Cursor::new(data))
        .with_guessed_format()?
        .decode()?;
    for size in CoverSize::ALL {
        let bytes = encode_resized(&image, size)?;
        write_atomically(&resized_path(data_dir, hash, size), &bytes)?;
    }
    Ok(())
}

fn encode_resized(image: &DynamicImage, size: CoverSize) -> Result<Vec<u8>, BoxError> {
    let max = size.max_px();
    let rgb = if image.width() > max || image.height() > max {
        image.thumbnail(max, max).to_rgb8()
    } else {
        image.to_rgb8()
    };
    let mut bytes = Vec::new();
    rgb.write_with_encoder(JpegEncoder::new_with_quality(
        &mut bytes,
        size.jpeg_quality(),
    ))?;
    Ok(bytes)
}

/// Writes through a temporary file, so a reader never sees half a file and
/// `is_stored` never counts one left behind by a crash.
fn write_atomically(path: &Path, data: &[u8]) -> Result<(), BoxError> {
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
    let temp = path.with_extension(format!(
        "{}.tmp",
        NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&temp, data)?;
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(())
}

fn read_size(data_dir: &Path, hash: &str, size: CoverSize) -> Option<(Vec<u8>, &'static str)> {
    if let Ok(data) = std::fs::read(resized_path(data_dir, hash, size)) {
        return Some((data, "image/jpeg"));
    }
    std::fs::read(svg_path(data_dir, hash))
        .ok()
        .map(|data| (data, "image/svg+xml"))
}

/// The largest stored copy, for when the original can't be read any more.
pub(crate) fn read_largest(data_dir: &Path, hash: &str) -> Option<Vec<u8>> {
    read_size(data_dir, hash, CoverSize::Medium).map(|(data, _)| data)
}

/// Deletes stored covers whose hash isn't in `referenced`. This includes
/// files left over from the old one-file-per-track layout.
pub(crate) fn prune_unused(data_dir: &Path, referenced: &HashSet<String>) -> usize {
    let Ok(entries) = std::fs::read_dir(store_dir(data_dir)) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let hash = name.split(['-', '.']).next().unwrap_or(name);
        if referenced.contains(hash) {
            continue;
        }
        let old_enough = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= PRUNE_MIN_AGE);
        if old_enough && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// What a `cover://localhost/<hash>-<variant>` URL asks for.
enum Variant {
    Stored(CoverSize),
    Full,
}

fn parse_key(path: &str) -> Option<(&str, Variant)> {
    let (hash, name) = path.trim_start_matches('/').split_once('-')?;
    let variant = match name {
        "full" => Variant::Full,
        name => Variant::Stored(CoverSize::from_name(name)?),
    };
    is_hash(hash).then_some((hash, variant))
}

/// Serves `cover://localhost/<hash>-<variant>`, off the main thread.
pub fn handle_request<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = ctx.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let response = match app.try_state::<LibraryState>() {
            Some(library) => respond(library.data_dir(), request.uri().path(), |hash| {
                library.full_cover(hash)
            }),
            None => empty_response(StatusCode::SERVICE_UNAVAILABLE),
        };
        responder.respond(response);
    });
}

fn respond(
    data_dir: &Path,
    path: &str,
    full_cover: impl FnOnce(&str) -> Option<Vec<u8>>,
) -> Response<Vec<u8>> {
    let Some((hash, variant)) = parse_key(path) else {
        return empty_response(StatusCode::BAD_REQUEST);
    };
    let found = match variant {
        Variant::Stored(size) => read_size(data_dir, hash, size),
        Variant::Full => full_cover(hash).map(|data| {
            let mime = mime_type(&data);
            (data, mime)
        }),
    };
    let Some((data, mime)) = found else {
        return empty_response(StatusCode::NOT_FOUND);
    };
    Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        // WebKitGTK only loads images from custom schemes in CORS mode.
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(data)
        .unwrap_or_else(|_| empty_response(StatusCode::INTERNAL_SERVER_ERROR))
}

fn empty_response(status: StatusCode) -> Response<Vec<u8>> {
    let mut response = Response::new(Vec::new());
    *response.status_mut() = status;
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_data_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("player-covers-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([200, 40, 90]));
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn stores_each_cover_once_at_both_sizes() {
        let dir = temp_data_dir("sizes");
        let data = png(1000, 1000);
        let hash = store(&dir, &data).unwrap();
        assert_eq!(store(&dir, &data).as_deref(), Some(hash.as_str()));

        let small =
            image::load_from_memory(&read_size(&dir, &hash, CoverSize::Small).unwrap().0).unwrap();
        let medium = image::load_from_memory(&read_largest(&dir, &hash).unwrap()).unwrap();
        assert_eq!((small.width(), small.height()), (128, 128));
        assert_eq!((medium.width(), medium.height()), (512, 512));
        assert_eq!(std::fs::read_dir(store_dir(&dir)).unwrap().count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeps_small_covers_at_their_own_size() {
        let dir = temp_data_dir("small");
        let hash = store(&dir, &png(300, 200)).unwrap();
        let medium = image::load_from_memory(&read_largest(&dir, &hash).unwrap()).unwrap();
        assert_eq!((medium.width(), medium.height()), (300, 200));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stores_svg_as_is_and_rejects_garbage() {
        let dir = temp_data_dir("svg");
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
        let hash = store(&dir, svg).unwrap();
        assert_eq!(
            read_size(&dir, &hash, CoverSize::Small).unwrap(),
            (svg.to_vec(), "image/svg+xml")
        );
        assert_eq!(store(&dir, b"not an image"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serves_stored_sizes_by_url_path() {
        let dir = temp_data_dir("respond");
        let hash = store(&dir, &png(600, 600)).unwrap();
        let no_full = |_: &str| None;
        let small = respond(&dir, &format!("/{hash}-small"), no_full);
        assert_eq!(small.status(), StatusCode::OK);
        assert_eq!(small.headers()[header::CONTENT_TYPE], "image/jpeg");
        assert_eq!(image::load_from_memory(small.body()).unwrap().width(), 128);

        let missing = "0".repeat(64);
        let status = |path: String| respond(&dir, &path, no_full).status();
        assert_eq!(status(format!("/{missing}-medium")), StatusCode::NOT_FOUND);
        assert_eq!(status(format!("/{hash}-huge")), StatusCode::BAD_REQUEST);
        assert_eq!(
            status("/..%2F..%2Fapp.db-small".into()),
            StatusCode::BAD_REQUEST
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serves_full_size_from_the_reader() {
        let dir = temp_data_dir("full");
        let original = png(900, 900);
        let hash = hash(&original);
        let full = respond(&dir, &format!("/{hash}-full"), |requested| {
            (requested == hash).then(|| original.clone())
        });
        assert_eq!(full.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(full.body(), &original);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_keeps_referenced_and_recent_files() {
        let dir = temp_data_dir("prune");
        let kept = store(&dir, &png(10, 10)).unwrap();
        let unused = store(&dir, &png(20, 20)).unwrap();
        let referenced = HashSet::from([kept.clone()]);
        // Everything was just written, so nothing is old enough yet.
        assert_eq!(prune_unused(&dir, &referenced), 0);
        assert!(is_stored(&dir, &unused));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
