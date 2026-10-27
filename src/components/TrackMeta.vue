<script setup lang="ts">
import { computed } from "vue";
import type { Track } from "../types";
import { artistLabel, hasArtistPage } from "../utils/artists";
import ArtistLink from './ArtistLink.vue';

const props = withDefaults(defineProps<{
  track: Track;
  showArtist?: boolean;
  showAlbum?: boolean;
  showYear?: boolean;
  showGenre?: boolean;
}>(), {
  showArtist: true,
  showAlbum: false,
  showYear: false,
  showGenre: false,
});

const emit = defineEmits<{
  openArtist: [name: string];
  openAlbum: [name: string, albumArtistOrArtist: string];
}>();

interface Part { text: string; link?: 'artist' | 'album' }

const parts = computed(() => {
  const t = props.track;
  const result: Part[] = [];
  if (props.showArtist) result.push({ text: artistLabel(t.artist), link: hasArtistPage(t.artist) ? 'artist' : undefined });
  if (props.showAlbum && t.album) result.push({ text: t.album, link: 'album' });
  if (props.showYear && t.year) result.push({ text: String(t.year) });
  if (props.showGenre && t.genre) result.push({ text: t.genre });
  return result;
});

function open(part: Part) {
  if (part.link === 'artist') emit('openArtist', part.text);
  else if (part.link === 'album') emit('openAlbum', part.text, props.track.album_artist_or_artist ?? '');
}
</script>

<template>
  <span class="track-album">
    <template v-for="(part, i) in parts" :key="i">
      <template v-if="i > 0"> · </template>
      <ArtistLink v-if="part.link === 'artist'" :artist="part.text" @open="emit('openArtist', $event)" />
      <a
        v-else-if="part.link"
        class="meta-link"
        href="#"
        @click.prevent.stop="open(part)"
      >{{ part.text }}</a>
      <template v-else>{{ part.text }}</template>
    </template>
  </span>
</template>

<style scoped>
.meta-link {
  color: inherit;
  text-decoration: none;
}
.meta-link:hover {
  color: var(--color-text);
  text-decoration: underline;
}
</style>
