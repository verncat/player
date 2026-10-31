# Native storage and access scope

Implementation: `store.rs`, `secrets.rs`, `native.rs`, native capability gateway.

Registry data and settings are stored locally under the plugin-system app data directory.
Registry writes use atomic replacement with backup recovery; the total registry limit
is 512 MiB. Per-ID settings use optimistic revision checks and a 64 KiB JSON limit.
Native artifacts are materialized under an ID/fingerprint directory only after approved
enable. Decoded binaries are SHA-256 checked; the ABI and exported ID are verified.

Credential providers remain macOS Keychain, Windows Credential Manager, Linux Secret
Service, and Android Keystore/encrypted preferences. Failed secret access is not
repeated automatically; presence checks do not load the secret. Disabling clears the
activation's credential cache. Secrets do not belong in settings, logs, or UI snapshots.

Host capabilities and callback replies validate ID, enabled state and generation.
Navigation/playback actions validate registered contributions and granted capabilities.
These checks are not a sandbox: all native libraries/APKs execute in Player's process
with its OS access. Preview communicates native-code execution before installation.
Publisher signatures are not inferred from the package fingerprint.

Legacy script packages are removed on registry migration, while per-ID settings and
secret metadata remain. Their records are disabled so the same ID can be reinstalled
as a native replacement. Builtin IDs retain their settings and preferences.

macOS library signature validation remains enabled; signed/hardened builds can reject
libraries that do not meet their signing requirements. Android pins APK file and signer
SHA-256, verifies before loading classes, and requires restart for a changed loaded JNI
library. Split APK plugins remain unsupported.
