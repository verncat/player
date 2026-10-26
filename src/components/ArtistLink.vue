<script setup lang="ts">
import { artistLabel, hasArtistPage } from '../utils/artists';

defineProps<{ artist?: string | null }>();
const emit = defineEmits<{ open: [name: string] }>();
</script>

<template>
  <a v-if="hasArtistPage(artist)" class="artist-link" href="#" @click.prevent.stop="emit('open', artist)"><slot>{{ artistLabel(artist) }}</slot></a>
  <span v-else><slot>{{ artistLabel(artist) }}</slot></span>
</template>

<style scoped>
.artist-link { color: inherit; text-decoration: none; }
.artist-link:hover, .artist-link:focus-visible { color: var(--color-text); text-decoration: underline; }
.artist-link:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 2px; border-radius: 2px; }
</style>
