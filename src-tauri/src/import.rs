//! Drag-and-drop import of audio files into the library.
//!
//! Dropped files/folders are copied (never moved) into `data_dir` following
//! the same path convention as Soulseek downloads:
//! `Artist/Album/filename`, `Artist/filename`, or `filename` when tags are
//! missing. Copied files are indexed immediately.

use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use tauri::Emitter;
use walkdir::WalkDir;

use crate::library::{index_file, is_audio_extension, is_cue_extension, LibraryState};
use crate::soulseek::{build_metadata_based_path, extract_audio_metadata};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub imported: usize,
    pub skipped: usize,
}

/// Matches the `index-progress` payload emitted by library indexing so the
/// existing frontend index log surfaces drag-and-drop imports too.
#[derive(Clone, Serialize)]
struct ImportProgress {
    current: usize,
    total: usize,
    status: &'static str,
    added: usize,
    track_name: Option<String>,
}

fn emit_progress(app: &tauri::AppHandle, progress: ImportProgress) {
    let _ = app.emit("index-progress", progress);
}

#[tauri::command]
pub fn import_dropped_files(
    app: tauri::AppHandle,
    state: tauri::State<'_, LibraryState>,
    paths: Vec<String>,
) -> Result<ImportResult, String> {
    let data_dir = state.data_dir().to_path_buf();
    let canonical_data_dir = fs::canonicalize(&data_dir).unwrap_or_else(|_| data_dir.clone());

    let mut sources: Vec<PathBuf> = Vec::new();
    for raw in &paths {
        let path = PathBuf::from(raw);
        if path.is_dir() {
            for entry in WalkDir::new(&path).into_iter().filter_map(|e| e.ok()) {
                if entry.file_type().is_file() {
                    sources.push(entry.path().to_path_buf());
                }
            }
        } else if path.is_file() {
            sources.push(path);
        }
    }

    let mut imported = 0usize;
    let mut skipped = 0usize;
    let total = sources.len();

    if total > 0 {
        emit_progress(
            &app,
            ImportProgress {
                current: 0,
                total,
                status: "indexing",
                added: 0,
                track_name: None,
            },
        );
    }

    for source in sources {
        let ext = source
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_lowercase())
            .unwrap_or_default();
        if !is_audio_extension(&ext) && !is_cue_extension(&ext) {
            skipped += 1;
            continue;
        }

        // Skip files that already live inside the library.
        if let Ok(canonical) = fs::canonicalize(&source) {
            if canonical.starts_with(&canonical_data_dir) {
                skipped += 1;
                continue;
            }
        }

        let Some(filename) = source.file_name() else {
            skipped += 1;
            continue;
        };

        let (artist, album) = if is_audio_extension(&ext) {
            extract_audio_metadata(&source)
        } else {
            (None, None)
        };
        let target_dir = build_metadata_based_path(&data_dir, artist.as_deref(), album.as_deref());
        if let Err(e) = fs::create_dir_all(&target_dir) {
            eprintln!(
                "[import] failed to create directory {}: {}",
                target_dir.display(),
                e
            );
            skipped += 1;
            continue;
        }
        let target = target_dir.join(filename);

        if target.exists() {
            skipped += 1;
            continue;
        }

        if let Err(e) = fs::copy(&source, &target) {
            eprintln!(
                "[import] failed to copy {} -> {}: {}",
                source.display(),
                target.display(),
                e
            );
            skipped += 1;
            continue;
        }

        {
            let conn = state.conn();
            let conn = conn.lock().unwrap();
            if let Err(e) = index_file(&conn, &data_dir, &target) {
                eprintln!("[import] failed to index {}: {}", target.display(), e);
            }
        }
        imported += 1;
        emit_progress(
            &app,
            ImportProgress {
                current: imported,
                total,
                status: "added",
                added: imported,
                track_name: target
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned()),
            },
        );
    }

    if total > 0 {
        emit_progress(
            &app,
            ImportProgress {
                current: total,
                total,
                status: "done",
                added: imported,
                track_name: None,
            },
        );
    }

    if imported > 0 {
        let _ = app.emit("library-changed", ());
    }

    Ok(ImportResult { imported, skipped })
}
