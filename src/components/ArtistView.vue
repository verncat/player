<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { AlbumSummary, Track } from "../types";
import { fetchArtistAlbums, fetchArtistTracks } from "../api";
import { isVariousArtists } from "../utils/artists";
import { useTrackActions } from "../composables/useTrackActions";
import PageHeader from "./PageHeader.vue";
import TrackRow from "./TrackRow.vue";
import AlbumCard from "./AlbumCard.vue";

const props = defineProps<{
  name: string;
  includeDuplicates: boolean;
  libraryVersion: number;
}>();

const emit = defineEmits<{
  play: [tracks: Track[], index: number];
}>();

const actions = useTrackActions();

const TOP_TRACK_COUNT = 5;
const COLLAPSED_TRACK_COUNT = 15;

const tracks = ref<Track[]>([]);
const albums = ref<AlbumSummary[]>([]);
const loading = ref(false);
const error = ref('');
const expandedSections = ref(new Set<string>());
const showAllTracks = ref(false);

let loadId = 0;
async function load() {
  const id = ++loadId;
  loading.value = true;
  error.value = '';
  try {
    const [albumList, trackList] = await Promise.all([
      fetchArtistAlbums(props.name, props.includeDuplicates),
      // Various Artists doesn't perform tracks; its page is just albums.
      isVariousArtists(props.name) ? [] : fetchArtistTracks(props.name, props.includeDuplicates),
    ]);
    if (id !== loadId) return;
    albums.value = albumList;
    tracks.value = inAlbumOrder(trackList, albumList);
  } catch (e) {
    if (id === loadId) error.value = String(e);
  } finally {
    if (id === loadId) loading.value = false;
  }
}
watch(() => [props.name, props.includeDuplicates, props.libraryVersion], load, { immediate: true });

// Orders tracks like the album cards; tracks not on one of `albumList` go
// last. Array sort is stable, so each album keeps its disc/track order.
function inAlbumOrder(trackList: Track[], albumList: AlbumSummary[]): Track[] {
  const key = (album: string | null, artist: string | null) => `${album ?? ''}\u0000${artist ?? ''}`;
  const albumIndex = new Map(albumList.map((a, i) => [key(a.album, a.album_artist_or_artist), i]));
  const position = (t: Track) => albumIndex.get(key(t.album, t.album_artist_or_artist)) ?? albumList.length;
  return [...trackList].sort((a, b) => position(a) - position(b));
}

watch(() => props.name, () => {
  expandedSections.value = new Set();
  showAllTracks.value = false;
});

const topTracks = computed(() =>
  tracks.value
    .filter((t) => t.is_liked || t.play_count > 0)
    .sort((a, b) => Number(b.is_liked) - Number(a.is_liked) || b.play_count - a.play_count)
    .slice(0, TOP_TRACK_COUNT));

const albumSections = computed(() => [
  { title: 'Albums', albums: albums.value.filter((a) => a.album_artist_or_artist === props.name), showArtist: false },
  { title: 'Appears on', albums: albums.value.filter((a) => a.album_artist_or_artist !== props.name), showArtist: true },
].filter((section) => section.albums.length));
const ownAlbumCount = computed(() => albumSections.value.find((s) => s.title === 'Albums')?.albums.length ?? 0);

const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? '' : 's'}`;
const details = computed(() => [
  ownAlbumCount.value ? plural(ownAlbumCount.value, 'album') : '',
  tracks.value.length ? plural(tracks.value.length, 'track') : '',
].filter(Boolean).join(' · '));
const isEmpty = computed(() => !tracks.value.length && !albums.value.length);

function toggleSection(title: string) {
  const next = new Set(expandedSections.value);
  if (!next.delete(title)) next.add(title);
  expandedSections.value = next;
}

const visibleTracks = computed(() =>
  showAllTracks.value ? tracks.value : tracks.value.slice(0, COLLAPSED_TRACK_COUNT));
</script>

<template>
  <section class="artist-view">
    <PageHeader kind="Artist" :title="name">
      <template #details>{{ details }}</template>
      <template #actions>
        <button v-if="tracks.length" class="play-button" @click="emit('play', tracks, 0)">Play</button>
      </template>
    </PageHeader>

    <div v-if="loading && isEmpty" class="status">Loading...</div>
    <div v-else-if="error" class="status">{{ error }}</div>
    <div v-else-if="isEmpty" class="status">No tracks found for this artist.</div>

    <template v-else>
      <section v-if="topTracks.length" class="artist-section">
        <h2 class="section-title">Top tracks</h2>
        <TrackRow
          v-for="track in topTracks"
          :key="track.id"
          :track="track"
          :hide="['identify']"
          :show-artist="track.artist !== name"
          @play="emit('play', topTracks, topTracks.indexOf(track))"
        />
      </section>

      <section v-for="section in albumSections" :key="section.title" class="artist-section">
        <div class="section-head">
          <h2 class="section-title">{{ section.title }}</h2>
          <button v-if="section.albums.length > 1" class="show-all" @click="toggleSection(section.title)">
            {{ expandedSections.has(section.title) ? 'Show less' : 'Show all' }}
          </button>
        </div>
        <div
          class="album-list"
          :class="{ expanded: expandedSections.has(section.title) }"
          v-horizontal-scroll
        >
          <AlbumCard
            v-for="album in section.albums"
            :key="`${album.album}\u0000${album.album_artist_or_artist}`"
            :album="album"
            :show-artist="section.showArtist"
            @open="actions.openAlbum(album.album, album.album_artist_or_artist)"
          />
        </div>
      </section>

      <section v-if="tracks.length" class="artist-section">
        <div class="section-head">
          <h2 class="section-title">All tracks</h2>
          <button
            v-if="tracks.length > COLLAPSED_TRACK_COUNT"
            class="show-all"
            @click="showAllTracks = !showAllTracks"
          >
            {{ showAllTracks ? 'Show less' : `Show all ${tracks.length}` }}
          </button>
        </div>
        <TrackRow
          v-for="(track, index) in visibleTracks"
          :key="track.id"
          :track="track"
          :hide="['identify']"
          :show-artist="track.artist !== name"
          @play="emit('play', tracks, index)"
        />
      </section>
    </template>
  </section>
</template>

<style scoped>
.status {
  color: var(--color-text-muted);
  padding: 16px 0;
}
.artist-section {
  margin-bottom: 32px;
}
.section-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 16px;
}
.section-title {
  font-size: 20px;
  font-weight: 700;
  color: var(--color-text);
  margin-bottom: 12px;
}
.show-all {
  background: none;
  border: none;
  color: var(--color-text-muted);
  font-size: 13px;
  font-weight: 700;
  cursor: pointer;
}
.show-all:hover { color: var(--color-text); text-decoration: underline; }

.album-list {
  display: flex;
  gap: 16px;
  overflow-x: auto;
  scrollbar-width: none;
}
.album-list.expanded {
  flex-wrap: wrap;
  overflow-x: visible;
}
</style>
