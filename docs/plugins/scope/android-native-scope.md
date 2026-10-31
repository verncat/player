# Android native plugin scope

Example: `plugins/examples/android-native`. Loader: `ApkPlugins.kt`, `android.rs`, `native.rs`.

An independently installed APK provides discovery metadata, API version 2, plugin ID,
a Kotlin entrypoint and compiled JNI library. Assets contain `{manifest,native}` only;
script fields are rejected. Player derives native.kind android-apk and the binding
from the installed package, its signing certificate and file hash.

Reflection contract: apiVersion() returns 2, pluginId() matches the manifest,
call(method,argsJson,hostCallback) returns JSON, and close() releases native resources.
The example allocates one native Rust instance per Kotlin instance. Rust handles
activate/render/action directly and calls host settings through the provided callback.

Each native invocation receives an opaque host token. Kotlin calls NativeHostBridge,
whose Player JNI export resolves only that invocation's active context and checks
permissions/generation before routing. Requests and responses are capped at 1 MiB.
Classloaders/JNI libraries remain retained until process exit. Native crashes are
process-wide; this loader is not an isolation boundary.

Discovery verifies metadata and signer/file identity before class initialization.
Changed files/signers require a new review; changed loaded JNI code requires restart.
Generation revocation rejects stale calls and closes instances. Removing a connection
does not uninstall the APK. Split APKs are unsupported; use a universal APK.

The standalone Gradle wrapper runs Rust tooling from the workspace, builds selected ABI
libraries through the installed NDK, and generates metadata without script assets.
Device tests validate native UI/actions, host callback persistence, pinning and lifecycle.
