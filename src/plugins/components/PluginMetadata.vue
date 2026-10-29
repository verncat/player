<script setup lang="ts">
import { computed, ref } from 'vue';
import { openUrl } from '@tauri-apps/plugin-opener';
import type { PluginManifest } from '../types';
const props = defineProps<{ manifest: PluginManifest }>();
const error = ref('');
const author = computed(() => typeof props.manifest.author === 'string'
  ? { name: props.manifest.author, url: undefined } : props.manifest.author);
function webLink(value?: string | null) {
  try {
    const url = new URL(value || '');
    return ['http:', 'https:'].includes(url.protocol) && !url.username && !url.password ? url.href : undefined;
  } catch { return undefined; }
}
async function open(url: string) {
  try { error.value = ''; await openUrl(url); } catch { error.value = 'Could not open this link.'; }
}
</script>
<template>
  <div class="plugin-metadata">
    <span v-if="author">By <a v-if="webLink(author.url)" :href="webLink(author.url)" @click.prevent="open(webLink(author.url)!)">{{ author.name }}</a><span v-else>{{ author.name }}</span></span>
    <span class="plugin-links">
      <a v-if="webLink(manifest.repository)" :href="webLink(manifest.repository)" @click.prevent="open(webLink(manifest.repository)!)">Repository</a>
      <a v-if="webLink(manifest.homepage) && manifest.homepage !== manifest.repository" :href="webLink(manifest.homepage)" @click.prevent="open(webLink(manifest.homepage)!)">Website</a>
    </span>
    <slot />
    <span v-if="error" class="plugin-link-error" role="alert">{{ error }}</span>
  </div>
</template>
<style scoped>
.plugin-metadata { display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px 12px; color: var(--color-text-muted); font-size: 12px; line-height: 1.5; overflow-wrap: anywhere; }
.plugin-links { display: flex; gap: 12px; flex-wrap: wrap; }.plugin-links:empty { display: none; }
.plugin-metadata a { color: var(--color-text-muted); text-decoration: underline; text-underline-offset: 3px; }.plugin-metadata a:hover { color: var(--color-text); }
.plugin-link-error { flex-basis: 100%; }
</style>
