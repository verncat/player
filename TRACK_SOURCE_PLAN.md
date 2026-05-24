# TrackSource plan

This note describes a small staged refactor for virtual playback sources.
The goal is to keep `Track` as the library/UI entity and move playback
specifics into a runtime `TrackSource`.

## Current shape

The app currently treats `tracks.path` as a library-relative audio file path.
Most user-facing features already hang off `tracks.id`: search, playlists,
history, likes, queueing, metadata edits, sync metadata, and UI rendering.

Playback is more file-oriented:

- frontend calls `playback_play(path)` for library tracks;
- frontend calls `playback_play_absolute(path, growing)` for Soulseek previews;
- Rust playback stores `current_file: Option<PathBuf>`;
- decode threads read a file path with Symphonia;
- `growing = true` makes EOF wait for Soulseek preview download progress.

This is close to a source abstraction already. The first refactor should keep
the existing decode/output/ring-buffer code and replace "play this path" with
"play this source".

## Target model

Keep `Track` as the stable library entity. Add a runtime playback source:

```rust
enum TrackSource {
    File {
        path: PathBuf,
    },
    CueSegment {
        audio_path: PathBuf,
        cue_path: PathBuf,
        start_sec: f64,
        end_sec: Option<f64>,
    },
    SoulseekPreview {
        username: String,
        filename: String,
        size: u64,
        cache_path: PathBuf,
    },
}
```

Later this can grow into a trait-based PCM source, but the first implementation
does not need to. A source resolver plus segment-aware decode is enough.

## Database shape

Use `tracks` for user-visible rows. This keeps playlists, history, likes,
search, and queueing intact.

Add source fields to `tracks`:

```sql
ALTER TABLE tracks ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'file';
ALTER TABLE tracks ADD COLUMN source_path TEXT;
ALTER TABLE tracks ADD COLUMN cue_path TEXT;
ALTER TABLE tracks ADD COLUMN segment_start_secs REAL;
ALTER TABLE tracks ADD COLUMN segment_end_secs REAL;
```

For normal files:

- `source_kind = 'file'`
- `path = source_path = library-relative file path` or `source_path = NULL`
- segment fields are `NULL`

For CUE tracks:

- `source_kind = 'cue'`
- `path = stable virtual id`, for example `Artist/Album/album.cue#03`
- `source_path = library-relative album image/audio file`
- `cue_path = library-relative cue file`
- `segment_start_secs = INDEX 01`
- `segment_end_secs = next INDEX 01`, or audio duration for the last track
- `duration_secs = segment_end_secs - segment_start_secs`

`path` remains unique. For virtual tracks it is an id, not a file path.

## Playback commands

Add a new command and keep old ones during migration:

```rust
playback_play_track(id: i64, position: Option<f64>, autoplay: Option<bool>)
```

The command loads the DB row, resolves a `TrackSource`, and starts playback.

Existing commands can remain:

- `playback_play(path)` for compatibility;
- `playback_play_absolute(path, growing)` for temporary previews until Soulseek
  is moved to `TrackSource`.

Frontend playback should eventually call `playback_play_track(track.id, ...)`
for real library tracks. Preview-only Soulseek rows can continue to use their
absolute preview path until stage 2.

## Decode behavior

Refactor the internal playback entry point from path-only to source-aware:

```rust
fn play_source(&self, source: TrackSource) -> Result<(), String>
```

The first implementation can still use the current file decoder:

- `File`: decode `path` from `0`;
- `CueSegment`: decode `audio_path`, seek to `start_sec + requested_position`,
  expose position relative to `start_sec`, stop naturally at `end_sec`;
- `SoulseekPreview`: decode `cache_path` with growing-file behavior.

Playback status should report source-relative position and duration:

- `File`: current behavior;
- `CueSegment`: `absolute_position - start_sec`, duration from segment bounds;
- `SoulseekPreview`: current growing duration behavior.

This keeps the output stream, ring buffer, volume, spectrum, pause/resume, and
most seek logic reusable.

## Stage 1: CUE support

Index `.cue` files in addition to audio files.

Parser responsibilities:

- read `FILE` entries;
- read `TRACK` number and type;
- read `TITLE`, `PERFORMER`, optionally `SONGWRITER`;
- read `INDEX 01`;
- convert `MM:SS:FF` to seconds, where `FF` is CD frames at 75 fps;
- resolve referenced audio files relative to the cue file directory;
- ignore unsupported or missing audio references gracefully.

Indexer behavior:

1. When scanning a `.cue`, parse it and create/update one `tracks` row per
   playable audio track.
2. Use a stable virtual `path`, such as `cue_rel#track_number`.
3. Set `source_kind = 'cue'`, `source_path`, `cue_path`, segment start/end,
   duration, title, artist, album, and track number.
4. Prefer per-track CUE metadata, then album-level CUE metadata, then path
   inference.
5. Use cover discovery from the referenced audio file directory.
6. Add generated virtual paths to the full-scan `visited` set so reindexing
   does not delete them.

Stale cleanup:

- If a cue file is deleted, delete rows where `cue_path = deleted_rel`.
- If a referenced audio file is deleted, keep the rows only if the cue still
  parses but mark them unplayable later, or delete them immediately. The simpler
  first behavior is delete.
- If cue content changes, replace rows for that `cue_path` with the newly parsed
  track set.

The album image/audio file itself can remain indexed as a normal file at first.
If it clutters the library, add a later rule to hide or mark whole-album image
tracks when they are referenced by a CUE.

## Stage 2: Soulseek as file-backed TrackSource

The acceptable first Soulseek source is file-backed, not a pure network PCM
stream.

Current behavior already downloads preview bytes into:

```text
.soulseek-preview-cache/<user>/<remote-parent>/<basename>
```

and playback reads that file with `growing = true`.

Turn that into:

```rust
TrackSource::SoulseekPreview {
    username,
    filename,
    size,
    cache_path,
}
```

Source behavior:

1. Resolve the preview cache path.
2. If no active transfer exists and the file is incomplete, start the current
   `client.download(...)` flow.
3. Register the download canceller and progress state exactly as today.
4. Start playback from `cache_path` with growing-file behavior.
5. On EOF:
   - if transfer is active, wait;
   - if transfer completed, finish playback;
   - if transfer failed/cancelled/timed out, stop playback cleanly.

Frontend behavior can get simpler after this:

- Soulseek preview tracks no longer need to carry `local_preview_path` as a
  playback escape hatch.
- The UI can create a transient preview `Track` or pass a preview source request.
- Playback still reports normal status, position, duration, and finished state.

Promotion to library remains file-based:

- if preview completed, move/reorganize the cache file into the library;
- index the final file;
- replace the active preview `Track` with the indexed library `Track` in UI
  state, as the app already does.

## Later: true streaming source

A pure streaming `SoulseekSource` is possible but should not be the first step.
The vendored Soulseek library currently writes chunks directly into a
`BufWriter<File>` and only exposes progress statuses. It does not expose a byte
stream to the player.

To support true streaming later, change the downloader sink:

```rust
enum DownloadSink {
    File(PathBuf),
    Channel(Sender<Bytes>),
    FileAndChannel { path: PathBuf, tx: Sender<Bytes> },
}
```

Then `SoulseekSource` could feed a custom Symphonia `MediaSource`. That is a
larger change and still would not guarantee arbitrary seek, because Soulseek
downloads are sequential in the current implementation.

## Recommended order

1. Add DB source columns and Rust `TrackSource`.
2. Add `playback_play_track(id, ...)` while keeping old commands.
3. Move normal file playback through `TrackSource::File`.
4. Add segment-aware decode and CUE indexing.
5. Move Soulseek preview playback to `TrackSource::SoulseekPreview`.
6. Remove or simplify frontend-only `local_preview_path` playback branching.
