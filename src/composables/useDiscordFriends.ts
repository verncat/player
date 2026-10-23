import { computed, onMounted, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';

export interface DiscordTrack { title: string; artist: string; album: string }
export interface DiscordFriend { profile_id: string; id: string; name: string; avatar: string; track: DiscordTrack | null }
interface DiscordSocialStatus { supported: boolean; connecting: boolean; connected: boolean; error: string; friends: DiscordFriend[] }
const status = ref<DiscordSocialStatus>({ supported: false, connecting: false, connected: false, error: '', friends: [] });
const busy = ref(false);
const error = ref('');
let consumers = 0;
let timer: ReturnType<typeof setTimeout> | undefined;
let polling = false;

async function refresh() {
  try { status.value = await invoke<DiscordSocialStatus>('discord_social_status'); }
  catch { status.value = { ...status.value, connected: false, friends: [] }; }
}
async function poll() {
  if (polling || !consumers) return;
  polling = true;
  await refresh();
  polling = false;
  if (consumers) timer = setTimeout(() => { void poll(); }, 2000);
}

export function useDiscordFriends() {
  async function connect() {
    if (busy.value) return;
    busy.value = true; error.value = '';
    try { await invoke('discord_social_connect'); await refresh(); }
    catch (cause) { error.value = String(cause); }
    finally { busy.value = false; }
  }
  async function disconnect() {
    if (busy.value) return;
    busy.value = true; error.value = '';
    try { await invoke('discord_social_disconnect'); await refresh(); }
    catch (cause) { error.value = String(cause); }
    finally { busy.value = false; }
  }
  onMounted(() => { if (++consumers === 1) { clearTimeout(timer); void poll(); } });
  onUnmounted(() => { if (--consumers === 0) clearTimeout(timer); });
  const byProfile = computed(() => new Map(status.value.friends.map(friend => [friend.profile_id, friend])));
  return { status, busy, error, byProfile, connect, disconnect };
}
