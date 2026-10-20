import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';

export interface PlaybackPresence {
  title: string;
  artist: string;
  album: string;
  position: number;
  duration: number;
  playing: boolean;
}

const STORAGE_KEY = 'player.discord-presence';

/** Playback adapter only: the integration can later move behind a plugin boundary. */
export function useDiscordPresence(playback: () => PlaybackPresence | null) {
  const enabled = ref(false);
  const available = ref(false);
  const configured = ref(false);
  const ready = ref(false);
  const storageError = ref('');
  const updateError = ref('');
  const error = computed(() => storageError.value || updateError.value);
  let disposed = false;
  let last: { enabled: boolean; playback: PlaybackPresence | null; at: number } | null = null;
  let pending: { enabled: boolean; playback: PlaybackPresence | null } | null = null;
  let sending = false;

  try { enabled.value = localStorage.getItem(STORAGE_KEY) === 'true'; } catch { /* Keep disabled if storage is unavailable. */ }

  async function flush() {
    if (sending) return;
    sending = true;
    while (pending && !disposed) {
      const next = pending;
      pending = null;
      try {
        await invoke('discord_update_presence', next);
        if (!disposed) updateError.value = '';
      } catch {
        if (!disposed) updateError.value = 'Could not update Discord integration. Try switching it off and on.';
        last = null;
        // Wait for another meaningful playback change or an explicit toggle before retrying.
        break;
      }
    }
    sending = false;
  }

  function publish() {
    if (!ready.value || !available.value || !configured.value || disposed) return;
    const snapshot = enabled.value ? playback() : null;
    const current = snapshot?.playing ? snapshot : null;
    const now = performance.now();
    if (last && last.enabled === enabled.value) {
      if (!last.playback && !current) return;
      if (last.playback && current) {
        const previous = last.playback;
        const expectedPosition = previous.position + (now - last.at) / 1000;
        if (previous.title === current.title && previous.artist === current.artist && previous.album === current.album
          && previous.duration === current.duration && Math.abs(expectedPosition - current.position) < 2) return;
      }
    }
    last = { enabled: enabled.value, playback: current ? { ...current } : null, at: now };
    pending = { enabled: enabled.value, playback: last.playback };
    void flush();
  }

  function setEnabled(value: boolean) {
    try {
      // Disabling must take effect even if localStorage is full or blocked.
      if (!value) enabled.value = false;
      localStorage.setItem(STORAGE_KEY, String(value));
      enabled.value = value;
      storageError.value = '';
      publish();
    } catch {
      storageError.value = 'Could not save Discord preferences on this device.';
      if (!value) publish();
    }
  }

  watch([enabled, playback], publish);
  onMounted(async () => {
    try {
      const status = await invoke<{ supported: boolean; configured: boolean }>('discord_availability');
      if (disposed) return;
      available.value = status.supported;
      configured.value = status.configured;
      ready.value = true;
      publish();
    } catch {
      // Optional integrations never prevent the player from starting.
      ready.value = true;
    }
  });
  onUnmounted(() => {
    disposed = true;
    pending = null;
    if (available.value && configured.value) {
      // App shutdown also clears the native adapter; this covers frontend reloads.
      void invoke('discord_update_presence', { enabled: false, playback: null }).catch(() => {});
    }
  });
  return { enabled, available, configured, ready, error, setEnabled };
}
