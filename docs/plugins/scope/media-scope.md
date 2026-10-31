# Native media scope

Implementation: `plugin_sources.rs`, playback coordinator, native gateway, import paths.

A plugin-owned resource has pluginId/resourceId, title/artist/album, optional duration
and cover hash. It is independent of a library track ID. The host issues an opaque
source token after authorizing the current activation. Plugins do not choose arbitrary
local filesystem paths outside their own canonical staging directory.
With storage, storage.staging returns that directory. With media.source,
media.register issues a source token, media.state updates transfer state, and
media.cover stores an image and returns its hash. Symlink escapes are rejected.

`host.play` uses the main playback coordinator, queue/output selection and media state.
`host.transport` validates playback control; `host.adopt` refreshes loaded playback and
queue state after importing a plugin resource. Native `playback` receives snapshots
only with playback.read; transport changes require playback.control.

Soulseek can buffer a growing native download before playback, then import or replace
the completed file through library.import/library.replace. Existing common import,
rollback, indexing, tag and cover rules apply. A panel closing does not own or cancel
the native transfer. Source grants are activation-scoped and revoked on disable;
owned playback is stopped. Generic media interfaces remain independent of Soulseek UI.
