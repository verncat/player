# Independent Discord package scope

Sources: plugins/discord-rpc/src/lib.rs and src/adapter.rs in player-plugins.
The package owns Discord IPC, worker lifecycle, reconnects, timestamps and throttling.
Player only forwards playback snapshots and renders generic returned settings UI.
Permissions: ui, settings, playback.read. Desktop only; ordinary install/uninstall.
No provider dependency, feature flag or adapter remains in Player’s build.
Tests use a fake transport; live Discord verification is separate.
