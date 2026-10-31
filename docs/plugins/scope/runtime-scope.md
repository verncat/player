# Native runtime scope

Implementation: `src-tauri/src/plugins/native.rs`, `src/plugins/host.ts`.

The backend owns native instances and library handles. Frontend `plugin_load` returns
metadata and event snapshots only. It then calls `plugin_native_call` for `activate`,
`render`, `action`, `search`, `renderSearch`, `playback`, and `event`.
Each request carries a plugin ID and activation generation; arguments/results are JSON
limited to 1 MiB. Calls to one activation are serialized outside the UI thread.

Native packages implement the Rust `NativePlugin` trait. Desktop libraries export
C ABI 1 create/call/free/destroy, ABI version, and plugin ID functions. The loader
validates ABI/ID before creating an instance. Each allocator releases its own buffers;
the host context is scoped to one invocation. Rust panic boundaries do not recover
segmentation faults or make arbitrary native code safe.

Native host calls use an activation-bound context. Settings/secrets/network/library
calls execute through the Rust capability gateway. Frontend-owned playback/navigation
uses `plugin-host-call` plus an owner/generation-checked `plugin_native_reply`.
`host.invalidate` refreshes the frontend. `host.background` completes through a
plugin-scoped event; it does not bypass capability checks.

Events emitted during activation are buffered, merged with replay snapshots, deduplicated
by revision, and delivered in order. Playback snapshots are coalesced while a native call
is in flight. Panel unmount or webview reload does not destroy native transfer state.

Disable persists revocation first, cancels pending frontend callbacks, deactivates and
destroys the instance, revokes source tokens, and stops owned playback. Libraries stay
resident until shutdown. Old generations cannot regain host capabilities. Native code
must stop its own workers; arbitrary in-process execution cannot be forcibly terminated
like a Worker. Retry creates a new enabled generation.
