# Native SDK and UI scope

SDK: `plugins/native-sdk/src/lib.rs`. Renderer/DTOs: `src/plugins/types.ts` and components.

Native plugins implement `NativePlugin::call(method, args, host)` and return JSON values.
The SDK provides `Host::call`, `Host::invalidate`, a test host, C buffer/envelope helpers,
and `export_plugin!`. It does not expose a JavaScript entrypoint or require Node.js.
The full C ABI and ownership rules are documented in the standalone repository's SDK guide.

Contributions remain `page`, `settings`, `search`, `track-action`, and `player-widget`.
They require `ui`, a unique local ID, kind and label. IDs are up to 64 characters in
`[a-z0-9-]`; at most 32 contributions. Optional fields are icon, description and order.
Only pages accept `showInMenu: false`; hidden pages remain internally navigable.
Player widgets declare `player.actions` or `player.status` slots.

`render` receives `viewId` and context and returns `group`, `text`, `input`, `button`,
`toggle`, or `results`. Limits: depth 12, 100 children per node, 1000 result rows.
No HTML, scripts or Vue components are passed. UI uses shared Player styles, accessible
controls, drafts, native action errors, and the same pending-button animation.

Actions receive `{action,payload,fields,context}`. Track actions receive `payload.track`;
search-row actions receive `payload.resourceId`. Search returns `{items,nextCursor}`;
`renderSearch` refreshes existing results without rerunning the network query.
Navigation is limited to registered pages owned by the calling plugin.
