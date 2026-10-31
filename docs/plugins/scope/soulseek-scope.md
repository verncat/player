# Native Soulseek package scope

Package: `plugins/soulseek/src/lib.rs`; host adapter: `src-tauri/src/plugins/soulseek.rs`.

The Rust package owns connection UI, search pagination, result rows, cover scheduling,
preview/import/replacement intentions, track-menu navigation and the player-status widget.
The host adapter owns connections/transfers and the common library/media gateways.
No plugin logic or UI state machine executes in JavaScript.

Contributions: connection settings, global search, hidden replacement page, Find another
version track action, and player.status widget. Permission/platform declarations remain
explicit. Passwords use secrets; settings store usernames and generation-scoped intentions.

Preview begins once a source is buffered; completed files import/replace through the
common gateway and adopt their saved library track in Player. Intentions survive UI
reconnects and native instance recovery within the same generation. Failed transfers
clear pending intentions; stale generations do not restore them. Results retain stable
pagination, and at most two background cover calls avoid blocking native render/search.

Checks cover paging without dropped results, internal page navigation, buffered playback,
import after recovery, duplicate completion, failed transfers, and stale intentions.
Live network/account behavior is separate from offline tests.
