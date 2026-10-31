<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import { usePluginHost } from '../host';
import type { AndroidPluginDiscovery, CatalogEntry, PluginDownload, PluginPreview } from '../types';
import PluginMetadata from './PluginMetadata.vue';
import PluginImage from './PluginImage.vue';
import PluginCatalog from './PluginCatalog.vue';
import AppToast from '../../components/AppToast.vue';

const emit = defineEmits<{ done: [] }>();
const host = usePluginHost();
const error = ref('');
const busy = ref('');
const catalogPendingId = computed(() => busy.value.match(/^(?:catalog|uninstall):(.+)$/)?.[1]);
const url = ref('');
const preview = ref<PluginPreview>();
const previewCard = ref<HTMLElement>();
const android = ref(false);
const apkPlugins = ref<AndroidPluginDiscovery['plugins']>([]);
const apkWarnings = ref<string[]>([]);
onMounted(() => { android.value = /Android/i.test(navigator.userAgent); });
let content = '';
function resetPreview() { preview.value = undefined; content = ''; }
function showPreview(candidate: string, checked: PluginPreview) {
  content = candidate; preview.value = checked;
  void nextTick(() => previewCard.value?.scrollIntoView({ block: 'nearest' }));
}
async function scanApks() {
  if (busy.value) return;
  busy.value = 'scan'; error.value = '';
  try {
    const found = await invoke<AndroidPluginDiscovery>('plugins_android_discover');
    apkPlugins.value = found.plugins; apkWarnings.value = found.warnings;
  } catch (e) { error.value = String(e); }
  finally { busy.value = ''; }
}
function previewApk(plugin: AndroidPluginDiscovery['plugins'][number]) {
  showPreview(plugin.content, plugin);
}
async function load(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  if (!file || busy.value) return;
  busy.value = 'preview'; error.value = ''; resetPreview();
  try {
    if (file.size > 96 * 1024 * 1024) throw new Error('Maximum native package size is 96 MiB');
    const candidate = await file.text();
    const checked = await invoke<PluginPreview>('plugins_preview', { content: candidate });
    showPreview(candidate, checked);
  } catch (e) { error.value = String(e); }
  finally { input.value = ''; busy.value = ''; }
}
async function loadUrl() {
  if (busy.value || !url.value.trim()) return;
  busy.value = 'download'; error.value = ''; resetPreview();
  try {
    const downloaded = await invoke<PluginDownload>('plugins_download', { url: url.value.trim() });
    showPreview(downloaded.content, downloaded);
  } catch (e) { error.value = String(e); }
  finally { busy.value = ''; }
}
async function loadCatalogPlugin(entry: CatalogEntry) {
  if (busy.value) return;
  busy.value = `catalog:${entry.id}`; error.value = ''; resetPreview();
  try {
    if (android.value && entry.androidApkUrl) {
      await openUrl(entry.androidApkUrl);
      error.value = 'Install the APK, then select Find APK plugins to connect it.';
      return;
    }

    const downloaded = await invoke<PluginDownload>('plugins_download', {
      url: entry.installUrl || entry.repository, expectedId: entry.id,
    });
    showPreview(downloaded.content, downloaded);
  } catch (e) { error.value = String(e); }
  finally { busy.value = ''; }
}
async function install() {
  if (!preview.value || busy.value) return;
  try {
    busy.value = 'install'; error.value = '';
    await host.install(content, preview.value.fingerprint, preview.value.source);
    resetPreview();
    emit('done');
  } catch (e) { error.value = String(e); }
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
</script>

<template>
  <section class="plugin-manager">
    <AppToast :message="error" @dismiss="error = ''" />
    <div v-if="preview" ref="previewCard" class="plugin-card">
      <div class="plugin-heading">
        <PluginImage :manifest="preview.manifest" />
        <div><h4>{{ preview.manifest.name }} <small>{{ preview.manifest.version }}</small></h4><p>{{ preview.manifest.description }}</p></div>
      </div>
      <PluginMetadata :manifest="preview.manifest" />
      <p class="plugin-native-note">Native code runs inside Player with the app’s system access.</p>
      <p>Permissions: {{ preview.manifest.permissions.join(', ') || 'none' }}</p>
      <template v-if="preview.manifest.androidNative">
        <small class="fingerprint">Signing certificate SHA-256: {{ preview.manifest.androidNative.certificateSha256 }}</small>
        <small class="fingerprint">APK SHA-256: {{ preview.manifest.androidNative.apkSha256 }}</small>
      </template>
      <small class="fingerprint">BLAKE3: {{ preview.fingerprint }}</small>
      <div class="plugin-actions plugin-install-actions">
        <button class="control-button control-button-primary" :disabled="Boolean(busy)" :aria-busy="busy === 'install'" @click="install">Install and grant these permissions</button>
        <button class="control-button" :disabled="Boolean(busy)" @click="resetPreview">Cancel</button>
      </div>
    </div>
    <PluginCatalog :busy="Boolean(busy)" :pending-id="catalogPendingId" @review="loadCatalogPlugin" @uninstall="uninstall" @notify="error = $event" />
    <div class="plugin-install">
      <h4>Manual installation</h4>
      <template v-if="android">
        <p>Install the extension APK on Android, then find it here.</p>
        <button class="control-button" :disabled="Boolean(busy)" :aria-busy="busy === 'scan'" @click="scanApks">{{ busy === 'scan' ? 'Scanning…' : 'Find APK plugins' }}</button>
        <p v-for="warning in apkWarnings" :key="warning" class="error" role="alert">{{ warning }}</p>
        <div v-for="plugin in apkPlugins" :key="plugin.manifest.id" class="plugin-card">
          <div class="plugin-heading">
            <PluginImage :manifest="plugin.manifest" />
            <div><h4>{{ plugin.manifest.name }} <small>{{ plugin.manifest.version }}</small></h4><p>{{ plugin.manifest.description }}</p></div>
          </div>
          <button class="control-button" :disabled="Boolean(busy)" @click="previewApk(plugin)">Review connection</button>
        </div>
      </template>
      <form class="plugin-url" @submit.prevent="loadUrl">
        <label for="plugin-url">Public GitHub repository, plugin folder or package URL</label>
        <div class="plugin-url-row"><input id="plugin-url" v-model="url" class="form-input" type="url" placeholder="https://github.com/author/plugin" autocomplete="off" :disabled="Boolean(busy)" /><button class="control-button" :disabled="Boolean(busy) || !url.trim()" :aria-busy="busy === 'download'">{{ busy === 'download' ? 'Loading…' : 'Load from URL' }}</button></div>
      </form>
      <label>Or choose a .player-plugin package<input class="form-file" type="file" accept=".player-plugin,.json" :disabled="Boolean(busy)" :aria-busy="busy === 'preview'" @change="load" /></label>
    </div>
  </section>
</template>

<style scoped src="./plugin-manager.css"></style>
