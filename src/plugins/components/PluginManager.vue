<script setup lang="ts">
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { usePluginHost } from '../host';
import SettingsToggle from '../../components/SettingsToggle.vue';
import PluginMetadata from './PluginMetadata.vue';
import PluginImage from './PluginImage.vue';
import AppToast from '../../components/AppToast.vue';

const emit = defineEmits<{ add: [] }>();
const host = usePluginHost();
const error = ref('');
const busy = ref('');
async function toggle(id: string, enabled: boolean) {
  if (busy.value) return;
  try { busy.value = id; error.value = ''; await host.setEnabled(id, enabled); }
  catch (e) { error.value = String(e); }
  finally { busy.value = ''; }
}
async function clearCache(id: string) {
  if (busy.value) return;
  try { busy.value = `cache:${id}`; error.value = ''; await invoke('plugins_clear_cache', { pluginId: id }); }
  catch (e) { error.value = String(e); }
  finally { busy.value = ''; }
}
async function uninstall(id: string) {
  if (busy.value) return;
  try {
    busy.value = `uninstall:${id}`; error.value = '';
    if (host.plugins.value.find(plugin => plugin.manifest.id === id)?.enabled) await host.setEnabled(id, false);
    await host.uninstall(id);
  }
  catch (e) { error.value = String(e); }
  finally { busy.value = ''; }
}
async function retry(id: string) {
  if (busy.value) return;
  try { busy.value = `retry:${id}`; error.value = ''; await host.retry(id); }
  catch (e) { error.value = String(e); }
  finally { busy.value = ''; }
}
</script>

<template>
  <section class="plugin-manager">
    <div class="plugin-manager-header">
      <h3>Plugins</h3>
      <button type="button" class="control-button control-button-primary" :disabled="Boolean(busy)" @click="emit('add')">Add plugin</button>
    </div>
    <AppToast :message="error" @dismiss="error = ''" />
    <p v-if="host.errors.value.host" class="error" role="alert">{{ host.errors.value.host }}</p>
    <p v-if="host.loading.value" role="status">Loading plugins…</p>
    <p v-if="!host.loading.value && !host.plugins.value.length">No plugins installed.</p>
    <article v-for="plugin in host.plugins.value" :key="plugin.manifest.id" class="plugin-card">
      <div class="plugin-heading">
        <PluginImage :manifest="plugin.manifest" />
        <div><h4>{{ plugin.manifest.name }} <small>{{ plugin.manifest.version }}</small></h4><p>{{ plugin.manifest.description }}</p></div>
        <SettingsToggle :model-value="plugin.enabled" :label="plugin.manifest.name" :disabled="!plugin.supported || Boolean(busy)" :aria-busy="busy === plugin.manifest.id" @update:model-value="toggle(plugin.manifest.id, $event)" />
      </div>
      <PluginMetadata :manifest="plugin.manifest" />
      <p v-if="plugin.manifest.androidNative">Android APK · {{ plugin.manifest.androidNative.packageName }}</p>
      <p v-if="!plugin.supported">Unavailable on this platform.</p>
      <p v-else-if="plugin.enabled && !host.running.value.includes(plugin.manifest.id)">Not running</p>
      <p v-if="host.errors.value[plugin.manifest.id]" class="error" role="alert">{{ host.errors.value[plugin.manifest.id] }}</p>
      <div class="plugin-actions">
        <button v-if="(plugin.enabled || host.errors.value[plugin.manifest.id]) && plugin.supported && !host.running.value.includes(plugin.manifest.id)" class="control-button" :disabled="Boolean(busy)" :aria-busy="busy === `retry:${plugin.manifest.id}`" @click="retry(plugin.manifest.id)">Retry</button>
        <button v-if="!plugin.enabled" class="control-button" :disabled="Boolean(busy)" :aria-busy="busy === `cache:${plugin.manifest.id}`" @click="clearCache(plugin.manifest.id)">Clear cache</button>
        <button v-if="!plugin.enabled" class="control-button" :disabled="Boolean(busy)" :aria-busy="busy === `uninstall:${plugin.manifest.id}`" @click="uninstall(plugin.manifest.id)">Uninstall</button>
      </div>
      <small>{{ plugin.manifest.id }}</small>
    </article>
  </section>
</template>

<style scoped src="./plugin-manager.css"></style>
