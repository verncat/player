// Core modules
pub mod actor;
pub mod client;
pub mod dispatcher;
pub mod error;
pub mod message;
pub mod path;
pub mod peer;
pub mod search_rate_limiter;
pub mod token;
pub mod types;
#[macro_use]
pub mod utils;

// Prelude module for commonly used items
pub mod prelude {
    pub use crate::actor::server_actor::PeerAddress;
    pub use crate::path::SoulseekPath;
    pub use crate::types::{DownloadStatus, File, FileAttributes, Search, SearchResult, Transfer};
    pub use crate::{debug, error, info, trace, warn};
}

// Re-export commonly used types
pub use actor::server_actor::PeerAddress;
pub use client::{Client, ClientSettings, DownloadHandle};
pub use error::{Result, SoulseekRs};
pub use path::SoulseekPath;
pub use token::{DownloadToken, PeerTransferToken, PierceToken, SearchToken};
pub use types::{DownloadStatus, File, FileAttributes, Search, SearchResult, Transfer};
