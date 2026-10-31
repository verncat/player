# Native plugin system scope

Entry point: [readme](../../plugin/readme.md).

The system accepts native code only. All plugin lifecycle, UI generation, search,
actions, playback handling, and event handling are native. Vue renders UI data;
it does not evaluate plugin code. Desktop packages use compiled shared libraries;
Android uses separately installed, pinned APKs. Bundled Discord/Soulseek packages
implement the same Rust trait and link into Player with FFI exports disabled.

The standalone `player-plugins` repository owns packages, SDK, packaging tools,
and examples. Player owns installation, native loading, registry/settings/secrets,
the UI schema, playback coordination, and native capability services. Source is
pinned by the `plugins/` gitlink. Runtime catalog requests are independent of that pin.

Contracts: manifest API 2, C ABI 1, catalog schema 2. No API 1 script compatibility
runtime is retained. Existing per-ID data and bundled activation preferences survive
migration; old external executable packages are removed and require native replacements.

Native instances are serialized per activation and invoked on blocking worker threads.
Host capabilities validate the requesting ID/generation; disabling revokes capabilities
before closing the instance and media sources. Native memory safety, OS access,
and process crashes remain the responsibility of native code, not a script sandbox.
