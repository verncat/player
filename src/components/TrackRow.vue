<script setup lang="ts">
import type { Track } from "../types";
import { formatDuration } from "../utils/format";
import { hashToColors, rarityClass, rarityVars } from "../utils/rarity";
import { coverUrl } from "../utils/covers";
import { useTrackActions, type TrackButton } from "../composables/useTrackActions";
import CoverImage from "./CoverImage.vue";
import TrackMeta from "./TrackMeta.vue";
import AppIcon from "./AppIcon.vue";

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
          <AppIcon name="now-playing" :size="12" />
        </span>
        <span v-else-if="actions.isNext(track.id)" class="track-playback-badge next" title="Up next">
          <AppIcon name="up-next" :size="12" />
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
      <AppIcon name="identify" :size="14" />
    </button>
    <button v-if="shows('edit')" class="icon-btn hover-action inline-action" title="Edit" @click.stop="actions.edit(track)">
      <AppIcon name="edit" :size="14" />
    </button>
    <span v-if="shows('playCount')" class="play-count" :title="track.play_count > 0 ? 'Play count' : undefined">
      <template v-if="track.play_count > 0"><AppIcon name="play-count" :size="12" /> {{ track.play_count }}</template>
    </span>
    <button
      v-if="shows('like')"
      class="icon-btn inline-action"
      :title="track.is_liked ? 'Unlike' : 'Like'"
      @click.stop="actions.toggleLike(track)"
    >
      <AppIcon :name="track.is_liked ? 'liked' : 'like'" :size="16" :class="{ liked: track.is_liked }" />
    </button>
    <button
      v-if="shows('addToPlaylist')"
      class="icon-btn hover-action inline-action"
      title="Add to playlist"
      @click.stop="actions.openAddToPlaylist(track, $event.clientX, $event.clientY)"
    >
      <AppIcon name="add-to-playlist" :size="16" />
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
.track-row:hover { background: var(--color-card-hover) !important; }
.track-row.rarity-tint { background: var(--rarity-tint); }
.track-row.track-row-current {
  background: color-mix(in srgb, var(--color-accent) 14%, transparent) !important;
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--color-accent) 30%, transparent);
}
.track-row.track-row-next {
  background: color-mix(in srgb, var(--color-accent) 8%, transparent) !important;
}
.track-cover-sm {
  width: 36px; height: 36px; border-radius: 3px; flex-shrink: 0;
}
.track-num {
  width: 24px; text-align: right;
  font-size: var(--fs-track-side); color: var(--color-text-muted); flex-shrink: 0;
}
.track-info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.track-title-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.track-title {
  font-size: var(--fs-track-title); font-weight: 500; color: var(--color-text);
  display: block; flex: 1; min-width: 0;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.track-album {
  font-size: var(--fs-track-meta); color: var(--color-text-muted);
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
  background: var(--color-accent);
  color: var(--color-on-accent);
}
.track-playback-badge.next {
  background: color-mix(in srgb, var(--color-accent) 14%, transparent);
  color: var(--color-accent-text);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--color-accent) 28%, transparent);
}
.track-playback-step {
  font-variant-numeric: tabular-nums;
}
.track-dur {
  font-size: var(--fs-track-side); color: var(--color-text-muted); flex-shrink: 0;
}

.icon-btn {
  background: none; border: none;
  color: var(--color-text-muted); cursor: pointer; padding: 4px;
  display: flex; align-items: center; justify-content: center;
  border-radius: 4px;
  flex-shrink: 0;
  transition: color .1s, opacity .12s;
}
.icon-btn:hover { color: var(--color-text); }
.liked { color: var(--color-accent); }
.hover-action { opacity: 0; }
.track-row:hover .hover-action,
.hover-action:focus-visible { opacity: 1; }
.play-count {
  display: inline-flex; align-items: center; justify-content: flex-end; gap: 4px;
  min-width: 36px;
  text-align: right;
  font-size: var(--fs-track-side);
  color: var(--color-text-muted);
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
