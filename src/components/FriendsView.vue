<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import AppIcon from './AppIcon.vue';
import ResponsivePopup from './ResponsivePopup.vue';
import { useTransientHistoryEntry } from '../composables/useHistoryNavigation';
import type { FriendProfile as Friend } from '../types/friends';

defineProps<{ profileId?: string }>();
const emit = defineEmits<{ openDeviceSettings: []; openFriend: [friend: Friend] }>();
const friends = ref<Friend[]>([]);
const loading = ref(true);
const error = ref('');
const dialogError = ref('');
const showAdd = ref(false);
const busy = ref(false);
const removing = ref('');
const resolving = ref('');
const friendId = ref('');
const name = ref('');
const query = ref('');
const addButton = ref<HTMLButtonElement | null>(null);
let disposed = false;
const filteredFriends = computed(() => {
  const text = query.value.trim().toLowerCase();
  return friends.value.filter(friend => `${friend.name} ${friend.profile_id} ${friend.soulseek_users.join(" ")}`.toLowerCase().includes(text));
});

async function load() {
  loading.value = true;
  error.value = '';
  try { friends.value = await invoke<Friend[]>('get_friends'); }
  catch { error.value = 'Could not load your friends. Please try again.'; }
  finally { loading.value = false; }
}

function visibleDialog() {
  return [...document.querySelectorAll<HTMLElement>('[data-friend-dialog]')]
    .find(element => element.getClientRects().length > 0);
}
async function openAdd() {
  friendId.value = '';
  name.value = '';
  dialogError.value = '';
  showAdd.value = true;
  await nextTick();
  visibleDialog()?.querySelector<HTMLInputElement>('input')?.focus();
}
function closeAdd() {
  if (busy.value) return;
  showAdd.value = false;
  addButton.value?.focus();
}
useTransientHistoryEntry(showAdd, closeAdd);

async function addFriend() {
  if (busy.value || !friendId.value.trim()) return;
  busy.value = true;
  dialogError.value = '';
  const savedId = friendId.value.trim();
  try {
    await invoke('add_friend', { profileId: savedId, name: name.value.trim() });
  } catch (cause) {
    dialogError.value = String(cause);
    return;
  } finally { busy.value = false; }
  if (disposed) return;
  closeAdd();
  await load();
  if (!disposed) await resolveFriend(savedId, true);
}

async function resolveFriend(profileId: string, silent = false) {
  if (resolving.value) return;
  resolving.value = profileId;
  error.value = '';
  try {
    await invoke('resolve_friend', { profileId });
    if (!disposed) await load();
  } catch (cause) {
    if (!disposed && !silent) error.value = `Soulseek lookup is unavailable. ${String(cause)}`;
  } finally { resolving.value = ''; }
}

async function removeFriend(friend: Friend) {
  if (removing.value) return;
  removing.value = friend.profile_id;
  error.value = '';
  try {
    await invoke('remove_friend', { profileId: friend.profile_id });
    friends.value = friends.value.filter(item => item.profile_id !== friend.profile_id);
  } catch { error.value = 'Could not remove this friend. Please try again.'; }
  finally { removing.value = ''; }
}

function onKeydown(event: KeyboardEvent) {
  if (!showAdd.value) return;
  if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); closeAdd(); }
  if (event.key === 'Tab') {
    const controls = [...(visibleDialog()?.querySelectorAll<HTMLElement>('input, button:not(:disabled)') ?? [])];
    const first = controls[0];
    const last = controls[controls.length - 1];
    if (first && last && (event.shiftKey ? document.activeElement === first : document.activeElement === last)) {
      event.preventDefault();
      (event.shiftKey ? last : first).focus();
    }
  }
}
onMounted(() => { void load(); document.addEventListener('keydown', onKeydown, true); });
onUnmounted(() => { disposed = true; document.removeEventListener('keydown', onKeydown, true); });
</script>

<template>
  <section class="friends-view">
    <div class="friends-toolbar">
      <input v-model="query" class="friends-search" type="text" placeholder="Search friends" aria-label="Search friends" />
      <button ref="addButton" class="friends-add" @click="openAdd">
        <AppIcon name="user-plus" :size="18" /> Add friend
      </button>
    </div>
    <div v-if="error" class="friends-error" role="alert">
      {{ error }} <button class="friends-text-button" @click="load">Try again</button>
    </div>
    <div v-if="loading" class="friends-empty" role="status">Loading friends...</div>
    <div v-else-if="!friends.length && !error" class="friends-empty">
      <AppIcon name="users" :size="40" />
      <h2>No friends yet</h2>
      <p>Add a friend using their Player Profile ID.</p>
      <p class="friends-note">Find your Profile ID in <a href="#" class="friends-settings-link" @click.prevent="emit('openDeviceSettings')">Devices &amp; Sync</a></p>
    </div>
    <div v-else-if="!filteredFriends.length && !error" class="friends-empty">No friends match your search.</div>
    <ul v-else class="friends-list" aria-label="Friends">
      <li v-for="friend in filteredFriends" :key="friend.profile_id" class="friend-row">
        <button class="friend-open" :aria-label="`Open chat with ${friend.name || 'Player friend'}`" @click="emit('openFriend', friend)">
        <div class="friend-avatar" aria-hidden="true">
          <span v-if="friend.name">{{ [...friend.name][0]?.toUpperCase() }}</span>
          <AppIcon v-else name="users" :size="22" />
        </div>
        <div class="friend-info">
          <span class="friend-name">{{ friend.name || 'Player friend' }}</span>
          <span class="friend-id">{{ friend.profile_id }}</span>
          <span v-if="resolving === friend.profile_id" class="friend-discovery" role="status">Searching Soulseek...</span>
          <span v-else-if="friend.soulseek_users.length" class="friend-discovery">Soulseek: {{ friend.soulseek_users.join(', ') }} <span class="friend-unverified">Unverified</span></span>
        </div>
        </button>
        <button class="friend-remove" :disabled="!!resolving || !!removing" :aria-label="`Find ${friend.name || friend.profile_id} on Soulseek`" title="Find on Soulseek" @click="resolveFriend(friend.profile_id)">
          <AppIcon name="refresh-cw" :size="18" :class="{ 'friend-searching': resolving === friend.profile_id }" />
        </button>
        <button class="friend-remove" :disabled="!!removing || resolving === friend.profile_id" :aria-label="`Remove ${friend.name || friend.profile_id}`" title="Remove friend" @click="removeFriend(friend)">
          <AppIcon name="x" :size="18" />
        </button>
      </li>
    </ul>
    <p v-if="friends.length" class="friends-note">Friends are saved on this device. Soulseek matches are discovery hints and do not verify a profile.</p>

    <ResponsivePopup :open="showAdd" centered panel-class="friends-popup" @close="closeAdd">
      <form data-friend-dialog class="friends-form" role="dialog" aria-modal="true" aria-label="Add friend" :aria-busy="busy" @submit.prevent="addFriend">
        <div class="friends-dialog-heading">
          <h2>Add friend</h2>
          <button type="button" class="friend-remove" aria-label="Close" :disabled="busy" @click="closeAdd"><AppIcon name="x" :size="20" /></button>
        </div>
        <p>Enter their Player Profile ID to add them to your friends.</p>
        <p class="friends-note">Soulseek discovery is optional. Your friend can be saved without a Soulseek account.</p>
        <label class="friends-field">Profile ID
          <input v-model="friendId" required maxlength="64" placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" autocomplete="off" autocapitalize="off" spellcheck="false" :disabled="busy" />
        </label>
        <label class="friends-field">Name <span class="friends-optional">Optional</span>
          <input v-model="name" maxlength="100" placeholder="How you know them" autocomplete="off" :disabled="busy" />
        </label>
        <p v-if="profileId" class="friends-note">Your Profile ID <span class="friend-id friends-own-id">{{ profileId }}</span></p>
        <p v-if="dialogError" class="friends-error" role="alert">{{ dialogError }}</p>
        <div class="friends-dialog-actions">
          <button type="button" class="friends-cancel" :disabled="busy" @click="closeAdd">Cancel</button>
          <button class="friends-add" type="submit" :disabled="busy || !friendId.trim()">{{ busy ? 'Adding...' : 'Add friend' }}</button>
        </div>
      </form>
    </ResponsivePopup>
  </section>
</template>

<style>
.responsive-popup-desktop.friends-popup { width: min(440px, calc(100vw - 24px)); padding: 0; border-radius: 12px; }
.responsive-popup-mobile-panel.friends-popup { width: 100%; }
</style>

<style scoped>
.friends-view { width: 100%; min-width: 0; }
.friends-toolbar { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 16px; margin-bottom: 24px; }
.friends-search { background: #282828; border: none; border-radius: 4px; color: #fff; font-size: var(--fs-input); padding: 8px 12px; width: 220px; min-width: 0; outline: none; }
.friends-search::placeholder { color: #777; }
.friends-search:focus { outline: 1px solid #555; }
button { font: inherit; cursor: pointer; }
button:disabled { opacity: .5; cursor: default; }
button:focus-visible, .friends-field input:focus-visible { outline: 2px solid #1db954; outline-offset: 3px; }
.friends-add { display: inline-flex; align-items: center; justify-content: center; gap: 8px; padding: 10px 18px; border: 0; border-radius: 24px; background: #1db954; color: #000; font-size: 14px; font-weight: 700; white-space: nowrap; }
.friends-add:hover:not(:disabled) { background: #1ed760; }
.friends-empty { display: flex; align-items: center; flex-direction: column; justify-content: center; min-height: 240px; padding: 32px 16px; text-align: center; color: #b3b3b3; }
.friends-empty h2 { color: #fff; font-size: 22px; margin: 20px 0 8px; }
.friends-empty p { margin: 4px 0; font-size: 14px; line-height: 1.6; }
.friends-list { list-style: none; padding: 0; margin: 0; }
.friend-row { display: flex; gap: 16px; align-items: center; padding: 12px; border-radius: 6px; }
.friend-open { display: flex; align-items: center; gap: 16px; flex: 1; min-width: 0; padding: 0; border: 0; background: transparent; color: inherit; text-align: left; }
.friend-row:hover { background: rgba(255,255,255,.06); }
.friend-avatar { display: flex; align-items: center; justify-content: center; width: 48px; height: 48px; flex-shrink: 0; border-radius: 50%; background: rgba(29,185,84,.12); color: #8de2a7; font-size: 20px; font-weight: 600; }
.friend-info { min-width: 0; flex: 1; display: flex; flex-direction: column; gap: 6px; }
.friend-name { color: #fff; font-size: 15px; overflow-wrap: anywhere; }
.friend-discovery { color: #b3b3b3; font-size: 12px; line-height: 1.5; overflow-wrap: anywhere; }
.friend-unverified { color: #888; margin-left: 8px; }
.friend-searching { animation: friend-spin 1s linear infinite; }
@keyframes friend-spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .friend-searching { animation: none; } }
.friend-id { color: #b3b3b3; font-family: monospace; font-size: 12px; overflow-wrap: anywhere; user-select: text; }
.friend-remove { display: flex; align-items: center; justify-content: center; border: 0; padding: 8px; background: transparent; color: #b3b3b3; border-radius: 50%; flex-shrink: 0; }
.friend-remove:hover:not(:disabled) { color: #fff; background: rgba(255,255,255,.08); }
.friends-note { color: #888; font-size: 12px; line-height: 1.6; margin: 20px 12px; }
.friends-settings-link { color: #b3b3b3; text-decoration: underline; text-underline-offset: 3px; }
.friends-settings-link:hover { color: #fff; }
.friends-settings-link:focus-visible { outline: 2px solid #1db954; outline-offset: 3px; }
.friends-form { padding: 24px; color: #fff; }
.friends-dialog-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.friends-dialog-heading h2 { margin: 0; font-size: 22px; }
.friends-form p { color: #b3b3b3; font-size: 14px; line-height: 1.6; margin: 16px 0; }
.friends-field { display: block; margin-top: 20px; font-size: 14px; font-weight: 600; }
.friends-optional { color: #888; font-weight: 400; margin-left: 6px; font-size: 12px; }
.friends-field input { display: block; box-sizing: border-box; margin-top: 8px; width: 100%; padding: 12px; background: transparent; border: 1px solid #555; border-radius: 6px; color: #fff; font: inherit; font-size: 14px; }
.friends-own-id { display: block; margin-top: 4px; }
.friends-error, .friends-form .friends-error { color: #ff9a9a; font-size: 14px; line-height: 1.5; }
.friends-text-button { color: #fff; background: transparent; border: 0; text-decoration: underline; padding: 8px; }
.friends-dialog-actions { display: flex; gap: 12px; justify-content: flex-end; margin-top: 24px; }
.friends-cancel { border: 1px solid #777; border-radius: 24px; background: transparent; color: #fff; padding: 10px 18px; font-size: 14px; font-weight: 600; }
.friends-cancel:hover:not(:disabled) { border-color: #fff; background: rgba(255,255,255,.06); }
@media (max-width: 600px) {
  .friends-search { width: 100%; }
  .friends-toolbar { gap: 12px; }
  .friend-row { padding: 12px 0; gap: 12px; }
  .friend-id { font-size: 11px; }
  .friends-form { padding: 20px; }
}
</style>
