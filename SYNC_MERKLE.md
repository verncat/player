# Sync Merkle Layout

This file describes a proposed Merkle-oriented shape for library sync data.

It is a design note only. Nothing here is implemented yet.

## Goals

- Make sync domains explicit and independently hashable.
- Allow partial sync by subtree instead of always merging one flat payload.
- Separate track state from track metadata fields so the protocol can sync likes/play counts independently from edited metadata.
- Avoid rebuilding expensive trees for append-only play history.

## Top-Level View

The current sync surface in [src-tauri/src/sync.rs](src-tauri/src/sync.rs) can be modeled as two large branches:

- `content_by_hash`: actual track file content and lightweight descriptors used to download blobs.
- `library_state`: metadata and user/library structure built on top of those blobs.

Conceptually:

```text
library_sync_root
├── content_by_hash
│   └── track::<file_hash>
│       ├── blob_hash
│       └── descriptor_hash
└── library_state
    ├── track_meta_by_hash
    ├── playlists_by_name
    ├── smart_playlists_by_id
    └── play_history_log
```

## Recommended Tree Shape

```text
library_sync_root
├── content_by_hash
│   └── track::<file_hash>
│       ├── blob
│       │   └── audio file bytes
│       └── descriptor
│           ├── path
│           ├── title?
│           ├── artist?
│           └── album?
│
└── library_state
    ├── track_meta_by_hash
    │   └── track::<file_hash>
    │       ├── state_hash
    │       │   └── state
    │       │       ├── is_liked
    │       │       ├── play_count
    │       │       ├── rarity?
    │       │       └── manually_edited
    │       └── fields_hash
    │           └── fields
    │               ├── title?
    │               ├── artist?
    │               ├── album?
    │               ├── track_number?
    │               ├── year?
    │               ├── genre?
    │               ├── tags?
    │               └── date_added?
    │
    ├── playlists_by_name
    │   └── playlist::<playlist_name>
    │       ├── identity_hash
    │       │   └── name
    │       └── tracks_hash
    │           └── items_in_order
    │               ├── 0000 -> <file_hash>
    │               ├── 0001 -> <file_hash>
    │               ├── 0002 -> <file_hash>
    │               └── ...
    │
    ├── smart_playlists_by_id
    │   └── smart_playlist::<playlist_id>
    │       ├── name
    │       ├── match_mode
    │       ├── rules_json
    │       └── updated_at
    │
    └── play_history_log
        ├── chunk::<time_bucket_or_sequence>
        │   ├── range
        │   │   ├── min_played_at
        │   │   └── max_played_at
        │   ├── rolling_hash
        │   └── events
        │       ├── event::<file_hash>::<played_at>
        │       ├── event::<file_hash>::<played_at>
        │       └── ...
        └── ...
```

## Why `track_meta_by_hash` Needs Two Levels

Instead of storing one combined hash per track metadata record, split it into:

```text
track::<file_hash>
├── state_hash
└── fields_hash
```

Where:

- `state_hash` covers mutable per-track state:
  - `is_liked`
  - `play_count`
  - `rarity`
  - `manually_edited`
- `fields_hash` covers editable metadata fields:
  - `title`
  - `artist`
  - `album`
  - `track_number`
  - `year`
  - `genre`
  - `tags`
  - `date_added`

This split makes the protocol more selective.

Examples:

- Sync only likes and counters: compare and request `state_hash` only.
- Sync only edited metadata: compare and request `fields_hash` only.
- Sync the full per-track logical record: compare both and only fetch the changed half.

This also matches the current merge policy in `sync.rs`, where state-like fields and manually edited fields are already treated differently.

## Why `play_history` Should Not Be a Full Tree

`play_history` is different from the rest of the sync data:

- it is append-only,
- it grows continuously,
- and new events are typically added at the tail.

Because of that, a classic per-event Merkle tree is a poor fit:

- too many leaves churn for a very hot structure,
- upper levels need frequent recomputation,
- and most syncs only care about the newest suffix anyway.

Recommended representation:

- store sorted events,
- partition them into chunks,
- track `min_played_at` and `max_played_at` per chunk,
- compute a rolling hash per chunk,
- sync by chunk boundary and time range.

Conceptually:

```text
play_history_log
├── chunk::000001
│   ├── min_played_at
│   ├── max_played_at
│   ├── event_count
│   ├── rolling_hash
│   └── events[]
├── chunk::000002
│   ├── min_played_at
│   ├── max_played_at
│   ├── event_count
│   ├── rolling_hash
│   └── events[]
└── ...
```

This gives cheap reconciliation:

- if a chunk hash matches, skip the whole chunk,
- if it differs, fetch only that chunk,
- if remote only has newer chunks, append them locally.

## Suggested Domain Rules

### 1. `content_by_hash`

Key:

- `file_hash`

Merge rule:

- content is immutable once addressed by hash,
- if blob exists locally, skip download,
- otherwise fetch `/file/<hash>`.

### 2. `track_meta_by_hash`

Key:

- `file_hash`

Split:

- `state_hash`
- `fields_hash`

Merge rule:

- `is_liked`: OR
- `play_count`: MAX
- `rarity`: adopt remote only when local is empty
- `date_added`: MIN
- edited fields: only adopt remote fields when local track is not already manually edited, or later define a stronger conflict rule.

### 3. `playlists_by_name`

Key:

- `playlist_name`

Children:

- identity
- ordered list of `file_hash` members

Merge rule:

- existing local playlist keeps its current order,
- unseen remote hashes are appended,
- absence in one peer does not imply deletion.

### 4. `smart_playlists_by_id`

Key:

- stable smart playlist id

Merge rule:

- compare `updated_at`,
- newer wins,
- if missing locally, insert.

### 5. `play_history_log`

Key:

- chunk id or time bucket

Leaf identity inside a chunk:

- `(file_hash, played_at)`

Merge rule:

- append-only union,
- deduplicate exact repeated events,
- prefer range or chunk comparison over a full Merkle tree.

## Sync Walk Through With This Model

One possible flow:

1. Compare `library_sync_root` summary.
2. If different, compare first-level branches:
   - `content_by_hash`
   - `library_state`
3. Under `library_state`, compare:
   - `track_meta_by_hash`
   - `playlists_by_name`
   - `smart_playlists_by_id`
   - `play_history_log`
4. For `track_meta_by_hash`, compare per-track:
   - `state_hash`
   - `fields_hash`
5. For `play_history_log`, compare chunk hashes and only fetch changed or new chunks.

This keeps the protocol selective instead of always transferring one monolithic `SyncData` payload.

## Practical Result

If this model is implemented later, the sync protocol can evolve from:

- one flat `SyncData` blob,

to:

- one root summary,
- subtree summaries,
- selective pulls per domain,
- selective pulls per `track::<hash>::state_hash` or `track::<hash>::fields_hash`,
- append-only history sync by chunk.

That would make metadata sync more incremental and would avoid wasting work on the hottest append-only structure in the system.