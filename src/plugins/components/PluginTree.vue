<script setup lang="ts">
import type { UiNode } from '../types';
import CoverImage from '../../components/CoverImage.vue';
import { coverUrl } from '../../utils/covers';
defineProps<{ node: UiNode; fields: Record<string, string | boolean>; busy: boolean }>();
const emit = defineEmits<{ action: [action: string, payload?: Record<string, unknown>] }>();
</script>
<template>
  <div v-if="node.type === 'group'" class="plugin-group">
    <h3 v-if="node.title">{{ node.title }}</h3>
    <PluginTree v-for="(child, index) in node.children || []" :key="index" :node="child" :fields="fields" :busy="busy" @action="(action, payload) => emit('action', action, payload)" />
  </div>
  <p v-else-if="node.type === 'text' && node.text" :class="{ 'plugin-error': node.tone === 'error' }">{{ node.text }}</p>
  <label v-else-if="node.type === 'input' && node.key" class="plugin-field">
    <span>{{ node.label }}</span><input v-model="fields[node.key]" :type="node.inputType === 'password' ? 'password' : 'text'" :autocomplete="node.inputType === 'password' ? 'new-password' : 'off'" :disabled="busy" />
  </label>
  <button v-else-if="node.type === 'button' && node.action" class="btn-secondary" :disabled="busy || node.disabled" @click="emit('action', node.action!, node.payload)">{{ node.label }}</button>
  <label v-else-if="node.type === 'toggle' && node.action" class="plugin-field plugin-toggle"><span>{{ node.label }}</span><input type="checkbox" :checked="Boolean(node.value)" :disabled="busy || node.disabled" @change="emit('action', node.action!, { ...node.payload, value: ($event.target as HTMLInputElement).checked })" /></label>
  <div v-else-if="node.type === 'results'" class="plugin-results">
    <div v-for="item in node.items || []" :key="item.id" class="plugin-result">
      <CoverImage class="track-cover-sm" :src="coverUrl(item.coverHash, 'small')" />
      <div class="plugin-result-copy"><strong>{{ item.title }}</strong><span>{{ item.subtitle }}</span><span v-if="item.status">{{ item.status }}</span></div>
      <div class="plugin-result-actions"><button v-for="action in item.actions || []" :key="action.id" class="btn-secondary" :disabled="busy || action.disabled" @click="emit('action', action.id, { resourceId: item.id })">{{ action.label }}</button></div>
    </div>
  </div>
</template>
<style scoped>
.plugin-group { display: grid; gap: 16px; }.plugin-group h3 { margin: 0; }.plugin-group p { color: var(--color-text-muted); margin: 0; line-height: 1.5; }
.plugin-field { display: grid; gap: 8px; max-width: 560px; }.plugin-field input { padding: 10px; border: 1px solid var(--color-border); border-radius: 6px; background: var(--color-bg-elevated); color: var(--color-text); min-width: 0; }
.plugin-toggle { display: flex; align-items: center; justify-content: space-between; }.plugin-error { color: var(--color-danger-text) !important; }
.plugin-group > button { justify-self: start; }.plugin-results { display: grid; gap: 8px; }.plugin-result { display: flex; gap: 12px; align-items: center; padding: 10px 0; border-bottom: 1px solid var(--color-border); }
.plugin-result-copy { min-width: 0; flex: 1; display: grid; gap: 5px; }.plugin-result-copy strong,.plugin-result-copy span { overflow-wrap: anywhere; }.plugin-result-copy span { color: var(--color-text-muted); font-size: 12px; }.plugin-result-actions { display: flex; flex-wrap: wrap; gap: 8px; }
@media(max-width: 650px) { .plugin-result { flex-wrap: wrap; }.plugin-result-actions { width: 100%; justify-content: flex-end; } }
</style>
