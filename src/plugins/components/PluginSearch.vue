<script setup lang="ts">
import { onUnmounted, ref, watch } from 'vue';
import { usePluginHost, validateUiNode } from '../host';
import type { SearchItem, UiContribution } from '../types';
import PluginTree from './PluginTree.vue';
import '../controls.css';
const props = defineProps<{ contribution: UiContribution; query: string }>();
const host = usePluginHost(); const items = ref<SearchItem[]>([]); const error = ref(''); const loading = ref(false); const submitted = ref('');
let sequence = 0; let disposed = false;
const currentCursor = ref<string | null>(null); const nextCursor = ref<string | null>(null); const previousCursors = ref<Array<string | null>>([]);
async function search(cursor: string | null = null, direction: 'new' | 'next' | 'previous' = 'new') {
  const query = props.query.trim(); if (!query || loading.value) return;
  const seq = ++sequence; loading.value = true; error.value = ''; submitted.value = query;
  try {
    const result = await host.call(props.contribution.pluginId, 'search', { providerId: props.contribution.id, query, cursor, limit: 120 }) as { items: SearchItem[]; nextCursor: string | null };
    if (seq === sequence && !disposed) {
      items.value = validateUiNode({ type: 'results', items: result.items }).items || [];
      if (direction === 'new') previousCursors.value = [];
      else if (direction === 'next') previousCursors.value.push(currentCursor.value);
      else previousCursors.value.pop();
      currentCursor.value = cursor; nextCursor.value = typeof result.nextCursor === 'string' ? result.nextCursor : null;
    }
  } catch (e) { if (seq === sequence) error.value = String(e); }
  finally { if (seq === sequence) loading.value = false; }
}
async function action(action: string, payload = {}) {
  try { error.value = ''; await host.call(props.contribution.pluginId, 'action', { action, payload }); }
  catch (e) { error.value = String(e); }
}
watch(() => props.query, () => { sequence++; items.value = []; error.value = ''; submitted.value = ''; loading.value = false; currentCursor.value = null; nextCursor.value = null; previousCursors.value = []; });
watch(host.revision, async () => {
  if (!items.value.length) return;
  const seq = sequence;
  try { const updated = await host.call(props.contribution.pluginId, 'renderSearch', { ids: items.value.map(i => i.id) }) as SearchItem[] | null; if (updated && seq === sequence && !disposed) items.value = validateUiNode({ type: 'results', items: updated }).items || []; } catch { /* Original search results remain usable. */ }
});
onUnmounted(() => { disposed = true; sequence++; });
</script>
<template><section class="plugin-search"><div class="section-head"><h2>{{ contribution.label }}</h2><button class="plugin-button btn-secondary" :disabled="loading || !query.trim()" @click="search()">{{ loading ? 'Searching…' : 'Search' }}</button></div><p v-if="error" role="alert">{{ error }}</p><p v-else-if="!submitted">Press Search to query {{ contribution.label }}.</p><p v-else-if="!loading && !items.length">No matches for “{{ submitted }}”.</p><PluginTree :node="{ type: 'results', items }" :fields="{}" :busy="false" @action="action" /><div v-if="previousCursors.length || nextCursor" class="plugin-search-pages"><button class="plugin-button btn-secondary" :disabled="loading || !previousCursors.length" @click="search(previousCursors[previousCursors.length - 1] ?? null, 'previous')">Previous</button><button class="plugin-button btn-secondary" :disabled="loading || !nextCursor" @click="search(nextCursor, 'next')">Next</button></div></section></template>
<style scoped>.plugin-search { margin-top: 24px; }.plugin-search .section-head { display: flex; justify-content: space-between; gap: 16px; }.plugin-search-pages { display: flex; justify-content: flex-end; gap: 12px; margin-top: 12px; }.plugin-search > p { color: var(--color-text-muted); }</style>
