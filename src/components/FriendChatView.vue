<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import AppIcon from './AppIcon.vue';
import { useDiscordFriends } from '../composables/useDiscordFriends';
import ResponsivePopup from './ResponsivePopup.vue';
import type { FriendChat, FriendMessage, FriendMessagePage, FriendProfile } from '../types/friends';

const props = defineProps<{ profileId: string }>();
const discord = useDiscordFriends();
const discordFriend = computed(() => discord.byProfile.value.get(props.profileId));
const trackBusy = ref(false);
const pendingTrack = computed(() => messages.value.some(message => message.track_request && !message.track_request.incoming && message.track_request.state === 'requested'));
const emit = defineEmits<{ loaded: [friend: FriendProfile]; openSoulseekSettings: [] }>();
const chat = ref<FriendChat | null>(null);
const loading = ref(true);
const error = ref('');
const messageError = ref('');
const selectedUser = ref('');
const accountMenu = ref(false);
const accountTrigger = ref<HTMLButtonElement | null>(null);
const messages = ref<FriendMessage[]>([]);
const history = ref<HTMLElement | null>(null);
const composer = ref<HTMLTextAreaElement | null>(null);
const drafts = ref<Record<string, string>>({});
const sending = ref(false);
const refreshing = ref(false);
const olderLoading = ref(false);
const messagesLoading = ref(false);
const hasOlder = ref(false);
const hasNew = ref(false);
let loadedOlder = false;
let timer: ReturnType<typeof setTimeout> | undefined;
let generation = 0;
let contextRequest = 0;
let disposed = false;
const draft = computed({ get: () => drafts.value[selectedUser.value] ?? '', set: value => { drafts.value[selectedUser.value] = value; } });
const tooLong = computed(() => new TextEncoder().encode(draft.value).length > 4096);
const canSend = computed(() => !!selectedUser.value && !!chat.value?.enabled && !!chat.value?.configured && !!draft.value.trim() && !tooLong.value && !sending.value);
const nearBottom = () => !history.value || history.value.scrollHeight - history.value.scrollTop - history.value.clientHeight < 80;

async function scrollBottom() {
  await nextTick();
  if (history.value) history.value.scrollTop = history.value.scrollHeight;
  hasNew.value = false;
}
function mergeMessages(incoming: FriendMessage[]) {
  const byId = new Map(messages.value.map(message => [message.id, message]));
  incoming.forEach(message => byId.set(message.id, message));
  messages.value = [...byId.values()].sort((a, b) => a.id - b.id);
}
async function fetchMessages(current: number) {
  if (!chat.value || !selectedUser.value) return;
  const atBottom = nearBottom();
  const previousLast = messages.value[messages.value.length - 1]?.id ?? 0;
  try {
    const page = await invoke<FriendMessagePage>('get_friend_messages', { profileId: props.profileId, account: chat.value.account, username: selectedUser.value });
    if (disposed || current !== generation) return;
    mergeMessages(page.messages);
    if (!loadedOlder) hasOlder.value = page.has_more;
    if (atBottom) await scrollBottom();
    else if ((page.messages[page.messages.length - 1]?.id ?? 0) > previousLast) hasNew.value = true;
  } catch (cause) {
    if (!disposed && current === generation) messageError.value = `Could not load messages. ${String(cause)}`;
  } finally { if (current === generation) messagesLoading.value = false; }
}
async function poll(current: number) {
  await fetchMessages(current);
  if (!disposed && current === generation) timer = setTimeout(() => { void poll(current); }, 2000);
}
function chooseUser(username: string) {
  clearTimeout(timer);
  const current = ++generation;
  accountMenu.value = false;
  selectedUser.value = username;
  messages.value = [];
  messageError.value = '';
  loadedOlder = false;
  hasOlder.value = false;
  hasNew.value = false;
  olderLoading.value = false;
  if (username) { messagesLoading.value = true; void poll(current); }
}
async function loadContext() {
  const request = ++contextRequest;
  loading.value = true;
  error.value = '';
  try {
    const context = await invoke<FriendChat>('get_friend_chat', { profileId: props.profileId });
    if (disposed || request !== contextRequest) return;
    chat.value = context;
    emit('loaded', context.friend);
    const username = context.usernames.includes(selectedUser.value) ? selectedUser.value
      : context.usernames.length === 1 ? context.usernames[0] : '';
    chooseUser(username);
  } catch (cause) { if (!disposed && request === contextRequest) error.value = String(cause); }
  finally { if (request === contextRequest) loading.value = false; }
}
async function findAccounts() {
  if (refreshing.value || sending.value) return;
  refreshing.value = true;
  messageError.value = '';
  try {
    await invoke('resolve_friend', { profileId: props.profileId });
    if (!disposed) await loadContext();
  } catch (cause) { if (!disposed) messageError.value = `Soulseek lookup is unavailable. ${String(cause)}`; }
  finally { refreshing.value = false; }
}
async function loadOlder() {
  if (!chat.value || !messages.value.length || olderLoading.value) return;
  const current = generation;
  const height = history.value?.scrollHeight ?? 0;
  const top = history.value?.scrollTop ?? 0;
  olderLoading.value = true;
  try {
    const page = await invoke<FriendMessagePage>('get_friend_messages', { profileId: props.profileId, account: chat.value.account, username: selectedUser.value, beforeId: messages.value[0].id });
    if (disposed || current !== generation) return;
    mergeMessages(page.messages);
    loadedOlder = true;
    hasOlder.value = page.has_more;
    await nextTick();
    if (history.value) history.value.scrollTop = top + history.value.scrollHeight - height;
  } catch { if (current === generation) messageError.value = 'Could not load older messages. Please try again.'; }
  finally { if (current === generation) olderLoading.value = false; }
}
async function send() {
  if (!canSend.value || !chat.value) return;
  const username = selectedUser.value;
  const body = draft.value;
  const current = generation;
  sending.value = true;
  messageError.value = '';
  try {
    await invoke('send_friend_message', { profileId: props.profileId, account: chat.value.account, username, body });
    if (disposed || current !== generation) return;
    if (drafts.value[username] === body) drafts.value[username] = '';
    await fetchMessages(current);
    await scrollBottom();
    composer.value?.focus();
  } catch (cause) {
    if (!disposed && current === generation) {
      messageError.value = `Could not confirm sending. ${String(cause)}`;
      await fetchMessages(current);
    }
  } finally { sending.value = false; }
}
async function requestTrack() {
  if (!chat.value || !selectedUser.value || trackBusy.value) return;
  const current = generation;
  trackBusy.value = true; messageError.value = '';
  try {
    await invoke('request_friend_track', { profileId: props.profileId, account: chat.value.account, username: selectedUser.value });
    if (!disposed && current === generation) { await fetchMessages(current); await scrollBottom(); }
  } catch (cause) { if (!disposed && current === generation) messageError.value = String(cause); }
  finally { trackBusy.value = false; }
}
async function respondTrack(id: string, accept: boolean) {
  if (!chat.value || trackBusy.value) return;
  const current = generation;
  trackBusy.value = true; messageError.value = '';
  try {
    await invoke('respond_friend_track', { profileId: props.profileId, account: chat.value.account, username: selectedUser.value, requestId: id, accept });
  } catch (cause) { if (!disposed && current === generation) messageError.value = String(cause); }
  finally { trackBusy.value = false; if (!disposed && current === generation) await fetchMessages(current); }
}
const requestStateLabel = (state: string) => ({ requested: 'Waiting for approval', approving: 'Preparing track...', shared: 'Shared for this request', declined: 'Declined', ready: 'Starting download...', downloading: 'Downloading...', completed: 'Added to your library', failed: 'Request failed', expired: 'Request expired' }[state] || state);
function composerKeydown(event: KeyboardEvent) {
  if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) { event.preventDefault(); void send(); }
}
const timeLabel = (timestamp: number) => new Intl.DateTimeFormat('en', { hour: '2-digit', minute: '2-digit' }).format(new Date(timestamp * 1000));
const dateLabel = (timestamp: number) => new Intl.DateTimeFormat('en', { day: 'numeric', month: 'short', year: 'numeric' }).format(new Date(timestamp * 1000));
const stateLabel = (message: FriendMessage) => message.state === 'sending' ? 'Sending...' : message.state === 'unknown' ? 'Delivery unconfirmed' : message.state === 'failed' ? 'Not sent' : '';
function closeAccountMenu(event: Event) {
  if (!accountMenu.value) return;
  if (event instanceof KeyboardEvent) {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    accountMenu.value = false;
    accountTrigger.value?.focus();
  } else if (event.target instanceof Element && !event.target.closest('.chat-account-popup, .chat-account-trigger')) {
    accountMenu.value = false;
  }
}
onMounted(() => {
  void loadContext();
  document.addEventListener('click', closeAccountMenu);
  document.addEventListener('keydown', closeAccountMenu);
});
onUnmounted(() => {
  disposed = true; generation++; contextRequest++; clearTimeout(timer);
  document.removeEventListener('click', closeAccountMenu);
  document.removeEventListener('keydown', closeAccountMenu);
});
</script>

<template>
  <section class="friend-chat">
    <div v-if="loading && !chat" class="chat-empty" role="status">Loading conversation...</div>
    <div v-else-if="error" class="chat-empty"><p role="alert">{{ error }}</p><button class="chat-action" @click="loadContext">Try again</button></div>
    <template v-else-if="chat">
      <div class="chat-toolbar">
        <div class="chat-avatar" aria-hidden="true"><img v-if="discordFriend?.avatar" :src="discordFriend.avatar" alt="" referrerpolicy="no-referrer" /><template v-else>{{ [...chat.friend.name][0]?.toUpperCase() || '' }}<AppIcon v-if="!chat.friend.name" name="friends" :size="24" /></template></div>
        <div class="chat-identity">
          <span class="chat-service">Soulseek PM</span>
          <span class="chat-profile-id">{{ chat.friend.profile_id }}</span>
          <span v-if="discordFriend" class="chat-discord-name">Discord: {{ discordFriend.name }}</span>
        </div>
        <ResponsivePopup :open="accountMenu" panel-class="chat-account-popup" @close="accountMenu = false">
          <template #trigger>
            <button v-if="chat.usernames.length" ref="accountTrigger" class="chat-action chat-account chat-account-trigger" :aria-expanded="accountMenu" :disabled="sending" @click="accountMenu = !accountMenu">
              {{ selectedUser || 'Choose account' }} <AppIcon name="dropdown" :size="16" />
            </button>
          </template>
          <div class="chat-account-list" aria-label="Soulseek accounts">
            <button v-for="username in chat.usernames" :key="username" @click="chooseUser(username)">{{ username }}<AppIcon v-if="selectedUser === username" name="selected" :size="16" /></button>
          </div>
        </ResponsivePopup>
        <button class="chat-icon-button" title="Find Soulseek accounts" aria-label="Find Soulseek accounts" :disabled="refreshing || sending" @click="findAccounts"><AppIcon name="refresh" :size="18" /></button>
      </div>
      <div v-if="discordFriend?.track" class="chat-listening">
        <AppIcon name="playback" :size="22" /><div><span class="chat-listening-label">Listening on Player</span><strong>{{ discordFriend.track.title }}</strong><span>{{ discordFriend.track.artist }}<template v-if="discordFriend.track.album"><br />{{ discordFriend.track.album }}</template></span></div>
        <button class="chat-action" :disabled="trackBusy || pendingTrack || !selectedUser || !chat.enabled || !chat.configured" @click="requestTrack">{{ pendingTrack ? 'Requested' : 'Request track' }}</button>
      </div>
      <p v-if="!chat.enabled || !chat.configured" class="chat-notice">Enable and configure Soulseek to send and receive messages. <a href="#" @click.prevent="emit('openSoulseekSettings')">Open settings</a></p>
      <p v-else-if="selectedUser" class="chat-account-note">Chatting with {{ selectedUser }} as {{ chat.account }}. This Soulseek account is unverified.</p>

      <div v-if="!chat.usernames.length" class="chat-empty">
        <AppIcon name="chat" :size="40" />
        <h2>No Soulseek account linked yet</h2>
        <p>This friend is saved. A Soulseek username is needed to start a conversation.</p>
        <button class="chat-action" :disabled="refreshing || !chat.enabled || !chat.configured" @click="findAccounts">{{ refreshing ? 'Searching...' : 'Find Soulseek account' }}</button>
      </div>
      <div v-else-if="!selectedUser" class="chat-empty"><AppIcon name="chat" :size="40" /><h2>Choose an account</h2><p>Select the Soulseek account you want to message.</p></div>
      <template v-else>
        <div ref="history" class="chat-history" aria-label="Conversation" tabindex="0" @scroll.passive="nearBottom() && (hasNew = false)">
          <div v-if="hasOlder" class="chat-older"><button class="chat-action" :disabled="olderLoading" @click="loadOlder">{{ olderLoading ? 'Loading...' : 'Load older messages' }}</button></div>
          <div v-if="messagesLoading && !messages.length" class="chat-empty" role="status">Loading messages...</div>
          <div v-else-if="!messages.length" class="chat-empty"><AppIcon name="chat" :size="36" /><h2>Start a conversation</h2><p>Your messages will be sent through Soulseek.</p></div>
          <template v-for="(message, index) in messages" :key="message.id">
            <div v-if="index === 0 || dateLabel(message.timestamp) !== dateLabel(messages[index - 1].timestamp)" class="chat-date">{{ dateLabel(message.timestamp) }}</div>
            <article class="chat-message" :class="{ outgoing: message.direction === 'outgoing', unconfirmed: message.state === 'unknown' }" :aria-label="message.direction === 'outgoing' ? 'Message from you' : `Message from ${selectedUser}`">
              <template v-if="message.track_request">
                <span class="chat-listening-label">{{ message.track_request.incoming ? 'Track request' : 'You requested a track' }}</span>
                <p><strong>{{ message.track_request.title }}</strong><br />{{ message.track_request.artist }}</p>
                <p v-if="message.track_request.incoming && message.track_request.state === 'requested'" class="chat-share-note">Accept to share this audio file with {{ selectedUser }} through Soulseek. The share expires after 10 minutes.</p>
                <div v-if="message.track_request.incoming && message.track_request.state === 'requested'" class="chat-request-actions">
                  <button class="chat-action" :disabled="trackBusy || !chat.enabled || !chat.configured" @click="respondTrack(message.track_request.id, true)">Accept and share</button>
                  <button class="chat-action" :disabled="trackBusy || !chat.enabled || !chat.configured" @click="respondTrack(message.track_request.id, false)">Decline</button>
                </div>
                <p v-else class="chat-share-note">{{ requestStateLabel(message.track_request.state) }}</p>
                <p v-if="message.track_request.error" class="chat-error">{{ message.track_request.error }}</p>
              </template>
              <p v-else>{{ message.body }}</p>
              <div class="chat-message-meta"><time :datetime="new Date(message.timestamp * 1000).toISOString()">{{ timeLabel(message.timestamp) }}</time><span v-if="stateLabel(message)">{{ stateLabel(message) }}</span></div>
            </article>
          </template>
        </div>
        <button v-if="hasNew" class="chat-new" @click="scrollBottom">New messages <AppIcon name="dropdown" :size="14" /></button>
        <form class="chat-composer" @submit.prevent="send">
          <textarea ref="composer" v-model="draft" rows="2" maxlength="4096" :placeholder="`Message ${selectedUser}`" aria-label="Message" :disabled="!chat.enabled || !chat.configured" @keydown="composerKeydown" />
          <button class="chat-send" type="submit" :disabled="!canSend" aria-label="Send message" title="Send message"><AppIcon name="send" :size="20" /></button>
          <p class="chat-compose-hint">{{ tooLong ? 'Message is too long.' : 'Enter to send, Shift+Enter for a new line' }}</p>
        </form>
      </template>
      <p v-if="messageError" class="chat-error" role="alert">{{ messageError }}</p>
    </template>
  </section>
</template>

<style>
.chat-account-popup { min-width: 220px; max-width: calc(100vw - 24px); }
.chat-account-list button { display: flex; align-items: center; justify-content: space-between; gap: 16px; width: 100%; padding: 12px 16px; background: transparent; border: 0; color: #fff; text-align: left; font: inherit; font-size: 14px; cursor: pointer; overflow-wrap: anywhere; }
.chat-account-list button:hover { background: rgba(255,255,255,.08); }
.chat-account-list button:focus-visible { outline: 2px solid #1db954; outline-offset: -2px; }
</style>
<style scoped>
.chat-avatar img { width: 100%; height: 100%; object-fit: cover; border-radius: inherit; }
.chat-discord-name { color: #a7a7a7; font-size: 12px; }
.chat-listening { display: flex; align-items: center; gap: 14px; padding: 16px 0; border-bottom: 1px solid rgba(255,255,255,.1); color: #8de2a7; }
.chat-listening > div { display: flex; flex-direction: column; flex: 1; min-width: 0; gap: 4px; }
.chat-listening strong { color: #fff; font-size: 14px; overflow-wrap: anywhere; }
.chat-listening > div > span:last-child { color: #a7a7a7; font-size: 13px; }
.chat-listening-label { display: block; color: #a7a7a7; font-size: 11px; margin-bottom: 4px; }
.chat-message p.chat-share-note { color: #a7a7a7; font-size: 12px; line-height: 1.6; margin-top: 8px; }
.chat-request-actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 12px; }
.friend-chat { flex: 1; min-height: 0; display: flex; flex-direction: column; margin: 0; color: #fff; }
.chat-toolbar { display: flex; align-items: center; gap: 14px; padding: 8px 0 16px; border-bottom: 1px solid rgba(255,255,255,.1); }
.chat-avatar { width: 48px; height: 48px; border-radius: 50%; display: flex; align-items: center; justify-content: center; background: rgba(29,185,84,.12); color: #8de2a7; font-size: 20px; font-weight: 600; flex-shrink: 0; }
.chat-identity { min-width: 0; flex: 1; display: flex; flex-direction: column; gap: 6px; }
.chat-service { font-size: 14px; font-weight: 600; }
.chat-profile-id { font-family: monospace; font-size: 12px; color: #a7a7a7; overflow-wrap: anywhere; user-select: text; }
button { font: inherit; cursor: pointer; }
button:disabled { opacity: .45; cursor: default; }
button:focus-visible, a:focus-visible, textarea:focus-visible, .chat-history:focus-visible { outline: 2px solid #1db954; outline-offset: 3px; }
.chat-action { display: inline-flex; align-items: center; justify-content: center; gap: 8px; background: transparent; border: 1px solid #535353; color: #b3b3b3; border-radius: 20px; padding: 7px 14px; font-size: 13px; font-weight: 600; }
.chat-action:hover:not(:disabled) { border-color: #fff; color: #fff; }
.chat-account { max-width: 260px; overflow-wrap: anywhere; }
.chat-icon-button { display: flex; align-items: center; justify-content: center; padding: 8px; border: 0; background: transparent; color: #b3b3b3; border-radius: 50%; }
.chat-icon-button:hover:not(:disabled) { color: #fff; background: rgba(255,255,255,.06); }
.chat-notice, .chat-account-note { color: #a7a7a7; font-size: 12px; line-height: 1.6; margin: 12px 0 0; }
a { color: #b3b3b3; text-underline-offset: 3px; }
a:hover { color: #fff; }
.chat-history { flex: 1; min-height: 0; overflow-y: auto; scrollbar-gutter: stable; scrollbar-width: thin; scrollbar-color: #555 transparent; padding: 20px 8px 16px 0; }
.chat-history::-webkit-scrollbar { width: 8px; }
.chat-history::-webkit-scrollbar-thumb { background: #555; border-radius: 4px; }
.chat-history::-webkit-scrollbar-track { background: transparent; }
.chat-empty { flex: 1; display: flex; min-height: 180px; align-items: center; justify-content: center; flex-direction: column; gap: 12px; text-align: center; color: #a7a7a7; padding: 24px 12px; }
.chat-empty h2 { color: #fff; font-size: 20px; margin: 4px 0 0; }
.chat-empty p { font-size: 14px; line-height: 1.6; margin: 0; }
.chat-date { text-align: center; color: #888; font-size: 12px; margin: 12px 0 20px; }
.chat-message { width: fit-content; max-width: min(72%, 640px); padding: 12px 16px 8px; margin-bottom: 12px; background: #282828; border-radius: 12px 12px 12px 4px; }
.chat-message.outgoing { margin-left: auto; background: #1b3528; border-radius: 12px 12px 4px 12px; }
.chat-message p { white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; margin: 0; font-size: 14px; line-height: 1.6; }
.chat-message-meta { display: flex; align-items: center; justify-content: flex-end; flex-wrap: wrap; gap: 10px; margin-top: 6px; font-size: 11px; color: #a7a7a7; }
.chat-message.unconfirmed .chat-message-meta { color: #d7b882; }
.chat-older { display: flex; justify-content: center; margin-bottom: 20px; }
.chat-composer { display: grid; grid-template-columns: minmax(0,1fr) auto; gap: 8px 12px; padding-top: 16px; border-top: 1px solid rgba(255,255,255,.1); }
.chat-composer textarea { width: 100%; box-sizing: border-box; min-height: 64px; max-height: 180px; resize: vertical; border: 1px solid #444; border-radius: 8px; padding: 12px 14px; background: rgba(255,255,255,.04); color: #fff; font: inherit; font-size: 14px; line-height: 1.5; }
.chat-composer textarea::placeholder { color: #888; }
.chat-send { display: flex; align-items: center; justify-content: center; align-self: center; width: 42px; height: 42px; border: 0; border-radius: 50%; background: #1db954; color: #000; }
.chat-send:hover:not(:disabled) { background: #1ed760; }
.chat-compose-hint { grid-column: 1 / -1; margin: 0; color: #888; font-size: 11px; }
.chat-error { color: #ff9a9a; margin: 10px 0 0; font-size: 13px; line-height: 1.5; overflow-wrap: anywhere; }
.chat-new { display: flex; align-items: center; gap: 6px; align-self: center; border: 1px solid #535353; background: #282828; color: #fff; border-radius: 20px; font-size: 12px; padding: 8px 14px; margin: 0 0 8px; }
@media(max-width: 600px) {
  .chat-toolbar { gap: 10px; flex-wrap: wrap; }
  .chat-avatar { width: 38px; height: 38px; }
  .chat-profile-id { font-size: 10px; }
  .chat-account { max-width: min(220px, 65vw); }
  .chat-message { max-width: 86%; padding: 10px 12px 8px; }
  .chat-compose-hint { display: none; }
  .chat-account-note { margin-top: 8px; font-size: 11px; }
}
</style>
