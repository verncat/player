# Native catalog scope

Implementation: `catalog.rs`, `catalog.ts`, `PluginCatalog.vue`.
Source: `https://raw.githubusercontent.com/verncat/player-plugins/main/catalog.json`.

Catalog schema 2 lists native packages only. The previous script example entry is
removed until a compiled native artifact is published. Empty catalogs are valid.
There is no bundled catalog, disk cache, backup, or offline fallback.

Required top-level fields: `schemaVersion: 2`, calendar `updatedAt`, `plugins`.
Entries require `id`, `name`, `description`, `repository`, `addedAt`, `platforms`;
optional fields are installUrl, version, author, image, updatedAt, androidApkUrl and tags.
At most 256 unique entries, 12 tags of up to 40 bytes, and 512 KiB total. Unknown
fields, invalid dates/URLs or a bad entry reject the whole response.

Settings → Plugins has Installed and Add plugins subtabs, rendered with the reusable
SettingsTabs component. The Add plugins tab (`settings/plugins/add`) contains the
catalog/search, URL/file installation and Android APK discovery. Successful
installation switches back to Installed.
Player fetches when Add plugins opens or Refresh catalog is selected. Old entries
are cleared before fetching; failures appear as toasts. Search matches names, IDs,
authors, descriptions, platforms and tags. New entries sort first, then by name.

Install downloads a real native `.player-plugin` and checks its ID before preview.
Metadata grants no permissions and does not authenticate the publisher. GitHub folder
URLs must contain `plugin.player-plugin`; source-only folders are not installable.
Official IDs are ordinary installable packages. Android entries with androidApkUrl
show Download APK, then use system installation plus Find APK plugins.

`PLAYER_PLUGIN_CATALOG_URL` can override the endpoint at Rust build time. Catalog
updates do not automatically replace installed libraries.
