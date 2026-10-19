<script setup lang="ts">
import type { AlbumSummary } from "../types";
import { coverUrl } from "../utils/covers";
import CoverImage from "./CoverImage.vue";

withDefaults(defineProps<{
  album: AlbumSummary;
  showArtist?: boolean;
}>(), {
  showArtist: false,
});

const emit = defineEmits<{
  open: [];
}>();

</script>

<template>
  <div class="album-card" @click="emit('open')">
    <CoverImage class="album-card-cover" :src="coverUrl(album.cover_hash, 'medium')" />
    <div class="album-card-title" :title="album.album">{{ album.album }}</div>
    <div class="album-card-meta">
      <template v-if="album.year">{{ album.year }} · </template>
      <template v-if="showArtist">{{ album.album_artist_or_artist }}</template>
      <template v-else>{{ album.track_count }} track{{ album.track_count === 1 ? '' : 's' }}</template>
    </div>
  </div>
</template>

<style scoped>
.album-card {
  width: 150px;
  flex-shrink: 0;
  padding: 12px;
  border-radius: var(--radius-card, 6px);
  background: var(--color-card, #181818);
  cursor: pointer;
  transition: background .18s;
}
.album-card:hover { background: var(--color-card-hover, #282828); }
.album-card-cover {
  aspect-ratio: 1;
  border-radius: 4px;
  margin-bottom: 10px;
  background-color: var(--color-surface, #282828);
  background-size: cover;
  background-position: center;
}
.album-card-title {
  font-size: var(--fs-card-title);
  font-weight: 700;
  color: var(--color-text, #fff);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.album-card-meta {
  margin-top: 4px;
  font-size: var(--fs-card-meta);
  color: var(--color-text-muted, #a7a7a7);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
