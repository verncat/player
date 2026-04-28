/// Identifies a specific download request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DownloadToken(pub u32);

/// Identifies a file search query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SearchToken(pub u32);

/// Identifies a peer connection attempt (pierce firewall / peer init).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PierceToken(pub u32);

macro_rules! impl_token {
    ($t:ty) => {
        impl From<u32> for $t {
            fn from(v: u32) -> Self {
                Self(v)
            }
        }
        impl From<$t> for u32 {
            fn from(t: $t) -> u32 {
                t.0
            }
        }
        impl std::fmt::Display for $t {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

impl_token!(DownloadToken);
impl_token!(SearchToken);
impl_token!(PierceToken);

/// Token assigned by the remote peer in a `TransferRequest` message and sent
/// over the wire for F-type / pierce-firewall connections.
/// Distinct from [`DownloadToken`], which is our internal stable key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PeerTransferToken(pub u32);

impl_token!(PeerTransferToken);
