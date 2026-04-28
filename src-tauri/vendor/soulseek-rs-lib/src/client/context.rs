use crate::peer::PeerRegistry;

pub struct ClientContext {
    pub peer_registry: PeerRegistry,
}

impl ClientContext {
    pub fn new(peer_registry: PeerRegistry) -> Self {
        Self { peer_registry }
    }
}
