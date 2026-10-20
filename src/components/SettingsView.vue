<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { AudioDevice } from '../types';
import { SETTINGS_SECTIONS, type DeviceSettings, type SettingsSection } from '../types/settings';
import { useAppearance } from '../composables/useAppearance';
import { formatSampleRate } from '../utils/format';
import SettingsToggle from './SettingsToggle.vue';
import AppIcon from './AppIcon.vue';
import WebGLAlbumRenderer from './WebGLAlbumRenderer.vue';

const props = defineProps<{
  section: SettingsSection | null;
  showDuplicateTracks: boolean;
  dataDir: string | null;
  indexBusy: boolean;
  indexCurrent: number;
  indexTotal: number;
  indexDone: boolean;
  indexAdded: number;
  identifyBusy: boolean;
  actionError: string;
  actionMessage: string;
  outputDevices: AudioDevice[];
  currentDevice: string | null;
  selectedDevice: string | null;
  currentSampleRate: number | null;
  nativeSampleRate: number | null;
  audioLoading: boolean;
  audioBusy: boolean;
  audioError: string;
  remoteOutputLabel: string;
  syncEnabled: boolean;
  syncBusy: boolean;
  syncError: string;
  syncActive: boolean;
  peersCount: number;
  updateAvailable: boolean;
  coverUrl: string | null;
  coverColors: [string, string];
}>();
const emit = defineEmits<{
  navigate: [section: SettingsSection | null];
  'update:showDuplicateTracks': [value: boolean];
  openFolder: [];
  reindex: [];
  identify: [];
  duplicates: [];
  devices: [];
  refreshAudio: [];
  selectOutput: [name: string];
  selectSampleRate: [sampleRate: number | null];
  toggleSync: [];
  deviceSaved: [settings: DeviceSettings];
  soulseekSaved: [];
}>();

const selected = computed(() => props.section ?? 'library');
const sectionInfo = computed(() => SETTINGS_SECTIONS.find((section) => section.id === selected.value)!);
const panel = ref<HTMLElement | null>(null);
watch(() => props.section, async () => { await nextTick(); panel.value?.focus({ preventScroll: true }); });
const currentOutput = computed(() => props.outputDevices.find((device) => device.name === props.currentDevice));
const sampleRates = computed(() => {
  const rates = [...(currentOutput.value?.sample_rates ?? [])];
  if (props.currentSampleRate && !rates.includes(props.currentSampleRate)) rates.push(props.currentSampleRate);
  return rates.sort((a, b) => a - b);
});

const config = ref<DeviceSettings | null>(null);
const configLoading = ref(false);
const configError = ref('');
const deviceName = ref('');
const emoji = ref('🎵');
const username = ref('');
const password = ref('');
const showPassword = ref(false);
const identitySaving = ref(false);
const identityError = ref('');
const identitySaved = ref(false);
const soulseekSaving = ref(false);
const soulseekError = ref('');
const soulseekSaved = ref(false);
const emojiOptions = ['🎵', '🎶', '🎤', '🎧', '🎼', '🎹', '🎸', '🥁', '📱', '💻', '🖥️', '⌚', '📻', '📡', '🔊', '🎺', '🎻', '🪕', '🎷', '🍕'];
const identityDirty = computed(() => !!config.value && (deviceName.value !== config.value.device_name || emoji.value !== config.value.emoji));
const soulseekDirty = computed(() => !!config.value && (username.value !== config.value.soulseek_username || password.value !== config.value.soulseek_password));
const soulseekEnabled = computed(() => config.value?.soulseek_enabled ?? false);
const { preferences, reducedMotion, systemReducedMotion, storageError, setPreference } = useAppearance();

async function loadConfig() {
  configLoading.value = true;
  configError.value = '';
  try {
    config.value = await invoke<DeviceSettings>('get_device_settings');
    resetIdentity();
    resetSoulseek();
  } catch {
    configError.value = 'Could not load device settings. Please try again.';
  } finally {
    configLoading.value = false;
  }
}
function resetIdentity() {
  deviceName.value = config.value?.device_name ?? '';
  emoji.value = config.value?.emoji || '🎵';
  identityError.value = '';
  identitySaved.value = false;
}
function resetSoulseek() {
  username.value = config.value?.soulseek_username ?? '';
  password.value = config.value?.soulseek_password ?? '';
  showPassword.value = false;
  soulseekError.value = '';
  soulseekSaved.value = false;
}
async function persistConfig(next: DeviceSettings) {
  await invoke('set_device_settings', {
    emoji: next.emoji, deviceName: next.device_name,
    soulseekEnabled: next.soulseek_enabled,
    soulseekUsername: next.soulseek_username, soulseekPassword: next.soulseek_password,
  });
  config.value = next;
}
async function saveIdentity() {
  if (identitySaving.value || soulseekSaving.value) return;
  identitySaving.value = true;
  identityError.value = '';
  identitySaved.value = false;
  try {
    const latest = await invoke<DeviceSettings>('get_device_settings');
    const next = { ...latest, device_name: deviceName.value.trim(), emoji: emoji.value };
    await persistConfig(next);
    deviceName.value = next.device_name;
    identitySaved.value = true;
    emit('deviceSaved', next);
  } catch {
    identityError.value = 'Could not save device identity. Your changes are still here.';
  } finally { identitySaving.value = false; }
}
async function saveSoulseek() {
  if (soulseekSaving.value || identitySaving.value) return;
  soulseekSaving.value = true;
  soulseekError.value = '';
  soulseekSaved.value = false;
  try {
    const latest = await invoke<DeviceSettings>('get_device_settings');
    await persistConfig({ ...latest, soulseek_username: username.value.trim(), soulseek_password: password.value });
    username.value = config.value!.soulseek_username;
    showPassword.value = false;
    soulseekSaved.value = true;
    emit('soulseekSaved');
  } catch {
    soulseekError.value = 'Could not save Soulseek credentials. Please try again.';
  } finally { soulseekSaving.value = false; }
}
async function toggleSoulseek(enabled: boolean) {
  if (soulseekSaving.value || identitySaving.value) return;
  soulseekSaving.value = true;
  soulseekError.value = '';
  soulseekSaved.value = false;
  try {
    const latest = await invoke<DeviceSettings>('get_device_settings');
    await persistConfig({ ...latest, soulseek_enabled: enabled });
    emit('soulseekSaved');
  } catch { soulseekError.value = 'Could not change Soulseek search. Please try again.'; }
  finally { soulseekSaving.value = false; }
}
onMounted(() => { void loadConfig(); });
</script>

<template>
  <section class="settings-page" :class="{ 'has-section': section !== null }">
    <button v-if="section" class="mobile-back" @click="emit('navigate', null)"><AppIcon name="arrow-left" :size="18" /> Settings</button>
    <div class="settings-layout">
      <nav class="settings-nav" aria-label="Settings sections">
        <button v-for="item in SETTINGS_SECTIONS" :key="item.id" :class="{ selected: selected === item.id }" :aria-current="section === item.id ? 'page' : undefined" @click="emit('navigate', item.id)">
          <AppIcon :name="item.icon" />
          <span class="nav-copy"><span class="nav-label">{{ item.label }}</span><span class="nav-description">{{ item.description }}</span></span>
          <span v-if="item.id === 'about' && updateAvailable" class="update-dot" aria-label="Update available" />
          <AppIcon class="nav-chevron" name="chevron-right" :size="20" />
        </button>
        <p class="local-note">Preferences stay on this device.</p>
      </nav>
      <div ref="panel" class="settings-panel" role="region" :aria-label="sectionInfo.label" tabindex="-1">
        <template v-if="selected === 'library'">
          <div class="settings-group">
            <h3>Music folder</h3>
            <p>Your files are the source of your library.</p>
            <div class="folder-card"><AppIcon name="folder-open" /><span>{{ dataDir || 'Music folder unavailable' }}</span><button class="icon-label" @click="emit('openFolder')">{{ dataDir ? 'Open folder' : 'Try again' }}<AppIcon v-if="dataDir" name="external-link" /></button></div>
          </div>
          <div class="settings-group">
            <h3>Library maintenance</h3>
            <div class="setting-row"><div><h4>Show duplicates</h4><p>Include tracks marked as duplicates in your library and playlists.</p></div><SettingsToggle :model-value="showDuplicateTracks" label="Show duplicates" @update:model-value="emit('update:showDuplicateTracks', $event)" /></div>
            <div class="setting-row"><div><h4>Reindex library</h4><p>Scan your music folder for new or changed files.</p></div><button :disabled="indexBusy" @click="emit('reindex')">{{ indexBusy ? 'Scanning...' : 'Reindex' }}</button></div>
            <div v-if="indexBusy" class="scan-progress" role="status"><progress :value="indexTotal ? indexCurrent : undefined" :max="indexTotal || 1" aria-label="Library scanning progress" /><p>{{ indexTotal ? `${indexCurrent} of ${indexTotal} files scanned` : 'Looking for music files...' }}</p></div>
            <p v-else-if="indexDone" class="success" role="status">Scan complete. {{ indexAdded }} files added or updated.</p>
            <div class="setting-row"><div><h4>Duplicates</h4><p>Review matching tracks and choose which ones to keep visible.</p></div><button @click="emit('duplicates')">Review duplicates</button></div>
            <div class="setting-row"><div><h4>Identify tracks</h4><p>Look up missing metadata using audio fingerprints. Requires an internet connection.</p></div><button :disabled="identifyBusy" @click="emit('identify')">{{ identifyBusy ? 'Identifying...' : 'Identify tracks' }}</button></div>
            <p v-if="actionError" class="error" role="alert">{{ actionError }}</p><p v-if="actionMessage" class="help" role="status">{{ actionMessage }}</p>
          </div>
        </template>

        <template v-else-if="selected === 'playback'">
          <div v-if="remoteOutputLabel" class="notice"><strong>Playing on {{ remoteOutputLabel }}</strong><p>Choosing a local output transfers playback to this device.</p></div>
          <div class="settings-group">
            <div class="setting-row"><div><h3>Audio output</h3><p>Choose where Player sends audio on this device.</p></div><button :disabled="audioLoading || audioBusy" @click="emit('refreshAudio')">{{ audioLoading ? 'Refreshing...' : 'Refresh' }}</button></div>
            <label class="field">
              <span>Output device</span>
              <select class="audio-select" :value="selectedDevice ?? ''" :disabled="audioLoading || audioBusy" @change="emit('selectOutput', ($event.target as HTMLSelectElement).value)">
                <option value="">System default</option>
                <option v-if="currentDevice && !currentOutput" :value="currentDevice">{{ currentDevice }}</option>
                <option v-for="device in outputDevices" :key="device.name" :value="device.name">{{ device.name }}</option>
              </select>
            </label>
            <label class="field">
              <span>Sample rate</span>
              <select class="audio-select" :value="currentSampleRate ?? ''" :disabled="audioLoading || audioBusy || !outputDevices.length" @change="emit('selectSampleRate', ($event.target as HTMLSelectElement).value ? Number(($event.target as HTMLSelectElement).value) : null)">
                <option value="">Auto</option>
                <option v-for="rate in sampleRates" :key="rate" :value="rate">{{ formatSampleRate(rate) }}</option>
              </select>
            </label>
            <p class="help">Auto uses the track sample rate. Available rates depend on your output device.</p>
            <p v-if="nativeSampleRate" class="help">Current track: {{ formatSampleRate(nativeSampleRate) }}</p>
            <p v-if="audioBusy" role="status">Switching audio output...</p>
            <p v-if="audioError" class="error" role="alert">{{ audioError }}</p>
          </div>
        </template>

        <template v-else-if="selected === 'devices' || selected === 'soulseek'">
          <p v-if="configLoading" role="status">Loading device settings...</p>
          <div v-else-if="configError" class="notice"><p class="error" role="alert">{{ configError }}</p><button @click="loadConfig">Try again</button></div>
          <template v-else-if="config">
            <template v-if="selected === 'devices'">
              <form class="settings-group" @submit.prevent="saveIdentity">
                <h3>Device identity</h3><p>This is how this Player appears to other devices.</p>
                <div class="device-preview"><span class="device-emoji">{{ emoji }}</span><div><strong>{{ deviceName.trim() || 'My Player' }}</strong><p>This device</p></div></div>
                <label class="field"><span>Device name</span><input v-model="deviceName" placeholder="My Player" autocomplete="off" maxlength="100" :disabled="identitySaving" /></label>
                <fieldset :disabled="identitySaving"><legend>Device emoji</legend><div class="emoji-picker"><button v-for="item in emojiOptions" :key="item" type="button" :class="{ chosen: emoji === item }" :aria-pressed="emoji === item" :aria-label="`Use ${item} as device emoji`" @click="emoji = item">{{ item }}</button></div></fieldset>
                <p v-if="identityError" class="error" role="alert">{{ identityError }}</p><p v-if="identitySaved && !identityDirty" class="success" role="status">Device identity saved.</p>
                <div class="form-actions"><button type="button" :disabled="!identityDirty || identitySaving" @click="resetIdentity">Cancel</button><button class="primary" :disabled="!identityDirty || identitySaving || soulseekSaving" type="submit">{{ identitySaving ? 'Saving...' : 'Save' }}</button></div>
              </form>
              <div class="settings-group"><h3>Local network</h3><div class="setting-row"><div><h4>LAN sync</h4><p>Sync music, playlists and listening history with Players on the same network.</p></div><SettingsToggle :model-value="syncEnabled" label="LAN sync" :disabled="syncBusy" @update:model-value="emit('toggleSync')" /></div><p v-if="syncError" class="error" role="alert">{{ syncError }}</p><div class="setting-row"><div><h4>{{ syncActive ? 'Sync in progress' : syncEnabled ? 'Sync is on' : 'Sync is off' }}</h4><p>{{ peersCount }} {{ peersCount === 1 ? 'device' : 'devices' }} found on your network.</p></div><button class="icon-label" @click="emit('devices')">Open devices <AppIcon name="arrow-right" /></button></div></div>
            </template>
            <template v-else>
              <div class="settings-group"><div class="setting-row"><div><h3>Soulseek search</h3><p>Search the Soulseek network and download music into your library.</p></div><SettingsToggle :model-value="soulseekEnabled" label="Soulseek search" :disabled="soulseekSaving || identitySaving" @update:model-value="toggleSoulseek" /></div></div>
              <form v-if="soulseekEnabled" class="settings-group" @submit.prevent="saveSoulseek">
                <h3>Connection</h3><p>Use your Soulseek account to search the network.</p>
                <label class="field"><span>Username</span><input v-model="username" autocomplete="username" spellcheck="false" :disabled="soulseekSaving" /></label>
                <label class="field"><span>Password</span><span class="password-field"><input v-model="password" :type="showPassword ? 'text' : 'password'" autocomplete="current-password" :disabled="soulseekSaving" /><button type="button" :aria-pressed="showPassword" @click="showPassword = !showPassword">{{ showPassword ? 'Hide' : 'Show' }}</button></span></label>
                <p v-if="soulseekSaved && !soulseekDirty" class="success" role="status">Soulseek credentials saved.</p>
                <div class="form-actions"><button type="button" :disabled="!soulseekDirty || soulseekSaving" @click="resetSoulseek">Cancel</button><button class="primary" type="submit" :disabled="!soulseekDirty || soulseekSaving || identitySaving || !username.trim() || !password">{{ soulseekSaving ? 'Saving...' : 'Save' }}</button></div>
              </form>
              <p v-if="soulseekError" class="error" role="alert">{{ soulseekError }}</p><p v-if="actionError" class="error" role="alert">{{ actionError }}</p>
              <div class="settings-group"><h3>Downloads</h3><p>Downloaded files are saved in the Soulseek folder inside your music library.</p><div class="folder-card"><span>{{ dataDir ? `${dataDir.replace(/[\\/]$/, '')}/Soulseek` : 'Loading folder...' }}</span><button class="icon-label" :disabled="!dataDir" @click="emit('openFolder')">Open music folder <AppIcon name="external-link" /></button></div></div>
            </template>
          </template>
        </template>

        <template v-else-if="selected === 'appearance'">
          <div class="appearance-preview" :class="{ 'preview-flat': !preferences.album3d || reducedMotion }" aria-label="Appearance preview">
            <div class="preview-art"><WebGLAlbumRenderer v-if="preferences.album3d && !reducedMotion" :cover-url="coverUrl" :colors="coverColors" rarity-color="#ff8000" :beat-scale="1" :is-playing="false" :tilt-x="4" :tilt-y="-12" :offset-x="0" :offset-y="0" :dragging="false" /><div v-else class="flat-art" :style="coverUrl ? { backgroundImage: `url(${coverUrl})` } : { background: `linear-gradient(135deg, ${coverColors[0]}, ${coverColors[1]})` }" /></div>
            <div class="preview-track" role="list" aria-label="Track rarity previews"><slot name="appearance-track" /></div>
          </div>
          <div class="settings-group"><div class="setting-row"><div><h4>Reduce motion</h4><p>{{ systemReducedMotion ? 'Enabled by your system preferences.' : 'Keep covers still and turn off decorative animations.' }}</p></div><SettingsToggle :model-value="reducedMotion" label="Reduce motion" :disabled="systemReducedMotion" @update:model-value="setPreference('reduceMotion', $event)" /></div><div class="setting-row"><div><h4>3D album cover</h4><p>{{ reducedMotion ? 'A still cover is used while reduced motion is on.' : 'Show the interactive album cover in now playing.' }}</p></div><SettingsToggle :model-value="preferences.album3d" label="3D album cover" :disabled="reducedMotion" @update:model-value="setPreference('album3d', $event)" /></div><div class="setting-row"><div><h4>Rarity animations</h4><p>{{ reducedMotion ? 'Animations are paused while reduced motion is on.' : 'Animate rare tracks while keeping their rarity colors.' }}</p></div><SettingsToggle :model-value="preferences.rarityAnimations" label="Rarity animations" :disabled="reducedMotion" @update:model-value="setPreference('rarityAnimations', $event)" /></div><p v-if="storageError" class="error" role="alert">{{ storageError }}</p></div>
        </template>
        <slot v-else-if="selected === 'about'" name="about" />
      </div>
    </div>
  </section>
</template>

<style scoped>
.settings-page { max-width: 1120px; margin: 0 auto; padding: 12px 8px 48px; }
p { color: #a7a7a7; font-size: 13px; line-height: 1.65; margin: 0; }
.settings-layout { display: grid; grid-template-columns: 184px minmax(0, 1fr); gap: 44px; align-items: start; }
.settings-nav { display: flex; flex-direction: column; gap: 4px; position: sticky; top: 0; }
.settings-nav > button { display: flex; align-items: center; gap: 12px; padding: 13px 12px; border: 0; border-radius: 6px; color: #a7a7a7; background: transparent; font-size: 13px; font-weight: 600; text-align: left; cursor: pointer; }
.settings-nav > button:where(:hover, :active) { background: rgb(255 255 255 / 6%); color: #fff; }
.settings-nav > button.selected { background: #1b2920; color: #81dfa2; }
.settings-nav svg { width: 19px; height: 19px; flex-shrink: 0; }
.nav-copy { flex: 1; }
.nav-description, .nav-chevron { display: none; }
.local-note { font-size: 11px; padding: 20px 12px 0; }
.update-dot { width: 6px; height: 6px; border-radius: 50%; background: #1db954; }
.settings-panel { min-width: 0; max-width: 700px; background: transparent; }
.settings-panel:focus { outline: none; }
h3 { font-size: 15px; margin: 0 0 7px; font-weight: 700; }
h4 { font-size: 14px; margin: 0 0 5px; font-weight: 500; }
.settings-group { margin-bottom: 32px; }
.setting-row { display: flex; align-items: center; justify-content: space-between; gap: 24px; padding: 21px 0; border-bottom: 1px solid #242424; }
.setting-row > div { min-width: 0; }
.setting-row p { max-width: 390px; }
button, input { font: inherit; }
.settings-panel button:where(:not(.settings-switch)), .mobile-back { border: 1px solid #505050; background: transparent; color: #fff; border-radius: 99px; padding: 9px 16px; font-size: 12px; font-weight: 600; cursor: pointer; flex-shrink: 0; }
.settings-panel .icon-label { display: inline-flex; align-items: center; gap: 6px; }
.settings-panel button:where(:not(.settings-switch):enabled):is(:hover, :active) { background: rgb(255 255 255 / 6%); border-color: #777; }
.settings-panel button:not(.settings-switch):disabled { opacity: .45; cursor: default; }
button:focus-visible, select:focus-visible, input:focus-visible { outline: 2px solid #1db954; outline-offset: 3px; }
.settings-panel :deep(.settings-switch) { padding: 3px; border-radius: 99px; }
.settings-panel .primary { background: #1db954; border-color: #1db954; color: #061d0e; padding-inline: 24px; }
.settings-panel .primary:enabled:is(:hover, :active) { background: #35cd6b; border-color: #35cd6b; }
.folder-card { display: flex; flex-wrap: wrap; align-items: center; gap: 14px; margin-top: 18px; padding: 18px; border-radius: 8px; }
.folder-card svg { width: 24px; height: 24px; color: #a7a7a7; flex-shrink: 0; }
.folder-card > span { flex: 1; min-width: 120px; overflow-wrap: anywhere; font-size: 13px; color: #d3d3d3; }
.field { display: flex; flex-direction: column; gap: 9px; margin-top: 22px; font-size: 13px; font-weight: 600; }
.field input { border: 1px solid #505050; background: transparent; color: #fff; border-radius: 5px; padding: 12px; width: 100%; min-height: 44px; font-size: 14px; }
.field input:disabled { opacity: .5; }
.field .audio-select { width: 100%; min-height: 44px; }
.help { font-size: 12px; margin-top: 12px; }
.notice { border-radius: 8px; padding: 16px; margin-bottom: 24px; }
.notice strong { font-size: 13px; display: block; margin-bottom: 6px; }
.device-preview { display: flex; align-items: center; gap: 16px; padding: 20px; margin-top: 20px; border-radius: 8px; }
.device-preview strong { font-size: 15px; overflow-wrap: anywhere; }
.device-emoji { display: grid; place-items: center; width: 44px; height: 44px; border-radius: 8px; background: #262626; font-size: 26px; flex-shrink: 0; }
fieldset { border: 0; padding: 0; margin: 22px 0 0; }
legend { font-size: 13px; font-weight: 600; margin-bottom: 12px; }
.emoji-picker { display: flex; flex-wrap: wrap; gap: 8px; }
.settings-panel .emoji-picker button { width: 38px; height: 38px; border-radius: 6px; padding: 0; font-size: 22px; border-color: transparent; }
.settings-panel .emoji-picker button.chosen { background: #1b2920; border-color: #1db954; }
.form-actions { display: flex; justify-content: flex-end; gap: 10px; margin-top: 24px; }
.error { color: #ff939b; margin-top: 14px; overflow-wrap: anywhere; }
.success { color: #81dfa2; margin-top: 14px; }
.password-field { display: flex; border: 1px solid #505050; border-radius: 5px; background: transparent; }
.password-field input { border: 0; background: transparent; min-width: 0; }
.settings-panel .password-field button { border: 0; border-radius: 0 4px 4px 0; }
.scan-progress { padding-top: 16px; }
.scan-progress p { color: #a7a7a7; font-size: 12px; font-variant-numeric: tabular-nums; }
.scan-progress progress {
  display: block;
  width: 100%;
  height: 4px;
  margin-bottom: 10px;
  border: 0;
  border-radius: 99px;
  overflow: hidden;
  appearance: none;
  -webkit-appearance: none;
  background: rgb(255 255 255 / 12%);
  color: #1db954;
}
.scan-progress progress::-webkit-progress-bar { background: rgb(255 255 255 / 12%); border-radius: 99px; }
.scan-progress progress::-webkit-progress-value { background: #1db954; border-radius: 99px; }
.scan-progress progress::-moz-progress-bar { background: #1db954; border-radius: 99px; }
.scan-progress progress:indeterminate {
  background: linear-gradient(90deg, transparent, #1db954, transparent) -100% 0 / 50% 100% no-repeat, rgb(255 255 255 / 12%);
  animation: scan-search 1.6s ease-in-out infinite;
}
.scan-progress progress:indeterminate::-webkit-progress-bar { background: transparent; }
.scan-progress progress:indeterminate::-moz-progress-bar { background: transparent; }
@keyframes scan-search { to { background-position: 200% 0, 0 0; } }
@media (prefers-reduced-motion: reduce) {
  .scan-progress progress:indeterminate { animation: none; background-position: 50% 0, 0 0; }
}
:global(.player-reduced-motion) .scan-progress progress:indeterminate { animation: none; background-position: 50% 0, 0 0; }
.appearance-preview { display: grid; grid-template-columns: clamp(88px, 32%, 208px) minmax(0, 1fr); align-items: center; gap: 24px; margin-bottom: 18px; padding: 24px 0; background: inherit; border-radius: 6px; overflow: hidden; container: appearance-preview / inline-size; }
.preview-track { display: flex; flex-direction: column; gap: 4px; width: 100%; min-width: 0; }
.preview-art { position: relative; justify-self: start; width: 100%; max-width: 208px; aspect-ratio: 1; }
.preview-art :deep(.webgl-album-renderer) { top: 0; }
.flat-art { position: absolute; inset: 16%; border-radius: 4px; background-size: cover; background-position: center; }
.mobile-back { display: none; }
@media (max-width: 1080px) { .settings-layout { grid-template-columns: 160px minmax(0, 1fr); gap: 24px; } .setting-row { gap: 16px; } .appearance-preview { gap: 16px; padding: 16px 0; } }
@container appearance-preview (max-width: 360px) {
  .preview-track :deep(.track-num), .preview-track :deep(.track-dur) { display: none; }
}
@media (max-width: 768px) {
  .settings-page { padding: 8px 0 32px; }
  .appearance-preview { grid-template-columns: minmax(0, 1fr); }
  .settings-layout { display: block; }
  .settings-nav { position: static; gap: 0; }
  .settings-nav > button { padding: 18px 4px; border-bottom: 1px solid #282828; border-radius: 0; gap: 16px; min-height: 76px; }
  .settings-nav > button.selected { background: transparent; color: #b3b3b3; }
  .settings-nav svg { width: 22px; height: 22px; }
  .nav-label { color: #fff; font-size: 15px; }
  .nav-description { display: block; color: #a7a7a7; font-size: 12px; font-weight: 400; margin-top: 6px; }
  .nav-chevron { display: block; color: #888; }
  .local-note { padding: 24px 4px; }
  .settings-panel { display: none; }
  .has-section .settings-nav { display: none; }
  .has-section .settings-panel { display: block; }
  .mobile-back { display: flex; gap: 10px; align-items: center; padding: 8px 0; margin-bottom: 10px; border: 0; border-radius: 3px; color: #b3b3b3; font-size: 14px; }
  .setting-row { align-items: flex-start; flex-wrap: wrap; gap: 14px; }
  .setting-row > div { flex: 1; min-width: 180px; }
  .setting-row :deep(.settings-switch) { margin-top: 2px; }
  .folder-card { padding: 16px; }
  .settings-panel button:not(.settings-switch) { min-height: 40px; }
  .settings-panel :deep(.settings-switch) { min-height: 24px; }
}
</style>
