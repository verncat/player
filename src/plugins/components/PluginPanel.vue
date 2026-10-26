<script setup lang="ts">
import { onUnmounted, ref, watch } from 'vue';
import { usePluginHost, validateUiNode } from '../host';
import type { UiNode } from '../types';
import PluginTree from './PluginTree.vue';
const props = defineProps<{ pluginId: string; viewId: string; context?: Record<string, unknown> }>();
const host = usePluginHost();
const node = ref<UiNode>(); const fields = ref<Record<string, string | boolean>>({});
const error = ref(''); const busy = ref(false); let sequence = 0; let disposed = false;
function defaults(n: UiNode) { if (n.type === 'input' && n.key && fields.value[n.key] === undefined) fields.value[n.key] = n.value ?? ''; n.children?.forEach(defaults); }
async function render() {
  const seq = ++sequence;
  try {
    const result = await host.call(props.pluginId, 'render', { viewId: props.viewId, context: props.context || {} });
    if (disposed || seq !== sequence) return;
    node.value = validateUiNode(result); defaults(node.value); error.value = '';
  } catch (e) { if (!disposed && seq === sequence) error.value = String(e); }
}
async function action(action: string, payload = {}) {
  if (busy.value) return; busy.value = true; error.value = '';
  try {
    await host.call(props.pluginId, 'action', { action, payload, fields: { ...fields.value }, context: props.context || {} });
    fields.value = {}; await render();
  } catch (e) { error.value = String(e); }
  finally { busy.value = false; }
}
watch(() => [props.pluginId, props.viewId, props.context], () => { fields.value = {}; node.value = undefined; error.value = ''; void render(); }, { immediate: true });
watch(host.revision, () => { if (!busy.value) void render(); });
onUnmounted(() => { disposed = true; sequence++; });
</script>
<template><section class="plugin-panel" :aria-busy="busy"><p v-if="error" role="alert" class="plugin-panel-error">{{ error }}</p><PluginTree v-if="node" :node="node" :fields="fields" :busy="busy" @action="action" /><p v-else-if="!error" role="status">Loading…</p></section></template>
<style scoped>.plugin-panel { min-width: 0; padding: 12px 0; }.plugin-panel-error { color: var(--color-danger-text); overflow-wrap: anywhere; }</style>
