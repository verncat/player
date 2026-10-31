<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { usePluginHost } from '../host';
import { catalogManifest, filterCatalog } from '../catalog';
import type { CatalogEntry, PluginCatalogListing } from '../types';
import PluginImage from './PluginImage.vue';
import PluginMetadata from './PluginMetadata.vue';

defineProps<{ busy: boolean; pendingId?: string }>();
const emit = defineEmits<{ review: [entry: CatalogEntry]; uninstall: [id: string]; notify: [message: string] }>();
const host = usePluginHost();
const listing = ref<PluginCatalogListing>();
const query = ref('');
const loading = ref(false);
let disposed = false;
const entries = computed(() => filterCatalog(listing.value?.catalog.plugins || [], query.value));
const installed = (id: string) => host.plugins.value.find(p => p.manifest.id === id);
const supported = (entry: CatalogEntry) => !!listing.value && entry.platforms.includes(listing.value.platform);
const tags = (entry: CatalogEntry) => [...new Set([...entry.platforms, ...entry.tags])];

async function load() {
  if (loading.value) return;
  loading.value = true;
  listing.value = undefined;
  try {
    const result = await invoke<PluginCatalogListing>('plugins_catalog');
    if (!disposed) listing.value = result;
  } catch (e) { if (!disposed) emit('notify', String(e)); }
  finally { if (!disposed) loading.value = false; }
}
onMounted(() => { void load(); });
onUnmounted(() => { disposed = true; });
</script>

<template>
  <section class="plugin-catalog" aria-label="Plugin catalog">
    <div class="catalog-heading">
      <div><h4>Plugin catalog</h4><p>Discover extensions shared by plugin authors.</p></div>
      <button type="button" class="control-button" :disabled="loading || busy" :aria-busy="loading" @click="load">{{ loading ? 'Refreshing…' : 'Refresh catalog' }}</button>
    </div>
    <label class="catalog-filter"><input v-model="query" class="form-input" type="search" aria-label="Search catalog" placeholder="Name, author or tag" autocomplete="off" /></label>
    <p v-if="!listing && loading" role="status">Loading catalog…</p>
    <p v-else-if="!loading && !entries.length" role="status">{{ listing?.catalog.plugins.length ? 'No plugins match your search.' : 'No plugins available.' }}</p>
    <article v-for="entry in entries" :key="entry.id" class="plugin-card catalog-card">
      <PluginImage class="catalog-image" :manifest="catalogManifest(entry)" />
      <div class="catalog-copy">
        <h4>{{ entry.name }} <small v-if="entry.version">{{ entry.version }}</small></h4>
        <p class="catalog-description">{{ entry.description }}</p>
        <PluginMetadata :manifest="catalogManifest(entry)">
          <span class="catalog-dates">Added <time :datetime="entry.addedAt">{{ entry.addedAt }}</time><span v-if="entry.updatedAt && entry.updatedAt !== entry.addedAt"> · Updated <time :datetime="entry.updatedAt">{{ entry.updatedAt }}</time></span></span>
        </PluginMetadata>
        <p v-if="!supported(entry)" class="catalog-status">Unavailable on this platform.</p>
      </div>
      <div class="catalog-footer">
        <ul class="catalog-tags" aria-label="Tags"><li v-for="tag in tags(entry)" :key="tag">{{ tag }}</li></ul>
        <button type="button" class="control-button control-button-compact" :disabled="busy || (!installed(entry.id) && !supported(entry))" :aria-busy="pendingId === entry.id" @click="installed(entry.id) ? emit('uninstall', entry.id) : emit('review', entry)">{{ installed(entry.id) ? 'Uninstall' : listing?.platform === 'android' && entry.androidApkUrl ? 'Download APK' : 'Install' }}</button>
      </div>
    </article>
  </section>
</template>

<style scoped>
.plugin-catalog { display: grid; gap: 16px; margin-top: 16px; }
.plugin-catalog h4,.plugin-catalog p { margin: 0; }
.plugin-catalog h4 { color: var(--color-text); font-size: 16px; }
.plugin-catalog p { line-height: 1.6; color: var(--color-text-muted); }
.catalog-heading { display: flex; align-items: center; gap: 16px; justify-content: space-between; }
.catalog-copy { min-width: 0; display: grid; gap: 5px; overflow-wrap: anywhere; }
.catalog-heading h4 { margin-bottom: 8px; }
.catalog-copy h4 { font-size: 15px; line-height: 1.4; }
.catalog-copy small { color: var(--color-text-muted); font-size: 12px; font-weight: 400; margin-inline-start: 4px; }
.catalog-card { display: grid; grid-template-columns: 44px minmax(0, 1fr); gap: 12px; padding: 16px; }
.catalog-card .catalog-image { width: 44px; height: 44px; flex-basis: 44px; }
.catalog-copy .catalog-description { font-size: 13px; line-height: 1.5; }
.catalog-copy .catalog-status { font-size: 12px; line-height: 1.5; }
.catalog-footer { grid-column: 1 / -1; display: flex; flex-wrap: wrap; align-items: center; gap: 10px 16px; }
.catalog-footer > button { flex-shrink: 0; margin-inline-start: auto; }
.catalog-dates { font-size: 12px; overflow-wrap: anywhere; }
.catalog-filter { display: grid; gap: 8px; }
.catalog-tags { display: flex; flex: 1 1 240px; flex-wrap: wrap; gap: 5px; list-style: none; padding: 0; margin: 0; min-width: 0; }
.catalog-tags li { border: 1px solid var(--color-border); border-radius: 99px; padding: 2px 7px; font-size: 11px; line-height: 1.4; color: var(--color-text-muted); overflow-wrap: anywhere; max-width: 100%; }
@media(max-width: 600px) { .plugin-catalog > .catalog-heading { align-items: flex-start; flex-direction: column; } }
@media(max-width: 400px) { .catalog-card { padding: 14px; gap: 10px; }.catalog-tags { flex-basis: 100%; } }
</style>
