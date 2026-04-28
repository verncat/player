use std::time::Duration;

use crate::actor::server_actor::PeerAddress;

const DEFALT_LISTEN_PORT: u32 = 2234;

/// TCP keepalive settings.
#[derive(Debug, Clone)]
pub enum KeepAliveSettings {
    Enabled {
        /// The idle time before sending a keepalive packet.
        idle: Duration,
        /// The interval between keepalive packets.
        interval: Duration,
        /// The number of keepalive packets to send before considering the connection dead.
        count: u32,
        /// The maximum idle time before considering the connection dead.
        max_idle: Duration,
    },
    Disabled,
}

#[derive(Debug, Clone)]
pub enum ReconnectSettings {
    Disabled,
    EnabledExponentialBackoff {
        min_delay: Duration,
        max_delay: Duration,
        /// The maximum number of attempts to reconnect.
        /// If None, the reconnect will continue indefinitely.
        max_attempts: Option<u32>,
    },
}

#[derive(Debug, Clone)]
pub struct SearchRateLimitSettings {
    pub searches: usize,
    pub per_period: Duration,
}

#[derive(Debug, Clone)]
pub struct DownloadRateLimitSettings {
    pub concurrent_downloads: u32,
}

/// A wrapper for a string that is plain text and is sent over the network via an unencrypted channel.
/// Soulseek uses an unencrypted channel for the username and password.
#[derive(Debug, Clone)]
pub struct PlainTextUnencrypted(pub String);

impl PlainTextUnencrypted {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for PlainTextUnencrypted {
    fn from(s: &str) -> Self {
        PlainTextUnencrypted(s.to_string())
    }
}

impl From<String> for PlainTextUnencrypted {
    fn from(s: String) -> Self {
        PlainTextUnencrypted(s)
    }
}

#[derive(Debug, Clone)]
pub struct ClientSettings {
    pub username: PlainTextUnencrypted,
    pub password: PlainTextUnencrypted,
    pub server_address: PeerAddress,
    pub enable_listen: bool,
    pub listen_port: u32,
    /// These are the settings for the TCP keepalive.
    /// This only affects the connections to the Soulseek server.
    pub tcp_keepalive_settings: KeepAliveSettings,
    /// These settings affect how to reconnect to the Soulseek server in case of a connection failure.
    pub reconnect_settings: ReconnectSettings,
    /// Controls the rate limiting of searches to prevent abuse, and being banned.
    pub search_rate_limit_settings: Option<SearchRateLimitSettings>,
    /// Controls the rate limiting of downloads to prevent abuse, and being banned.
    pub download_rate_limit_settings: Option<DownloadRateLimitSettings>,
    /// Advertised shared folder count — sent to the server after login and to peers on request.
    pub shared_folders: u32,
    /// Advertised shared file count — sent to the server after login and to peers on request.
    pub shared_files: u32,
}

impl ClientSettings {
    pub fn new(
        username: impl Into<PlainTextUnencrypted>,
        password: impl Into<PlainTextUnencrypted>,
    ) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
            ..Default::default()
        }
    }
}

impl Default for ClientSettings {
    fn default() -> Self {
        Self {
            username: PlainTextUnencrypted(String::new()),
            password: PlainTextUnencrypted(String::new()),
            server_address: PeerAddress::new("server.slsknet.org".to_string(), 2416),
            enable_listen: true,
            listen_port: DEFALT_LISTEN_PORT,
            tcp_keepalive_settings: KeepAliveSettings::Enabled {
                idle: Duration::from_secs(10),
                interval: Duration::from_secs(2),
                count: 10,
                max_idle: Duration::from_secs(30),
            },
            reconnect_settings: ReconnectSettings::EnabledExponentialBackoff {
                min_delay: Duration::from_secs(1),
                max_delay: Duration::from_secs(60),
                max_attempts: None,
            },
            search_rate_limit_settings: Some(SearchRateLimitSettings {
                searches: 34,
                per_period: Duration::from_secs(220),
            }),
            download_rate_limit_settings: Some(DownloadRateLimitSettings {
                concurrent_downloads: 2,
            }),
            shared_folders: 1,
            shared_files: 499,
        }
    }
}
