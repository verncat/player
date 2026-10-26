<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { usePluginHost } from '../host';
import { catalogManifest, filterCatalog } from '../catalog';
import type { CatalogEntry, PluginCatalogListing } from '../types';
import PluginImage from './PluginImage.vue';
import PluginMetadata from './PluginMetadata.vue';
import '../controls.css';

defineProps<{ busy: boolean }>();
const emit = defineEmits<{ review: [entry: CatalogEntry] }>();
const host = usePluginHost();
const listing = ref<PluginCatalogListing>();
const query = ref('');
const loading = ref(false);
const error = ref('');
let disposed = false;
const entries = computed(() => filterCatalog(listing.value?.catalog.plugins || [], query.value));
const installed = (id: string) => host.plugins.value.find(p => p.manifest.id === id);
const supported = (entry: CatalogEntry) => !!listing.value && entry.platforms.includes(listing.value.platform);

async function load(refresh: boolean) {
  loading.value = true; error.value = '';
  try {
    const result = await invoke<PluginCatalogListing>('plugins_catalog', { refresh });
    if (!disposed) listing.value = result;
  } catch (e) { if (!disposed) error.value = String(e); }
  finally { if (!disposed) loading.value = false; }
}
onMounted(async () => { await load(false); if (!disposed) await load(true); });
onUnmounted(() => { disposed = true; });
</script>

<template>
  <section class="plugin-catalog" aria-label="Plugin catalog">
    <div class="catalog-heading">
      <div><h4>Plugin catalog</h4><p>Discover extensions shared by plugin authors.</p></div>
      <button type="button" class="plugin-button btn-secondary" :disabled="loading || busy" @click="load(true)">{{ loading ? 'Refreshing…' : 'Refresh catalog' }}</button>
    </div>
    <p v-if="listing" class="catalog-status">{{ listing.source === 'remote' ? 'Latest catalog' : 'Offline catalog' }} · Updated <time :datetime="listing.catalog.updatedAt">{{ listing.catalog.updatedAt }}</time></p>
    <p v-if="error" class="catalog-error" role="alert">{{ error }}</p>
    <p v-else-if="listing?.warning" class="catalog-status" role="status">{{ listing.warning }}</p>
    <label class="catalog-filter"><span>Search catalog</span><input v-model="query" type="search" placeholder="Name, author or tag" autocomplete="off" /></label>
    <p v-if="!listing && loading" role="status">Loading catalog…</p>
    <p v-else-if="listing && !entries.length">{{ listing.catalog.plugins.length ? 'No plugins match your search.' : 'No plugins have been added to the catalog yet.' }}</p>
    <article v-for="entry in entries" :key="entry.id" class="plugin-card catalog-card">
      <div class="catalog-heading">
        <PluginImage :manifest="catalogManifest(entry)" />
        <div class="catalog-copy"><h4>{{ entry.name }} <small v-if="entry.version">{{ entry.version }}</small></h4><p>{{ entry.description }}</p></div>
      </div>
      <PluginMetadata :manifest="catalogManifest(entry)" />
      <p class="catalog-dates">Added <time :datetime="entry.addedAt">{{ entry.addedAt }}</time><span v-if="entry.updatedAt && entry.updatedAt !== entry.addedAt"> · Updated <time :datetime="entry.updatedAt">{{ entry.updatedAt }}</time></span></p>
      <p class="catalog-platforms">{{ entry.platforms.join(' · ') }}</p>
      <ul v-if="entry.tags.length" class="catalog-tags" aria-label="Tags"><li v-for="tag in entry.tags" :key="tag">{{ tag }}</li></ul>
      <p v-if="installed(entry.id)">Installed{{ installed(entry.id)?.enabled && !installed(entry.id)?.builtin ? ' — disable it before updating.' : '' }}</p>
      <p v-if="!supported(entry)">Unavailable on this platform.</p>
      <button type="button" class="plugin-button btn-secondary" :disabled="busy || !supported(entry) || installed(entry.id)?.enabled || installed(entry.id)?.builtin" @click="emit('review', entry)">Review plugin</button>
    </article>
  </section>
</template>

<style scoped>
.plugin-catalog { display: grid; gap: 16px; margin-top: 16px; }
.plugin-catalog h4,.plugin-catalog p { margin: 0; }
.plugin-catalog h4 { color: var(--color-text); font-size: 16px; }
.plugin-catalog p { line-height: 1.6; color: var(--color-text-muted); }
.catalog-heading { display: flex; align-items: center; gap: 16px; justify-content: space-between; }
.catalog-copy { flex: 1; min-width: 0; overflow-wrap: anywhere; }
.catalog-heading h4 { margin-bottom: 8px; }
.catalog-heading small { color: var(--color-text-muted); font-size: 12px; font-weight: 400; }
.catalog-card { display: grid; gap: 20px; }
.catalog-card > button { justify-self: start; }
.catalog-status,.catalog-dates,.catalog-platforms { font-size: 12px; overflow-wrap: anywhere; }
.catalog-filter { display: grid; gap: 8px; }
.catalog-filter input { width: 100%; min-width: 0; padding: 10px; color: var(--color-text); background: var(--color-bg-elevated); border: 1px solid var(--color-border); border-radius: 6px; }
.catalog-filter input:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 2px; }
.catalog-tags { display: flex; flex-wrap: wrap; gap: 8px; list-style: none; padding: 0; margin: 0; }
.catalog-tags li { border: 1px solid var(--color-border); border-radius: 99px; padding: 3px 9px; font-size: 12px; color: var(--color-text-muted); }
.catalog-error { color: var(--color-danger-text) !important; overflow-wrap: anywhere; }
@media(max-width: 600px) { .plugin-catalog > .catalog-heading { align-items: flex-start; flex-direction: column; } }
</style>
