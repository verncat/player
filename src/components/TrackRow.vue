<script setup lang="ts">
import type { Track } from "../types";
import { formatDuration } from "../utils/format";
import { hashToColors, rarityClass, rarityVars } from "../utils/rarity";
import { coverUrl } from "../utils/covers";
import { useTrackActions, type TrackButton } from "../composables/useTrackActions";
import CoverImage from "./CoverImage.vue";
import TrackMeta from "./TrackMeta.vue";

// Everything except "play" goes through the injected TrackActions, because
// only the list knows which queue a click should play.
const props = withDefaults(defineProps<{
  track: Track;
  showNumber?: boolean;
  showCover?: boolean;
  showArtist?: boolean;
  showAlbum?: boolean;
  showYear?: boolean;
  showGenre?: boolean;
  playlistId?: number | null;
  hide?: TrackButton[];
}>(), {
  showNumber: false,
  showCover: true,
  showArtist: true,
  showAlbum: true,
  showYear: false,
  showGenre: false,
  playlistId: null,
  hide: () => [],
});

const shows = (button: TrackButton) => !props.hide.includes(button);

const emit = defineEmits<{
  play: [];
}>();

const actions = useTrackActions();


const LONG_PRESS_MS = 420;
const MOVE_TOLERANCE_PX = 12;
let pressTimer: ReturnType<typeof setTimeout> | null = null;
let pressX = 0;
let pressY = 0;
let suppressClick = false;

function onTouchStart(e: TouchEvent) {
  if (e.touches.length !== 1) return;
  cancelPress();
  pressX = e.touches[0].clientX;
  pressY = e.touches[0].clientY;
  pressTimer = setTimeout(() => {
    pressTimer = null;
    suppressClick = true; // the finger lifting off would otherwise count as a click
    actions.openMenu(props.track, pressX, pressY, props.playlistId);
  }, LONG_PRESS_MS);
}
function onTouchMove(e: TouchEvent) {
  if (!pressTimer || !e.touches.length) return;
  const dx = e.touches[0].clientX - pressX;
  const dy = e.touches[0].clientY - pressY;
  if (Math.hypot(dx, dy) > MOVE_TOLERANCE_PX) cancelPress();
}
function cancelPress() {
  if (pressTimer) clearTimeout(pressTimer);
  pressTimer = null;
}
function onClick() {
  if (suppressClick) {
    suppressClick = false;
    return;
  }
  emit('play');
}
</script>

<template>
  <div
    class="track-row"
    :class="[rarityClass(track.rarity), {
      'track-row-current': actions.isCurrent(track.id),
      'track-row-next': actions.isNext(track.id),
    }]"
    :style="rarityVars(track.rarity)"
    @click="onClick"
    @contextmenu.prevent="actions.openMenu(track, $event.clientX, $event.clientY, playlistId)"
    @touchstart.passive="onTouchStart"
    @touchmove.passive="onTouchMove"
    @touchend="cancelPress"
    @touchcancel="cancelPress"
  >
    <CoverImage
      v-if="showCover"
      class="track-cover-sm"
      :src="coverUrl(track.cover_hash, 'small')"
      :placeholder="hashToColors(track.file_hash)"
    />
    <span v-if="showNumber" class="track-num">{{ track.track_number ?? '–' }}</span>
    <div class="track-info">
      <div class="track-title-row">
        <span class="track-title">{{ track.title || track.path }}</span>
        <span v-if="actions.isCurrent(track.id)" class="track-playback-badge current" title="Now playing">
          <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M8 5v14l11-7z"/></svg>
        </span>
        <span v-else-if="actions.isNext(track.id)" class="track-playback-badge next" title="Up next">
          <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M8 5v14l11-7z"/></svg>
          <span class="track-playback-step">1</span>
        </span>
      </div>
      <TrackMeta
        :track="track"
        :show-artist="showArtist"
        :show-album="showAlbum"
        :show-year="showYear"
        :show-genre="showGenre"
        @open-artist="actions.openArtist"
        @open-album="actions.openAlbum"
      />
    </div>

    <slot name="extra" />

    <button v-if="shows('identify')" class="icon-btn hover-action inline-action" title="Identify" @click.stop="actions.identify(track)">
      <svg viewBox="0 0 24 24" fill="currentColor" width="14" height="14"><path d="m22 2-2.5 1.4L17.1 2l1.4 2.5L17.1 7l2.4-1.4L22 7l-1.4-2.5zm-7.63 5.29a.996.996 0 0 0-1.41 0L1.29 18.96a.996.996 0 0 0 0 1.41l2.34 2.34c.39.39 1.02.39 1.41 0L16.7 11.05a.996.996 0 0 0 0-1.41l-2.33-2.35zM5.21 19.38l-1.59-1.59 8.93-8.93 1.59 1.59-8.93 8.93z"/></svg>
    </button>
    <button v-if="shows('edit')" class="icon-btn hover-action inline-action" title="Edit" @click.stop="actions.edit(track)">
      <svg viewBox="0 0 24 24" fill="currentColor" width="14" height="14"><path d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04a1.003 1.003 0 0 0 0-1.42l-2.34-2.33a1.003 1.003 0 0 0-1.42 0l-1.83 1.83 3.75 3.75 1.84-1.83z"/></svg>
    </button>
    <span v-if="shows('playCount')" class="play-count" :title="track.play_count > 0 ? 'Play count' : undefined">
      <template v-if="track.play_count > 0">▶ {{ track.play_count }}</template>
    </span>
    <button
      v-if="shows('like')"
      class="icon-btn inline-action"
      :title="track.is_liked ? 'Unlike' : 'Like'"
      @click.stop="actions.toggleLike(track)"
    >
      <svg v-if="track.is_liked" viewBox="0 0 24 24" fill="#1db954" width="16" height="16"><path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"/></svg>
      <svg v-else viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M16.5 3c-1.74 0-3.41.81-4.5 2.09C10.91 3.81 9.24 3 7.5 3 4.42 3 2 5.42 2 8.5c0 3.78 3.4 6.86 8.55 11.54L12 21.35l1.45-1.32C18.6 15.36 22 12.28 22 8.5 22 5.42 19.58 3 16.5 3zm-4.4 15.55l-.1.1-.1-.1C7.14 14.24 4 11.39 4 8.5 4 6.5 5.5 5 7.5 5c1.54 0 3.04.99 3.57 2.36h1.87C13.46 5.99 14.96 5 16.5 5c2 0 3.5 1.5 3.5 3.5 0 2.89-3.14 5.74-7.9 10.05z"/></svg>
    </button>
    <button
      v-if="shows('addToPlaylist')"
      class="icon-btn hover-action inline-action"
      title="Add to playlist"
      @click.stop="actions.openAddToPlaylist(track, $event.clientX, $event.clientY)"
    >
      <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M14 10H3v2h11v-2zm0-4H3v2h11V6zM3 16h7v-2H3v2zm11.41-2.83L13 14.59 11.59 13 10 14.59l3 3.01 4-4.01-1.59-1.42z"/></svg>
    </button>
    <span class="track-dur">{{ formatDuration(track.duration_secs) }}</span>
  </div>
</template>

<style scoped>
.track-row {
  display: flex; align-items: center; gap: 12px;
  padding: 8px 8px; border-radius: 4px;
  cursor: pointer; user-select: none;
}
.track-row:hover { background: #282828 !important; }
.track-row.rarity-tint { background: color-mix(in srgb, var(--rc) 12%, #181818); }
.track-row.track-row-current {
  background: rgba(29, 185, 84, 0.14) !important;
  box-shadow: inset 0 0 0 1px rgba(29, 185, 84, 0.3);
}
.track-row.track-row-next {
  background: rgba(29, 185, 84, 0.08) !important;
}
.track-cover-sm {
  width: 36px; height: 36px; border-radius: 3px; flex-shrink: 0;
}
.track-num {
  width: 24px; text-align: right;
  font-size: var(--fs-track-side); color: #a7a7a7; flex-shrink: 0;
}
.track-info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.track-title-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.track-title {
  font-size: var(--fs-track-title); font-weight: 500; color: #fff;
  display: block; flex: 1; min-width: 0;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.track-album {
  font-size: var(--fs-track-meta); color: #a7a7a7;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.track-playback-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 7px;
  border-radius: 999px;
  font-size: var(--fs-badge);
  font-weight: 700;
  line-height: 1;
  flex-shrink: 0;
}
.track-playback-badge.current {
  background: #1db954;
  color: #08120b;
}
.track-playback-badge.next {
  background: rgba(29, 185, 84, 0.14);
  color: #8de2a7;
  box-shadow: inset 0 0 0 1px rgba(29, 185, 84, 0.28);
}
.track-playback-step {
  font-variant-numeric: tabular-nums;
}
.track-dur {
  font-size: var(--fs-track-side); color: #a7a7a7; flex-shrink: 0;
}

.icon-btn {
  background: none; border: none;
  color: #b3b3b3; cursor: pointer; padding: 4px;
  display: flex; align-items: center; justify-content: center;
  border-radius: 4px;
  flex-shrink: 0;
  transition: color .1s, opacity .12s;
}
.icon-btn:hover { color: #fff; }
.hover-action { opacity: 0; }
.track-row:hover .hover-action,
.hover-action:focus-visible { opacity: 1; }
.play-count {
  min-width: 36px;
  text-align: right;
  font-size: var(--fs-track-side);
  color: #a7a7a7;
  flex-shrink: 0;
}

@media (max-width: 768px) {
  .track-row { gap: 8px; padding: 6px 4px; }
  .track-cover-sm { width: 32px; height: 32px; }
  .track-num { display: none; }
  /* No hover on touch screens: actions are in the long-press menu. */
  .inline-action, .play-count { display: none; }
}
@media (max-width: 480px) {
  .track-album, .track-dur { display: none; }
}
</style>
