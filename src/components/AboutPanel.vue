<script setup lang="ts">
import { ref } from 'vue';
import { Marked } from 'marked';
import appIconUrl from '../../src-tauri/icons/128x128.png';
import contributorSnapshot from '../generated/contributors.json';
import type { AboutInfo, AboutUpdateStatus, Contributor } from '../types/settings';
import AppIcon from './AppIcon.vue';

const markdown = new Marked({ gfm: true, breaks: true, renderer: { html: () => '' } });
const contributors: Contributor[] = contributorSnapshot.contributors;
const failedAvatars = ref(new Set<string>());
const assetsBaseUrl = import.meta.env.BASE_URL;

defineProps<{
  info: AboutInfo | null;
  loading: boolean;
  error: string;
  status: AboutUpdateStatus | null;
  checking: boolean;
  updating: boolean;
  progress: number | null;
  desktopReady: boolean;
  updateError: string;
  updateDone: string;
}>();
const emit = defineEmits<{ check: []; update: []; openRelease: []; openRepository: []; retry: []; openExternal: [url: string] }>();
function openNoteLink(event: MouseEvent) {
  const link = (event.target as HTMLElement).closest('a');
  if (!link) return;
  event.preventDefault();
  const url = link.getAttribute('href');
  if (url && /^https?:\/\//i.test(url)) emit('openExternal', url);
}
</script>

<template>
  <div class="about-panel">
    <p v-if="loading && !info" role="status">Loading app information...</p>
    <div v-else-if="error && !info" class="error" role="alert">{{ error }} <button @click="emit('retry')">Try again</button></div>
    <template v-else>
      <div class="app-identity">
        <div class="app-summary">
          <img class="app-mark" :src="appIconUrl" alt="" width="64" height="64" />
          <div><h3>Player</h3><p>Local-first music player with native playback, smart playlists, Soulseek search, and LAN sync.</p></div>
        </div>
        <button v-if="info?.release_repo" class="text-link repository-link" @click="emit('openRepository')">GitHub repository <AppIcon name="open-in-new" /></button>
      </div>
      <div class="info-row"><span>Version</span><strong>{{ info?.current_version || 'Unavailable' }}</strong></div>
      <div class="info-row"><span>Platform</span><span>{{ info?.platform || 'Unavailable' }}</span></div>
      <div class="update-card">
        <div><h3>{{ status?.has_update ? 'An update is available' : 'App updates' }}</h3>
          <p v-if="status?.has_update">Version {{ status.latest_version }} is ready.</p>
          <p v-else-if="status">{{ status.latest_version ? 'You have the latest release.' : status.message }}</p>
          <p v-else>Check for the latest version of Player.</p>
        </div>
        <button v-if="status?.has_update" class="primary" :disabled="updating" @click="emit('update')">
          {{ updating ? (progress !== null ? `Downloading ${progress}%` : 'Downloading...') : desktopReady || status.asset_url ? 'Update now' : 'View release' }}
        </button>
        <button v-else :disabled="checking" @click="emit('check')">{{ checking ? 'Checking...' : 'Check for updates' }}</button>
        <progress v-if="updating && progress !== null" max="100" :value="progress" aria-label="Update download progress" />
      </div>
      <p v-if="updateError" class="error" role="alert">{{ updateError }}</p>
      <p v-if="updateDone" role="status">{{ updateDone }}</p>
      <div v-if="status?.release_url" class="links">
        <button class="text-link" @click="emit('openRelease')">Release page <AppIcon name="open-in-new" /></button>
      </div>
      <div v-if="contributors.length" class="contributors" aria-labelledby="contributors-heading">
        <h3 id="contributors-heading">Contributors</h3>
        <p>The people building Player.</p>
        <ul class="contributors-list">
          <li v-for="contributor in contributors" :key="contributor.login">
            <a class="contributor-link" :href="contributor.profileUrl" target="_blank" rel="noopener noreferrer" @click.prevent="emit('openExternal', contributor.profileUrl)">
              <span class="contributor-avatar" aria-hidden="true">
                <img v-if="contributor.avatarPath && !failedAvatars.has(contributor.login)" :src="`${assetsBaseUrl}${contributor.avatarPath}`" alt="" width="40" height="40" loading="lazy" decoding="async" @error="failedAvatars.add(contributor.login)" />
                <template v-else>{{ contributor.login[0].toUpperCase() }}</template>
              </span>
              <span class="contributor-info"><strong>{{ contributor.login }}</strong><span>GitHub profile</span></span>
              <AppIcon class="contributor-arrow" name="open-in-new" />
            </a>
          </li>
        </ul>
      </div>
      <div v-if="status?.has_update && status.release_notes" class="release-notes">
        <h3>Release notes</h3>
        <div @click="openNoteLink" v-html="markdown.parse(status.release_notes)" />
      </div>
      <div v-else class="changelog">
        <h3>Changelog</h3>
        <ul v-if="info?.changelog.length"><li v-for="entry in info.changelog" :key="entry.short_hash">{{ entry.subject }}</li></ul>
        <p v-else>No changelog is available for this build.</p>
      </div>
    </template>
  </div>
</template>

<style scoped>
.about-panel { display: flex; flex-direction: column; gap: 20px; color: #b3b3b3; font-size: 14px; }
.app-identity { display: flex; flex-wrap: wrap; align-items: center; gap: 16px; margin-bottom: 8px; }
.app-summary { display: flex; align-items: center; gap: 16px; flex: 1 1 280px; min-width: 0; }
.app-mark { display: block; width: 64px; height: 64px; flex-shrink: 0; object-fit: contain; }
.repository-link { margin-left: auto; flex-shrink: 0; white-space: nowrap; }
h3 { color: #fff; font-size: 16px; margin: 0 0 8px; }
.app-identity h3 { font-size: 24px; }
p { line-height: 1.6; margin: 0; }
.info-row { display: flex; justify-content: space-between; gap: 16px; padding-bottom: 16px; border-bottom: 1px solid #282828; }
strong { color: #fff; }
.update-card { display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 20px; padding: 20px; background: #181818; border: 1px solid #282828; border-radius: 8px; }
button { border: 1px solid #505050; background: transparent; color: #fff; padding: 9px 16px; border-radius: 99px; cursor: pointer; font-size: 13px; font-weight: 600; }
button:hover { background: #282828; }
button:disabled { opacity: .5; cursor: default; }
button:focus-visible { outline: 2px solid #1db954; outline-offset: 3px; }
.contributor-link:focus-visible { outline: 2px solid #1db954; outline-offset: 3px; }
.primary { background: #1db954; color: #061d0e; border-color: #1db954; }
.primary:hover { background: #35cd6b; }
.error { color: #ff939b; overflow-wrap: anywhere; }
.links { display: flex; flex-wrap: wrap; gap: 12px; }
.text-link { display: inline-flex; align-items: center; gap: 6px; border: none; padding: 0; color: #b3b3b3; border-radius: 2px; }
.text-link:hover { background: transparent; color: #fff; text-decoration: underline; }
.contributors-list { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(200px, 100%), 1fr)); gap: 12px; list-style: none; padding: 0; margin: 16px 0 0; }
.contributor-link { display: flex; align-items: center; gap: 12px; padding: 16px; border: 1px solid #282828; border-radius: 8px; background: #181818; color: #b3b3b3; text-decoration: none; }
.contributor-link:hover { background: #222; border-color: #505050; color: #fff; }
.contributor-avatar { display: grid; place-items: center; width: 40px; height: 40px; border-radius: 50%; background: #253a2d; color: #81dfa2; font-size: 16px; font-weight: 700; flex-shrink: 0; }
.contributor-avatar img { display: block; width: 100%; height: 100%; border-radius: inherit; object-fit: cover; }
.contributor-info { display: flex; flex-direction: column; gap: 5px; min-width: 0; }
.contributor-info strong { font-size: 14px; }
.contributor-info > span { font-size: 12px; }
.contributor-arrow { margin-left: auto; }
progress { width: 100%; accent-color: #1db954; }
.changelog ul { padding-left: 18px; }
.changelog li { padding: 6px 0; line-height: 1.5; }
.release-notes { line-height: 1.7; overflow-wrap: anywhere; }
.release-notes :deep(a) { color: #6bda94; }
</style>
