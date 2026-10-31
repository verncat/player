# Native Discord package scope

Package: `plugins/discord-rpc/src/lib.rs`; host adapter: `src-tauri/src/plugins/discord.rs`.

The package implements NativePlugin, consumes playback snapshots and returns its settings
UI directly from Rust. It calls discord.update with the current playing track and clears
activity on pause. The host adapter owns Discord IPC, reconnects, timestamp handling,
and throttling. Errors are displayed through the package's native-rendered UI.

Manifest API 2 registers the settings contribution and requests ui, settings,
playback.read and native.discord. Platforms: macOS, Windows, Linux. The package is
linked into Player with FFI exports disabled; the existing discord-rpc Cargo feature
controls the host integration. No script, Worker or frontend presence adapter is shipped.

Checks cover native pause/error behavior, coalescing and reconnects in the host adapter.
Live validation requires Discord desktop running. Disable clears native presence.
