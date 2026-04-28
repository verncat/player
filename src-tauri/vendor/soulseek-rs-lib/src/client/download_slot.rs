/// RAII guard representing one active download slot.
///
/// Holding a `DownloadSlot` means one concurrency slot is in use.
/// Store it in `ConnectedWorker::active_slots` keyed by `DownloadToken`.
/// The slot is freed when this value is dropped (i.e. removed from the map).
/// The active count is just `active_slots.len()` — no separate counter needed.
pub struct DownloadSlot;
