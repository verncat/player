<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import AppIcon from './AppIcon.vue';
import ResponsivePopup from './ResponsivePopup.vue';

interface ProfileInfo { user_id: string; public_key: string }
interface PairingPeer {
  host: string;
  port: number;
  name: string;
  device_name?: string;
  device_id?: string | null;
  profile_public_key?: string | null;
}
interface PairingView {
  phase: string;
  role: 'join' | 'share' | null;
  peer_name: string | null;
  code: string | null;
  confirmed: boolean;
  expires_in: number;
  error: string | null;
}

const emit = defineEmits<{ profile: [info: ProfileInfo] }>();
const open = ref(false);
const busy = ref(false);
const checked = ref(false);
const error = ref('');
const view = ref<PairingView | null>(null);
const selectedPeer = ref<PairingPeer | null>(null);
let timer: ReturnType<typeof setTimeout> | undefined;
let generation = 0;
let returnFocus: HTMLElement | null = null;

function visibleDialog() {
  return [...document.querySelectorAll<HTMLElement>('[data-profile-pairing-dialog]')]
    .find(element => element.getClientRects().length > 0);
}

watch(open, async (value) => {
  if (value) {
    returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    await nextTick();
    visibleDialog()?.focus();
  } else if (returnFocus?.isConnected) {
    returnFocus.focus();
  }
});

const title = computed(() => {
  if (view.value?.phase === 'complete') return 'Profile linked';
  if (view.value?.code) return 'Compare the code';
  return selectedPeer.value ? 'Use this profile' : 'Link a new device';
});
const terminal = computed(() => ['complete', 'error', 'expired', 'cancelled'].includes(view.value?.phase ?? ''));
const peerName = computed(() => view.value?.peer_name || selectedPeer.value?.device_name || selectedPeer.value?.name || 'the other device');

async function refreshProfile() {
  emit('profile', await invoke<ProfileInfo>('local_profile_info'));
}

function schedulePoll(token: number) {
  clearTimeout(timer);
  timer = setTimeout(async () => {
    if (!open.value || token !== generation) return;
    try {
      const next = await invoke<PairingView>('profile_pairing_poll');
      if (token !== generation) return;
      view.value = next;
      if (next.phase === 'complete') await refreshProfile();
      else if (!terminal.value) schedulePoll(token);
    } catch (cause) {
      if (token === generation) error.value = String(cause);
    }
  }, 1000);
}

function begin(peer: PairingPeer | null) {
  if (open.value) return false;
  generation += 1;
  selectedPeer.value = peer;
  view.value = null;
  checked.value = false;
  error.value = '';
  open.value = true;
  return true;
}

function useProfile(peer: PairingPeer) {
  begin(peer);
}

async function allow() {
  if (!begin(null)) return;
  busy.value = true;
  const token = generation;
  try {
    const next = await invoke<PairingView>('profile_pairing_allow');
    if (token !== generation) return;
    view.value = next;
    schedulePoll(token);
  } catch (cause) {
    if (token === generation) error.value = String(cause);
  } finally {
    if (token === generation) busy.value = false;
  }
}

async function connect() {
  const peer = selectedPeer.value;
  if (!peer || busy.value) return;
  busy.value = true;
  error.value = '';
  const token = generation;
  try {
    const next = await invoke<PairingView>('profile_pairing_join', {
      host: peer.host, port: peer.port, publicKey: peer.profile_public_key, deviceId: peer.device_id,
    });
    if (token !== generation) return;
    view.value = next;
    schedulePoll(token);
  } catch (cause) {
    if (token === generation) error.value = String(cause);
  } finally {
    if (token === generation) busy.value = false;
  }
}

async function confirm() {
  if (!checked.value || busy.value || view.value?.confirmed) return;
  busy.value = true;
  const token = generation;
  try {
    const next = await invoke<PairingView>('profile_pairing_confirm');
    if (token === generation) view.value = next;
  } catch (cause) {
    if (token === generation) error.value = String(cause);
  } finally {
    if (token === generation) busy.value = false;
  }
}

async function close() {
  if (!open.value || busy.value) return;
  clearTimeout(timer);
  // Keep the popup closed until native cleanup finishes so a new session cannot race it.
  busy.value = true;
  generation += 1;
  try {
    await invoke('profile_pairing_cancel');
    await refreshProfile();
    open.value = false;
  } catch (cause) {
    error.value = String(cause);
  } finally {
    busy.value = false;
  }
}

function onKeydown(event: KeyboardEvent) {
  if (open.value && event.key === 'Escape') { event.preventDefault(); void close(); }
  if (open.value && event.key === 'Tab') {
    const dialog = visibleDialog();
    const elements = dialog?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled)');
    const first = elements?.[0];
    const last = elements?.[elements.length - 1];
    if (!first) event.preventDefault();
    else if (event.shiftKey && (document.activeElement === first || document.activeElement === dialog)) {
      event.preventDefault(); last?.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault(); first.focus();
    }
  }
}
onMounted(() => {
  void refreshProfile().catch(cause => { error.value = String(cause); });
  window.addEventListener('keydown', onKeydown);
});
onUnmounted(() => {
  generation += 1;
  clearTimeout(timer);
  window.removeEventListener('keydown', onKeydown);
  if (open.value) void invoke('profile_pairing_cancel').catch(() => {});
});
defineExpose({ allow, useProfile });
</script>

<template>
  <ResponsivePopup :open="open" centered panel-class="profile-pairing-panel" @close="close">
    <section class="profile-pairing" role="dialog" aria-modal="true" :aria-label="title" tabindex="-1" data-profile-pairing-dialog>
      <div class="pairing-heading">
        <AppIcon :name="view?.phase === 'complete' ? 'circle-check' : 'key-round'" :size="24" />
        <h3>{{ title }}</h3>
      </div>
      <template v-if="view?.phase === 'complete'">
        <p v-if="view.role === 'share'">Your profile has been sent to {{ peerName }}.</p>
        <p v-else>This device now uses the shared profile.</p>
        <p>Linked devices can add other devices. There is no main player.</p>
      </template>
      <template v-else-if="view?.phase === 'expired'">
        <p>The linking session expired. Start again on both devices.</p>
      </template>
      <template v-else-if="view?.phase === 'cancelled'">
        <p>Linking was cancelled.</p>
      </template>
      <template v-else-if="view?.code">
        <p v-if="view.role === 'share'">Share your profile with <strong>{{ peerName }}</strong>. This device will be able to link other devices.</p>
        <p v-else>Use the profile from <strong>{{ peerName }}</strong>. This replaces the profile on this device. Your music library stays here.</p>
        <p>Compare all three groups on both screens.</p>
        <output class="pairing-code" aria-label="Pairing verification code">{{ view.code }}</output>
        <p v-if="view.confirmed" class="pairing-status" role="status">Waiting for confirmation on the other device.</p>
        <label v-else class="pairing-check">
          <input v-model="checked" type="checkbox" :disabled="busy">
          <span>The codes match on both devices</span>
        </label>
      </template>
      <template v-else-if="selectedPeer && !view">
        <p>Use the profile from <strong>{{ peerName }}</strong> on this device. Your current profile will be replaced here.</p>
        <p>On {{ peerName }}, open Devices and choose Allow linking. Then continue here.</p>
      </template>
      <template v-else-if="!error && !view?.error">
        <p>On the new device, open Devices and choose Use profile next to this player.</p>
        <p>Keep both players open. You will compare a code before sharing your profile.</p>
        <p class="pairing-status" role="status">{{ busy ? 'Starting linking session' : view?.phase === 'connecting' ? 'Connecting to the new device' : 'Waiting for a new device' }}</p>
      </template>
      <p v-if="view?.expires_in && !terminal" class="pairing-expiry">Expires in {{ Math.ceil(view.expires_in / 60) }} min</p>
      <p v-if="error || view?.error" class="pairing-error" role="alert">{{ error || view?.error }}</p>
      <div class="pairing-actions">
        <button class="pairing-button" :disabled="busy" @click="close">{{ terminal ? 'Done' : 'Cancel' }}</button>
        <button v-if="selectedPeer && !view" class="pairing-button primary" :disabled="busy" @click="connect">{{ busy ? 'Connecting' : 'Continue' }}</button>
        <button v-if="view?.code && !view.confirmed && !terminal" class="pairing-button primary" :disabled="busy || !checked" @click="confirm">Codes match</button>
      </div>
    </section>
  </ResponsivePopup>
</template>

<style>
.profile-pairing-panel { width: min(440px, calc(100vw - 24px)); padding: 0; }
.responsive-popup-mobile-panel.profile-pairing-panel { width: 100%; }
</style>
<style scoped>
.profile-pairing { padding: 24px; color: var(--text-primary, #fff); }
.pairing-heading { display: flex; align-items: center; gap: 12px; margin-bottom: 20px; }
.pairing-heading h3 { font-size: 20px; margin: 0; }
.pairing-heading :deep(svg) { color: var(--accent, #1db954); flex-shrink: 0; }
p { color: var(--text-secondary, #b3b3b3); line-height: 1.6; font-size: 14px; margin: 12px 0; }
strong { color: var(--text-primary, #fff); }
.pairing-code { display: block; padding: 20px 8px; margin: 20px 0; border-radius: 10px; text-align: center; font-family: monospace; font-size: clamp(19px, 5vw, 27px); letter-spacing: 2px; white-space: nowrap; background: rgba(255,255,255,.06); }
.pairing-check { display: flex; gap: 10px; align-items: center; font-size: 14px; cursor: pointer; line-height: 1.5; }
.pairing-check input { accent-color: var(--accent, #1db954); width: 18px; height: 18px; flex-shrink: 0; }
.pairing-status { color: #8de2a7; }
.pairing-expiry { font-size: 12px; }
.pairing-error { color: #ff9a9a; overflow-wrap: anywhere; }
.pairing-actions { display: flex; justify-content: flex-end; flex-wrap: wrap; gap: 10px; margin-top: 24px; }
.pairing-button { padding: 10px 16px; border-radius: 8px; border: 1px solid rgba(255,255,255,.12); background: rgba(255,255,255,.06); color: #fff; cursor: pointer; font: inherit; font-size: 14px; }
.pairing-button:hover:not(:disabled) { background: rgba(255,255,255,.12); }
.pairing-button.primary { background: rgba(29,185,84,.18); border-color: rgba(29,185,84,.3); color: #8de2a7; }
.pairing-button.primary:hover:not(:disabled) { background: rgba(29,185,84,.28); }
.pairing-button:disabled { opacity: .45; cursor: default; }
.pairing-button:focus-visible { outline: 2px solid #8de2a7; outline-offset: 3px; }
</style>
