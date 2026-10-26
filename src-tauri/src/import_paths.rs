//! Shared metadata and library placement rules.
use lofty::prelude::{Accessor, TaggedFileExt};
use lofty::probe::Probe;
use std::path::{Path, PathBuf};

pub(crate) fn sanitize_path_segment(segment: &str) -> String {
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
pub(crate) fn build_metadata_based_path(
    root: &Path,
    artist: Option<&str>,
    album: Option<&str>,
) -> PathBuf {
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

/// Extracts artist and album metadata from an audio file.
pub(crate) fn extract_audio_metadata(file_path: &Path) -> (Option<String>, Option<String>) {
    match Probe::open(file_path) {
        Ok(probe) => match probe.read() {
            Ok(tagged) => {
                if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
                    let artist = tag.artist().as_deref().map(|s| s.to_owned());
                    let album = tag.album().as_deref().map(|s| s.to_owned());
                    return (artist, album);
                }
            }
            Err(_) => {}
        },
        Err(_) => {}
    }
    (None, None)
}
