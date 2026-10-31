# Native delivery and validation scope

Sources: `verncat/player-plugins`; Player pins its tested commit as `plugins/`.
Release workflows initialize that submodule before Cargo resolves Rust path dependencies.
Catalog publication remains independent of the bundled code pin.

From Player:

```sh
pnpm test:plugins
pnpm test:plugin-sdk
pnpm exec vue-tsc --noEmit
pnpm exec vite build
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo check --manifest-path src-tauri/Cargo.toml --no-default-features --lib
```

The frontend checks native IPC, activation-bound callbacks, registered navigation,
events, settings drafts and pending buttons. Rust host checks package/hash validation,
registry migration, credentials, catalog/download security, and real compiled native
library loading through C ABI 1. Native package tests cover their UI/action/search logic.

From player-plugins:

```sh
cargo test --workspace --locked
cargo build --workspace --locked
./gradlew -p examples/android-native assembleDebug
```

The native repo CI tests/builds on macOS, Linux and Windows. Android JNI/Kotlin builds
and device instrumentation are separate. Live Discord/Soulseek connections, signing
and hardened release behavior need their platform verification. macOS library validation
is retained; this change does not relax that protection.

API 2 is intentionally incompatible with script packages. Package preview does not run
native code; enable requires a matching target and approved package fingerprint.
