# Native package and installation scope

Entry point: [readme](../../plugin/readme.md). Implementation: `model.rs`, `remote.rs`, `store.rs`.

An external `.player-plugin` is UTF-8 JSON with `manifest` and `native`.
`native.kind: "library"` contains `artifacts`, keyed by OS/architecture. Each artifact
has base64 `data` and a lowercase SHA-256 `sha256`. Supported keys are
`macos-aarch64`, `macos-x86_64`, `windows-aarch64`, `windows-x86_64`,
`linux-aarch64`, and `linux-x86_64`. Its OS must appear in manifest platforms.

Maximums: 64 MiB decoded per library, 96 MiB per package, eight artifacts. Preview
checks hashes, metadata, reserved IDs/permissions, platforms, and API version without
opening a library. A fingerprint covers the entire package. Installation verifies the
approved fingerprint and persists a disabled package. Actual loading occurs on enable.

Manifest API version is 2. Existing author/repository/homepage/image fields and
contributions remain supported. `player.*`, `native.discord`, and `native.soulseek`
are reserved for bundled native packages. Builtins have `native.kind: "builtin"`
and cannot be installed through the external package path.

Installed Android APKs use `native.kind: "android-apk"`; Player derives the manifest's
`androidNative` package/class/file/certificate binding from the installed APK. They are
not unpinned desktop packages with an Android permission added manually.

The downloader accepts direct HTTPS native packages and public GitHub directories
containing `plugin.player-plugin`, resolving branches to a specific commit. Raw source
manifests are rejected. Redirects remain HTTPS without credentials. Native binaries
are not fetched implicitly from arbitrary manifest paths or executed during preview.

Updating requires disabling the old version. The ID remains stable and settings survive.
Libraries remain resident until process exit even after instance destruction.
On hardened macOS, libraries must satisfy the existing signature validation rules;
this implementation does not disable them.
