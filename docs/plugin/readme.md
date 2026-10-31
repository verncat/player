# Player native plugin system

Player supports native plugins only. Plugin API **2** replaces the former script
contract; desktop libraries use C ABI **1**, and the catalog uses schema **2**.
There is no plugin Worker, executable script, or JavaScript SDK.

Native plugin code handles lifecycle, rendering, actions, search, playback snapshots,
and native events. The Vue frontend renders returned UI data using Player's shared
components. Host settings, secrets, playback coordination, and capability services
remain in Player.

## Packages and author documentation

Sources, Rust SDK, tooling, examples, and the public catalog live in
[verncat/player-plugins](https://github.com/verncat/player-plugins), pinned locally
as `plugins/` for the generic SDK only. Discord and Soulseek ship independently;
a fresh Player installation has no preinstalled providers. Initialize it before building:

```sh
git submodule update --init -- plugins
```

- [Repository layout](https://github.com/verncat/player-plugins/blob/main/docs/structure.md).
- [Native package development](https://github.com/verncat/player-plugins/blob/main/docs/development.md).
- [Rust SDK and C ABI](https://github.com/verncat/player-plugins/blob/main/docs/sdk.md).
- [Android APK example](https://github.com/verncat/player-plugins/blob/main/examples/android-native/README.md).

## Host implementation

| Area | Source |
| --- | --- |
| ABI loading, native instances, host callbacks | [native.rs](../../src-tauri/src/plugins/native.rs) |
| Native package/manifest validation | [model.rs](../../src-tauri/src/plugins/model.rs) |
| Registry, settings, grants, migration | [store.rs](../../src-tauri/src/plugins/store.rs) |
| Public package downloads | [remote.rs](../../src-tauri/src/plugins/remote.rs) |
| Remote catalog | [catalog.rs](../../src-tauri/src/plugins/catalog.rs) |
| Native capabilities and lifecycle integration | [mod.rs](../../src-tauri/src/plugins/mod.rs) |
| Android discovery and pinned loader | [android.rs](../../src-tauri/src/plugins/android.rs), [ApkPlugins.kt](../../src-tauri/gen/android/app/src/main/java/dev/verncat/player/ApkPlugins.kt) |
| Credential providers | [secrets.rs](../../src-tauri/src/plugins/secrets.rs) |
| Frontend data bridge | [host.ts](../../src/plugins/host.ts) |
| Installed plugins / Add plugin page | [PluginManager.vue](../../src/plugins/components/PluginManager.vue), [PluginInstaller.vue](../../src/plugins/components/PluginInstaller.vue) |
| UI renderer | [components](../../src/plugins/components) |

## Scope documents

[System](../plugins/scope/system-scope.md), [package](../plugins/scope/package-scope.md),
[runtime](../plugins/scope/runtime-scope.md), [SDK/UI](../plugins/scope/sdk-ui-scope.md),
[media](../plugins/scope/media-scope.md), [storage/security](../plugins/scope/storage-security-scope.md),
[catalog](../plugins/scope/catalog-scope.md), [Discord](../plugins/scope/discord-scope.md),
[Soulseek](../plugins/scope/soulseek-scope.md), [Android](../plugins/scope/android-native-scope.md),
[delivery](../plugins/scope/delivery-scope.md).

Native code executes with Player's system access; host permission checks are not an
OS sandbox. macOS library signature validation remains enabled. Legacy script packages
are rejected; migration preserves settings/credentials while removing their executable
packages and disabling their records so native replacements can be installed.
