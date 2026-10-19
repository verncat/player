<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { Track } from "../types";
import { fetchAlbumTracks } from "../api";
import { hashToColors } from "../utils/rarity";
import { artistLabel, hasArtistPage } from "../utils/artists";
import { useCovers } from "../composables/useCovers";
import { useTrackActions } from "../composables/useTrackActions";
import PageHeader from "./PageHeader.vue";
import TrackRow from "./TrackRow.vue";

const props = defineProps<{
  name: string;
  /** With `name`, identifies the album ("" = its tracks have no artist). */
  albumArtistOrArtist: string;
  includeDuplicates: boolean;
  libraryVersion: number;
}>();

const emit = defineEmits<{
  play: [tracks: Track[], index: number];
}>();

const actions = useTrackActions();

const tracks = ref<Track[]>([]);
const loading = ref(false);
const error = ref('');

let loadId = 0;
async function load() {
  const id = ++loadId;
  loading.value = true;
  error.value = '';
  try {
    const result = await fetchAlbumTracks(props.name, props.albumArtistOrArtist, props.includeDuplicates);
    if (id === loadId) tracks.value = result;
  } catch (e) {
    if (id === loadId) error.value = String(e);
  } finally {
    if (id === loadId) loading.value = false;
  }
}
watch(() => [props.name, props.albumArtistOrArtist, props.includeDuplicates, props.libraryVersion], load, { immediate: true });

const year = computed(() => tracks.value.find((t) => t.year)?.year ?? null);

interface Disc { number: number | null; start: number; tracks: Track[] }
const discs = computed(() => {
  const result: Disc[] = [];
  tracks.value.forEach((track, index) => {
    const last = result[result.length - 1];
    if (last && last.number === track.disc_number) last.tracks.push(track);
    else result.push({ number: track.disc_number, start: index, tracks: [track] });
  });
  return result;
});
const totalMinutes = computed(() =>
  Math.round(tracks.value.reduce((sum, t) => sum + (t.duration_secs ?? 0), 0) / 60));

const { covers, fetchCovers } = useCovers();
const coverTrack = computed(() => tracks.value.find((t) => t.has_cover) ?? null);
watch(coverTrack, (track) => { if (track) fetchCovers([track]); }, { immediate: true });
const coverUrl = computed(() => (coverTrack.value ? covers.value[coverTrack.value.id] ?? null : null));
const coverGradient = computed(() => hashToColors(tracks.value[0]?.file_hash ?? null));
</script>

<template>
  <section class="album-view">
    <PageHeader kind="Album" :title="name" :cover-url="coverUrl" :cover-gradient="coverGradient">
      <template #details>
        <a v-if="hasArtistPage(albumArtistOrArtist)" class="artist-link" href="#" @click.prevent="actions.openArtist(albumArtistOrArtist)">{{ albumArtistOrArtist }}</a>
        <template v-else>{{ artistLabel(albumArtistOrArtist) }}</template>
        <template v-if="year"> · {{ year }}</template>
        <template v-if="tracks.length">
          · {{ tracks.length }} track{{ tracks.length === 1 ? '' : 's' }} · {{ totalMinutes }} min
        </template>
      </template>
      <template #actions>
        <button v-if="tracks.length" class="play-button" @click="emit('play', tracks, 0)">Play</button>
      </template>
    </PageHeader>

    <div v-if="loading && !tracks.length" class="status">Loading…</div>
    <div v-else-if="error" class="status">{{ error }}</div>
    <div v-else-if="!tracks.length" class="status">No tracks found for this album.</div>

    <div v-else class="tracklist">
      <template v-for="disc in discs" :key="disc.start">
        <h3 v-if="discs.length > 1" class="disc-heading">
          {{ disc.number !== null ? `Disc ${disc.number}` : 'No disc number' }}
        </h3>
        <TrackRow
          v-for="(track, i) in disc.tracks"
          :key="track.id"
          :track="track"
          :hide="['identify']"
          show-number
          :show-cover="false"
          :show-artist="track.artist !== albumArtistOrArtist"
          :show-album="false"
          @play="emit('play', tracks, disc.start + i)"
        />
      </template>
    </div>
  </section>
</template>

<style scoped>
.artist-link {
  color: var(--color-text, #fff);
  font-weight: 700;
  text-decoration: none;
}
.artist-link:hover { text-decoration: underline; }

.status {
  color: var(--color-text-muted, #a7a7a7);
  padding: 16px 0;
}
.tracklist {
  display: flex;
  flex-direction: column;
}
.disc-heading {
  font-size: 13px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: .04em;
  color: var(--color-text-muted, #a7a7a7);
  padding: 8px 8px;
  border-bottom: 1px solid var(--color-border, #282828);
  margin: 16px 0 4px;
}
.disc-heading:first-child { margin-top: 0; }
</style>
