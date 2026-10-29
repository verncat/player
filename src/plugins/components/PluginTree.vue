<script setup lang="ts">
import type { UiNode } from '../types';
import CoverImage from '../../components/CoverImage.vue';
import { coverUrl } from '../../utils/covers';
import SettingsToggle from '../../components/SettingsToggle.vue';
defineProps<{ node: UiNode; fields: Record<string, string | boolean>; busy: boolean; pendingButton?: string; path?: string }>();
const emit = defineEmits<{ action: [action: string, payload?: Record<string, unknown>, button?: string] }>();
const resultButton = (path: string | undefined, resourceId: string, action: string) => JSON.stringify([path || '', resourceId, action]);
</script>
<template>
  <div v-if="node.type === 'group'" class="plugin-group">
    <h3 v-if="node.title">{{ node.title }}</h3>
    <PluginTree v-for="(child, index) in node.children || []" :key="index" :node="child" :fields="fields" :busy="busy" :pending-button="pendingButton" :path="`${path || ''}/${index}`" @action="(action, payload, button) => emit('action', action, payload, button)" />
  </div>
  <p v-else-if="node.type === 'text' && node.text" :class="{ 'plugin-error': node.tone === 'error' }">{{ node.text }}</p>
  <label v-else-if="node.type === 'input' && node.key" class="plugin-field form-field">
    <span>{{ node.label }}</span><input v-model="fields[node.key]" class="form-input" :type="node.inputType === 'password' ? 'password' : 'text'" :autocomplete="node.inputType === 'password' ? 'new-password' : 'off'" :disabled="busy" />
  </label>
  <button v-else-if="node.type === 'button' && node.action" class="control-button" :disabled="busy || node.disabled" :aria-busy="busy && pendingButton === `${path || ''}/button`" @click="emit('action', node.action!, node.payload, `${path || ''}/button`)">{{ node.label }}</button>
  <div v-else-if="node.type === 'toggle' && node.action" class="plugin-field plugin-toggle"><span>{{ node.label }}</span><SettingsToggle :model-value="Boolean(node.value)" :label="node.label || node.action" :disabled="busy || node.disabled" :aria-busy="busy && pendingButton === `${path || ''}/toggle`" @update:model-value="value => emit('action', node.action!, { ...node.payload, value }, `${path || ''}/toggle`)" /></div>
  <div v-else-if="node.type === 'results'" class="plugin-results">
    <div v-for="item in node.items || []" :key="item.id" class="plugin-result">
      <CoverImage class="track-cover-sm" :src="coverUrl(item.coverHash, 'small')" />
      <div class="plugin-result-copy"><strong>{{ item.title }}</strong><span>{{ item.subtitle }}</span><span v-if="item.status">{{ item.status }}</span></div>
      <div class="plugin-result-actions"><button v-for="action in item.actions || []" :key="action.id" class="control-button" :disabled="busy || action.disabled" :aria-busy="busy && pendingButton === resultButton(path, item.id, action.id)" @click="emit('action', action.id, { resourceId: item.id }, resultButton(path, item.id, action.id))">{{ action.label }}</button></div>
    </div>
  </div>
</template>
<style scoped>
.plugin-group { display: grid; gap: 16px; }.plugin-group h3 { margin: 0; }.plugin-group p { color: var(--color-text-muted); margin: 0; line-height: 1.5; }
.plugin-field { min-width: 0; }
.plugin-toggle { display: flex; align-items: center; justify-content: space-between; }.plugin-error { color: var(--color-danger-text) !important; }
.plugin-group > .control-button { justify-self: start; }.plugin-results { display: grid; gap: 8px; }.plugin-result { display: flex; gap: 12px; align-items: center; padding: 10px 0; border-bottom: 1px solid var(--color-border); }
.plugin-result-copy { min-width: 0; flex: 1; display: grid; gap: 5px; }.plugin-result-copy strong,.plugin-result-copy span { overflow-wrap: anywhere; }.plugin-result-copy span { color: var(--color-text-muted); font-size: 12px; }.plugin-result-actions { display: flex; flex-wrap: wrap; gap: 8px; }
@media(max-width: 650px) { .plugin-result { flex-wrap: wrap; }.plugin-result-actions { width: 100%; justify-content: flex-end; } }
</style>
