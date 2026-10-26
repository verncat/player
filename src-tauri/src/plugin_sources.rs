//! File-backed plugin resources. The decoder depends on this module, never a provider.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceIdentity {
    pub plugin_id: String,
    pub resource_id: String,
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    pub duration_seconds: Option<f64>,
    pub cover_hash: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferState {
    Active,
    Completed,
    Failed,
    TimedOut,
    Cancelled,
}

static TRANSFERS: LazyLock<Mutex<HashMap<PathBuf, TransferState>>> =
    LazyLock::new(Default::default);
pub fn transfer_state(path: &Path) -> Option<TransferState> {
    TRANSFERS.lock().unwrap().get(path).copied()
}
pub fn set_transfer_state(path: &Path, state: TransferState) {
    TRANSFERS.lock().unwrap().insert(path.to_path_buf(), state);
}
