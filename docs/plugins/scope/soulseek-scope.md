# Independent Soulseek package scope

Sources: plugins/soulseek/src/lib.rs, src/service.rs and vendor/soulseek-rs-lib.
The package owns connection management, its Tokio runtime, protocol client, search,
transfers, cover fetching, previews and import/replacement intentions. Player contains
no Soulseek dependency or provider-specific gateway command.

Generic storage.staging and media.register/state/cover connect staged files to Player.
Workers queue events; manifest pollIntervalMs 250 drains them through scoped host calls.
Credentials use host secrets. Disable cancels tasks and closes the package runtime.
Contributions remain connection settings, global search, hidden replacement page,
track-menu action and player.status widget. Desktop libraries and the independent
soulseek/android APK use the same Rust implementation. Android uses system install
and generic APK discovery. Offline tests do not contact the live Soulseek network.
