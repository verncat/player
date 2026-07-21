<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import ResponsivePopup from "./ResponsivePopup.vue";
import type { AudioDevice, Track } from "../types";
import { formatDuration, formatSampleRate, formatTime } from "../utils/format";
import { hashToColors } from "../utils/rarity";

interface CurrentTrackSummary {
  title: string;
  artist: string;
  colors: [string, string];
}

interface Peer {
  name: string;
  host: string;
  port: number;
  addresses: string[];
  device_name?: string;
  device_emoji?: string;
  playback?: any;
}

const props = defineProps<{
  nowPlaying: Track | null;
  currentTrack: CurrentTrackSummary;
  covers: Record<number, string | null>;
  beatScale: number;
  isLiked: boolean;
  isShuffled: boolean;
  isPlaying: boolean;
  repeatMode: number;
  currentTime: number;
  duration: number;
  showSoulseekPreviewBuffer: boolean;
  soulseekPreviewBufferPercent: number;
  displayProgressPercent: number;
  showQueueMenu: boolean;
  queueSource: string;
  queue: Track[];
  showDeviceMenu: boolean;
  outputDevices: AudioDevice[];
  remoteOutputPeer: Peer | null;
  currentDevice: string | null;
  currentSampleRate: number | null;
  remoteOutputDevices: Peer[];
  deviceMenuError: string;
  volume: number;
  nativeSampleRateLabel: string;
  peerLabel: (peer: Peer) => string;
  isRemoteOutputPeer: (peer: Peer) => boolean;
}>();

const emit = defineEmits<{
  openDetail: [];
  "update:isLiked": [value: boolean];
  "update:isShuffled": [value: boolean];
  "update:repeatMode": [value: number];
  "update:showQueueMenu": [value: boolean];
  "update:showDeviceMenu": [value: boolean];
  playPrev: [];
  playNext: [];
  togglePlay: [];
  seek: [event: MouseEvent];
  toggleQueueMenu: [];
  toggleDeviceMenu: [];
  jumpToQueueItem: [index: number];
  pickLocalDevice: [name: string, sampleRate?: number | null, keepOpen?: boolean];
  pickLocalSampleRate: [device: AudioDevice, event: Event];
  pickRemoteDevice: [peer: Peer];
  setVolume: [event: MouseEvent];
}>();

const ZIPPY_TOGGLE_COUNT = 5;
const ZIPPY_TOGGLE_WINDOW_MS = 2_000;
const ZIPPY_INTRO_MS = 5_400;
const ZIPPY_FALLING_MS = 600;
const ZIPPY_PAUSE_STEP_MS = 2_000;
const ZIPPY_CLICK_ANIMATION_MS = 600;
const ZIPPY_PEEK_SRC = "/zippi-sprites/zippi_peek_up_optimized.webp";
const ZIPPY_HANDS_SRC = "/zippi-sprites/zippi_hands_up_optimized.webp";
const ZIPPY_UP_SRC = "/zippi-sprites/zippi_up_optimized.webp";
const ZIPPY_LEFT_SRC = "/zippi-sprites/zippi_left_optimized.webp";
const ZIPPY_FALLING_SRC = "/zippi-sprites/zippi_falling_optimized.webp";
const ZIPPY_BORING_SRC = "/zippi-sprites/zippi_boring_optimized.webp";
const ZIPPY_BORING_POINTS_SRC = "/zippi-sprites/zippi_boring_points_down_optimized.webp";
const ZIPPY_BORING_CLICK_SRC = "/zippi-sprites/zippi_boring_click_down_optimized.webp";

interface ZippySprite {
  key: string;
  src: string;
  position: {
    top: string;
    right: string;
    bottom: string;
    left: string;
    transform: string;
  };
  imageTransform: string;
  motion?: "falling";
}

const ZIPPY_DEFAULT_POSITION = {
  top: "auto",
  right: "auto",
  bottom: "calc(var(--player-footer-height) - 10px)",
  left: "50%",
  transform: "translateX(-50%)",
};
const ZIPPY_HANDS: ZippySprite = {
  key: "hands-up",
  src: ZIPPY_HANDS_SRC,
  position: ZIPPY_DEFAULT_POSITION,
  imageTransform: "translateY(15%)",
};
const ZIPPY_BORING: ZippySprite = {
  key: "boring",
  src: ZIPPY_BORING_SRC,
  position: ZIPPY_DEFAULT_POSITION,
  imageTransform: "translateY(10%)",
};
const ZIPPY_BORING_POINTS: ZippySprite = {
  key: "boring-points-down",
  src: ZIPPY_BORING_POINTS_SRC,
  position: ZIPPY_DEFAULT_POSITION,
  imageTransform: "translateY(10%)",
};
const ZIPPY_BORING_CLICK: ZippySprite = {
  key: "boring-click-down",
  src: ZIPPY_BORING_CLICK_SRC,
  position: ZIPPY_DEFAULT_POSITION,
  imageTransform: "translateY(23%)",
};
const ZIPPY_SPRITES: ZippySprite[] = [
  ZIPPY_HANDS,
  {
    key: "peek-up",
    src: ZIPPY_PEEK_SRC,
    position: ZIPPY_DEFAULT_POSITION,
    imageTransform: "translateY(15%)",
  },
  {
    key: "up",
    src: ZIPPY_UP_SRC,
    position: {
      top: "0",
      right: "auto",
      bottom: "auto",
      left: "50%",
      transform: "translateX(-50%) translateY(-40%)",
    },
    imageTransform: "none",
  },
  {
    key: "left",
    src: ZIPPY_LEFT_SRC,
    position: {
      top: "50%",
      right: "0",
      bottom: "auto",
      left: "auto",
      transform: "translateY(-50%) translateX(30%)",
    },
    imageTransform: "none",
  },
  {
    key: "left-mirrored",
    src: ZIPPY_LEFT_SRC,
    position: {
      top: "50%",
      right: "auto",
      bottom: "auto",
      left: "0",
      transform: "translateY(-50%) translateX(-30%)",
    },
    imageTransform: "scaleX(-1)",
  },
  {
    key: "falling",
    src: ZIPPY_FALLING_SRC,
    position: {
      top: "0",
      right: "auto",
      bottom: "auto",
      left: "50%",
      transform: "translateX(-50%) translateY(-40%)",
    },
    imageTransform: "none",
    motion: "falling",
  },
];

const zippyVisible = ref(false);
const zippyIntroActive = ref(false);
const zippyAnimationRun = ref(0);
const activeZippySprite = ref<ZippySprite>(ZIPPY_HANDS);
const zippyPausePhase = ref<"idle" | "boring" | "points" | "click">("idle");
const playToggleTimestamps: number[] = [];
const zippyPreloadImages: HTMLImageElement[] = [];
let zippyIntroTimer: ReturnType<typeof setTimeout> | undefined;
let zippyPausePointsTimer: ReturnType<typeof setTimeout> | undefined;
let zippyPauseClickTimer: ReturnType<typeof setTimeout> | undefined;
let zippyClickResetTimer: ReturnType<typeof setTimeout> | undefined;
let previousBeatScale = 1;
let zippyMotionLockedUntil = 0;

const displayedZippySprite = computed(() => {
  if (zippyPausePhase.value === "points") return ZIPPY_BORING_POINTS;
  if (zippyPausePhase.value === "click") return ZIPPY_BORING_CLICK;
  if (zippyPausePhase.value === "boring" || !props.isPlaying) return ZIPPY_BORING;
  return activeZippySprite.value;
});
const zippyOverFooter = computed(() => (
  zippyPausePhase.value === "boring"
  || zippyPausePhase.value === "points"
  || zippyPausePhase.value === "click"
));

const zippyBeatStyle = computed(() => {
  const beatScale = props.isPlaying ? props.beatScale : 1;
  const beatDelta = Math.max(0, beatScale - 1);
  const sprite = displayedZippySprite.value;
  const position = sprite.position;
  return {
    top: position.top,
    right: position.right,
    bottom: position.bottom,
    left: position.left,
    transform: sprite.motion === "falling"
      ? position.transform
      : `${position.transform} translate3d(0, ${(-60 * beatDelta).toFixed(2)}px, 0) scale(${beatScale})`,
  };
});

function pickNextZippySprite() {
  const candidates = ZIPPY_SPRITES.filter(
    sprite => sprite.key !== activeZippySprite.value.key,
  );
  const nextSprite = candidates[Math.floor(Math.random() * candidates.length)] ?? ZIPPY_HANDS;
  activeZippySprite.value = nextSprite;
  if (nextSprite.motion === "falling") {
    zippyMotionLockedUntil = performance.now() + ZIPPY_FALLING_MS;
  }
}

function clearZippyPauseTimers(includeClickReset = true) {
  if (zippyPausePointsTimer !== undefined) clearTimeout(zippyPausePointsTimer);
  if (zippyPauseClickTimer !== undefined) clearTimeout(zippyPauseClickTimer);
  if (includeClickReset && zippyClickResetTimer !== undefined) clearTimeout(zippyClickResetTimer);
  zippyPausePointsTimer = undefined;
  zippyPauseClickTimer = undefined;
  if (includeClickReset) zippyClickResetTimer = undefined;
}

function startZippyPauseSequence() {
  clearZippyPauseTimers();
  if (zippyIntroTimer !== undefined) clearTimeout(zippyIntroTimer);
  zippyIntroTimer = undefined;
  zippyIntroActive.value = false;
  zippyPausePhase.value = "boring";

  zippyPausePointsTimer = setTimeout(() => {
    zippyPausePointsTimer = undefined;
    if (!zippyVisible.value || props.isPlaying) return;
    zippyPausePhase.value = "points";

    zippyPauseClickTimer = setTimeout(() => {
      zippyPauseClickTimer = undefined;
      if (!zippyVisible.value || props.isPlaying) return;
      zippyPausePhase.value = "click";
      emit("togglePlay");

      zippyClickResetTimer = setTimeout(() => {
        zippyClickResetTimer = undefined;
        zippyPausePhase.value = props.isPlaying ? "idle" : "boring";
      }, ZIPPY_CLICK_ANIMATION_MS);
    }, ZIPPY_PAUSE_STEP_MS);
  }, ZIPPY_PAUSE_STEP_MS);
}

function enableZippyMode() {
  zippyAnimationRun.value += 1;
  zippyVisible.value = true;
  zippyIntroActive.value = true;
  zippyPausePhase.value = "idle";
  activeZippySprite.value = ZIPPY_HANDS;
  zippyMotionLockedUntil = 0;
  if (zippyIntroTimer !== undefined) clearTimeout(zippyIntroTimer);
  zippyIntroTimer = setTimeout(() => {
    zippyIntroActive.value = false;
    zippyIntroTimer = undefined;
  }, ZIPPY_INTRO_MS);
}

function disableZippyMode() {
  zippyVisible.value = false;
  zippyIntroActive.value = false;
  zippyPausePhase.value = "idle";
  zippyMotionLockedUntil = 0;
  clearZippyPauseTimers();
  if (zippyIntroTimer !== undefined) clearTimeout(zippyIntroTimer);
  zippyIntroTimer = undefined;
}

function toggleZippyMode() {
  if (zippyVisible.value) disableZippyMode();
  else enableZippyMode();
}

function handlePlayToggle() {
  const now = performance.now();
  playToggleTimestamps.push(now);
  while (
    playToggleTimestamps.length > 0
    && playToggleTimestamps[0] < now - ZIPPY_TOGGLE_WINDOW_MS
  ) {
    playToggleTimestamps.shift();
  }

  if (playToggleTimestamps.length >= ZIPPY_TOGGLE_COUNT) {
    playToggleTimestamps.length = 0;
    toggleZippyMode();
  }

  emit("togglePlay");
}

onMounted(() => {
  for (const sprite of [
    ...ZIPPY_SPRITES,
    ZIPPY_BORING,
    ZIPPY_BORING_POINTS,
    ZIPPY_BORING_CLICK,
  ]) {
    const image = new Image();
    image.decoding = "async";
    image.src = sprite.src;
    zippyPreloadImages.push(image);
  }
});

watch(() => props.beatScale, (beatScale) => {
  const isNewBeat = beatScale > 1.025 && beatScale - previousBeatScale > 0.015;
  if (
    isNewBeat
    && props.isPlaying
    && zippyVisible.value
    && !zippyIntroActive.value
    && performance.now() >= zippyMotionLockedUntil
  ) {
    pickNextZippySprite();
  }
  previousBeatScale = beatScale;
});

watch(() => props.isPlaying, (isPlaying) => {
  if (!isPlaying) {
    if (activeZippySprite.value.motion === "falling") {
      activeZippySprite.value = ZIPPY_HANDS;
      zippyMotionLockedUntil = 0;
    }
    if (zippyVisible.value) startZippyPauseSequence();
    return;
  }

  clearZippyPauseTimers(false);
  if (zippyPausePhase.value !== "click") zippyPausePhase.value = "idle";
});

onUnmounted(() => {
  if (zippyIntroTimer !== undefined) clearTimeout(zippyIntroTimer);
  clearZippyPauseTimers();
  zippyPreloadImages.length = 0;
});
</script>

<template>
  <div class="player-shell" :class="{ 'zippy-active': zippyVisible }">
    <div
      v-if="zippyVisible && zippyIntroActive && props.isPlaying && zippyPausePhase === 'idle'"
      :key="zippyAnimationRun"
      class="zippy-stage"
      aria-hidden="true"
    >
      <img
        class="zippy-sprite zippy-peek"
        :src="ZIPPY_PEEK_SRC"
        alt=""
        draggable="false"
      />
      <img
        class="zippy-sprite zippy-hands"
        :src="ZIPPY_HANDS_SRC"
        alt=""
        draggable="false"
      />
    </div>

    <div
      v-if="zippyVisible && (!zippyIntroActive || !props.isPlaying || zippyPausePhase !== 'idle')"
      class="zippy-viewport-stage"
      :class="{ 'zippy-over-footer': zippyOverFooter }"
      aria-hidden="true"
    >
      <Transition name="zippy-swap">
        <div
          :key="displayedZippySprite.key"
          class="zippy-beat"
          :class="{
            'zippy-falling': displayedZippySprite.motion === 'falling',
            'zippy-click-beat': zippyPausePhase === 'click',
          }"
          :style="zippyBeatStyle"
        >
          <img
            class="zippy-viewport-sprite"
            :src="displayedZippySprite.src"
            :style="{
              transform: displayedZippySprite.imageTransform,
              '--zippy-image-transform': displayedZippySprite.imageTransform,
            }"
            alt=""
            draggable="false"
          />
        </div>
      </Transition>
    </div>

    <footer class="player">
    <div class="player-left" @click="nowPlaying && emit('openDetail')" style="cursor: pointer;">
      <div class="thumb" :style="{
        ...(nowPlaying && covers[nowPlaying.id]
          ? { backgroundImage: `url(${covers[nowPlaying.id]})`, backgroundSize: 'cover', backgroundPosition: 'center' }
          : { background: `linear-gradient(135deg, ${currentTrack.colors[0]}, ${currentTrack.colors[1]})` }),
        transform: `scale(${beatScale})`,
        transformOrigin: 'center center',
        willChange: 'transform',
      }" />
      <div class="track-meta">
        <div class="track-name"><span class="marquee-text">{{ currentTrack.title }}&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;{{ currentTrack.title }}</span></div>
        <div class="track-artist">{{ currentTrack.artist }}</div>
      </div>
      <button class="icon-btn" :class="{ green: isLiked }" @click.stop="emit('update:isLiked', !isLiked)">
        <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"/></svg>
      </button>
    </div>

    <div class="player-center">
      <div class="ctrl-row" :style="{ transform: `scale(${beatScale})`, transformOrigin: 'center', willChange: 'transform' }">
        <button class="icon-btn" :class="{ green: isShuffled, dot: isShuffled }" @click="emit('update:isShuffled', !isShuffled)" title="Shuffle">
          <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M10.59 9.17L5.41 4 4 5.41l5.17 5.17 1.42-1.41zM14.5 4l2.04 2.04L4 18.59 5.41 20 17.96 7.46 20 9.5V4h-5.5zm.33 9.41l-1.41 1.41 3.13 3.13L14.5 20H20v-5.5l-2.04 2.04-3.13-3.13z"/></svg>
        </button>
        <button class="icon-btn" title="Previous" @click="emit('playPrev')">
          <svg viewBox="0 0 24 24" fill="currentColor" width="20" height="20"><path d="M6 6h2v12H6zm3.5 6 8.5 6V6z"/></svg>
        </button>
        <button class="play-btn" @click="handlePlayToggle">
          <svg v-if="!isPlaying" viewBox="0 0 24 24" fill="currentColor" width="20" height="20"><path d="M8 5v14l11-7z"/></svg>
          <svg v-else viewBox="0 0 24 24" fill="currentColor" width="20" height="20"><path d="M6 19h4V5H6v14zm8-14v14h4V5h-4z"/></svg>
        </button>
        <button class="icon-btn" title="Next" @click="emit('playNext')">
          <svg viewBox="0 0 24 24" fill="currentColor" width="20" height="20"><path d="M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z"/></svg>
        </button>
        <button class="icon-btn" :class="{ green: repeatMode > 0, dot: repeatMode > 0 }" @click="emit('update:repeatMode', (repeatMode + 1) % 3)" title="Repeat">
          <svg v-if="repeatMode < 2" viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4z"/></svg>
          <svg v-else viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4zm-4-2v-5h-1l-2 1v1h1.5v3H13z"/></svg>
        </button>
      </div>
      <div class="progress-row">
        <span class="time">{{ formatTime(currentTime) }}</span>
        <div class="bar" @click="emit('seek', $event)">
          <div v-if="showSoulseekPreviewBuffer" class="bar-buffer" :style="`width:${soulseekPreviewBufferPercent}%`" />
          <div class="bar-fill" :style="`width:${displayProgressPercent}%`">
            <div class="bar-thumb" />
          </div>
        </div>
        <span class="time">{{ formatTime(duration) }}</span>
      </div>
    </div>

    <div class="player-right">
      <ResponsivePopup
        :open="showQueueMenu"
        wrapper-class="queue-menu-wrapper"
        panel-class="queue-dropdown"
        @close="emit('update:showQueueMenu', false)"
      >
        <template #trigger>
          <button class="icon-btn" title="Queue" @click.stop="emit('toggleQueueMenu')">
            <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M3 13h2v-2H3v2zm0 4h2v-2H3v2zm0-8h2V7H3v2zm4 4h14v-2H7v2zm0 4h14v-2H7v2zM7 7v2h14V7H7z"/></svg>
          </button>
        </template>
        <div class="dropdown-header">Queue · {{ queueSource === "recent" ? "Recently played" : "Library" }}</div>
        <div class="queue-now" v-if="nowPlaying">
          <span class="queue-now-label">Now playing</span>
          <div class="queue-item active">
            <div class="queue-item-cover" :style="covers[nowPlaying.id]
              ? `background-image: url(${covers[nowPlaying.id]}); background-size: cover; background-position: center`
              : `background: linear-gradient(135deg, ${hashToColors(nowPlaying.file_hash)[0]}, ${hashToColors(nowPlaying.file_hash)[1]})`" />
            <div class="queue-item-info">
              <span class="queue-item-title">{{ nowPlaying.title || nowPlaying.path }}</span>
              <span class="queue-item-artist">{{ nowPlaying.artist || "Unknown" }}</span>
            </div>
            <span class="queue-item-dur">{{ formatDuration(nowPlaying.duration_secs) }}</span>
          </div>
        </div>
        <div class="queue-list" v-if="queue.length">
          <span class="queue-next-label">Next up</span>
          <div
            v-for="(track, i) in queue"
            :key="i"
            class="queue-item"
            @click="emit('jumpToQueueItem', i)"
          >
            <div class="queue-item-cover" :style="covers[track.id]
              ? `background-image: url(${covers[track.id]}); background-size: cover; background-position: center`
              : `background: linear-gradient(135deg, ${hashToColors(track.file_hash)[0]}, ${hashToColors(track.file_hash)[1]})`" />
            <div class="queue-item-info">
              <span class="queue-item-title">{{ track.title || track.path }}</span>
              <span class="queue-item-artist">{{ track.artist || "Unknown" }}</span>
            </div>
            <span class="queue-item-dur">{{ formatDuration(track.duration_secs) }}</span>
          </div>
        </div>
        <div v-else class="queue-empty">Queue is empty</div>
      </ResponsivePopup>

      <ResponsivePopup
        :open="showDeviceMenu"
        wrapper-class="device-menu-wrapper"
        panel-class="device-dropdown"
        @close="emit('update:showDeviceMenu', false)"
      >
        <template #trigger>
          <button class="icon-btn" title="Output device" @click.stop="emit('toggleDeviceMenu')">
            <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M17 2H7c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h10c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zM12 20c-1.66 0-3-1.34-3-3s1.34-3 3-3 3 1.34 3 3-1.34 3-3 3zm5-12H7V5h10v3z"/></svg>
          </button>
        </template>
        <div class="dropdown-header">Output device</div>
        <div class="device-list">
          <div
            v-for="dev in outputDevices"
            :key="dev.name"
            class="device-item device-item-local"
            @click="emit('pickLocalDevice', dev.name, undefined, true)"
          >
            <div class="device-item-main">
              <span class="device-check">{{ !remoteOutputPeer && dev.name === currentDevice ? "✓" : "" }}</span>
              <span class="device-name">{{ dev.name }}</span>
              <select
                v-if="!remoteOutputPeer && dev.name === currentDevice && dev.sample_rates.length"
                class="device-sample-rate"
                :value="currentSampleRate ?? ''"
                @click.stop
                @change="emit('pickLocalSampleRate', dev, $event)"
              >
                <option value="">{{ nativeSampleRateLabel }}</option>
                <option v-for="rate in dev.sample_rates" :key="rate" :value="rate">
                  {{ formatSampleRate(rate) }}
                </option>
              </select>
            </div>
          </div>
          <div v-if="remoteOutputDevices.length" class="device-section-label">Player devices</div>
          <a
            v-for="peer in remoteOutputDevices"
            :key="`${peer.host}:${peer.port}`"
            href="#"
            class="device-item"
            @click.prevent="emit('pickRemoteDevice', peer)"
          >
            <span class="device-check">{{ isRemoteOutputPeer(peer) ? "✓" : "" }}</span>
            <span class="device-name">{{ peerLabel(peer) }}</span>
          </a>
        </div>
        <div v-if="deviceMenuError" class="device-error">{{ deviceMenuError }}</div>
      </ResponsivePopup>

      <div class="vol-wrap">
        <button class="icon-btn">
          <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16">
            <path v-if="volume > 0" d="M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z"/>
            <path v-else d="M16.5 12c0-1.77-1.02-3.29-2.5-4.03v2.21l2.45 2.45c.03-.2.05-.41.05-.63zm2.5 0c0 .94-.2 1.82-.54 2.64l1.51 1.51C20.63 14.91 21 13.5 21 12c0-4.28-2.99-7.86-7-8.77v2.06c2.89.86 5 3.54 5 6.71zM4.27 3L3 4.27 7.73 9H3v6h4l5 5v-6.73l4.25 4.25c-.67.52-1.42.93-2.25 1.18v2.06c1.38-.31 2.63-.95 3.69-1.81L19.73 21 21 19.73l-9-9L4.27 3zM12 4L9.91 6.09 12 8.18V4z"/>
          </svg>
        </button>
        <div class="bar vol" @click="emit('setVolume', $event)">
          <div class="bar-fill" :style="`width:${volume}%`">
            <div class="bar-thumb" />
          </div>
        </div>
      </div>
    </div>
    </footer>
  </div>
</template>

<style scoped>
.player-shell {
  --player-footer-height: 90px;
  --zippy-size: 400px;
  grid-column: 1 / 3;
  grid-row: 2 / 3;
  position: relative;
  z-index: 100;
  overflow: visible;
  isolation: isolate;
}

.player {
  background: #181818;
  border-top: 1px solid #282828;
  display: grid;
  grid-template-columns: 1fr 2fr 1fr;
  align-items: center;
  padding: 0 16px;
  overflow: visible;
  position: relative;
  z-index: 1;
  width: 100%;
  height: 100%;
}

.zippy-stage {
  position: absolute;
  left: 50%;
  bottom: calc(100% - 10px);
  width: var(--zippy-size);
  height: var(--zippy-size);
  overflow: hidden;
  pointer-events: none;
  transform: translateX(-50%);
  z-index: 0;
  contain: layout paint;
}

.zippy-sprite {
  position: absolute;
  left: 0;
  bottom: -15%;
  width: 100%;
  height: auto;
  user-select: none;
  transform-origin: 50% 100%;
  will-change: transform, opacity;
  filter:
    drop-shadow(0 8px 12px rgba(0, 0, 0, .48))
    drop-shadow(0 0 12px rgba(74, 226, 221, .13));
}

.zippy-viewport-stage {
  position: fixed;
  inset: 0;
  overflow: hidden;
  pointer-events: none;
  z-index: 0;
}

.zippy-viewport-stage.zippy-over-footer {
  z-index: 2;
}

.zippy-beat {
  position: absolute;
  width: var(--zippy-size);
  height: var(--zippy-size);
  transform-origin: 50% 100%;
  will-change: transform;
}

.zippy-falling {
  animation: zippy-fall .6s linear both;
  transform-origin: 50% 50%;
}

.zippy-click-beat .zippy-viewport-sprite {
  animation: zippy-click-beat .6s linear both;
}

@keyframes zippy-click-beat {
  0% {
    transform: var(--zippy-image-transform) translateY(-6px) scale(1.1);
  }
  20% {
    transform: var(--zippy-image-transform) translateY(-2.4px) scale(1.04);
  }
  50% {
    transform: var(--zippy-image-transform) translateY(-.6px) scale(1.01);
  }
  100% {
    transform: var(--zippy-image-transform) scale(1);
  }
}

@keyframes zippy-fall {
  0% {
    opacity: 1;
    transform: translateX(-50%) translateY(-40%) rotate(-2deg);
  }
  10% {
    transform: translateX(calc(-50% - 8px)) translateY(calc(1vh - 40.6%)) rotate(3deg);
  }
  20% {
    transform: translateX(calc(-50% + 10px)) translateY(calc(4vh - 42.4%)) rotate(-4deg);
  }
  30% {
    transform: translateX(calc(-50% - 12px)) translateY(calc(9vh - 45.4%)) rotate(5deg);
  }
  40% {
    transform: translateX(calc(-50% + 13px)) translateY(calc(16vh - 49.6%)) rotate(-5deg);
  }
  50% {
    transform: translateX(calc(-50% - 14px)) translateY(calc(25vh - 55%)) rotate(6deg);
  }
  60% {
    transform: translateX(calc(-50% + 14px)) translateY(calc(36vh - 61.6%)) rotate(-6deg);
  }
  70% {
    transform: translateX(calc(-50% - 13px)) translateY(calc(49vh - 69.4%)) rotate(5deg);
  }
  80% {
    transform: translateX(calc(-50% + 11px)) translateY(calc(64vh - 78.4%)) rotate(-4deg);
  }
  90% {
    opacity: 1;
    transform: translateX(calc(-50% - 7px)) translateY(calc(81vh - 88.6%)) rotate(3deg);
  }
  100% {
    opacity: 0;
    transform: translateX(-50%) translateY(calc(100vh - 100%)) rotate(0);
  }
}

.zippy-viewport-sprite {
  position: absolute;
  inset: 0;
  width: 100%;
  height: auto;
  opacity: 1;
  user-select: none;
  transform-origin: 50% 50%;
  will-change: transform, opacity;
  filter:
    drop-shadow(0 8px 12px rgba(0, 0, 0, .48))
    drop-shadow(0 0 12px rgba(74, 226, 221, .13));
}

.zippy-swap-enter-active,
.zippy-swap-leave-active {
  transition: opacity .14s ease, filter .14s ease;
}

.zippy-swap-enter-from {
  opacity: 0;
  filter:
    brightness(1.7)
    saturate(1.35)
    drop-shadow(0 8px 12px rgba(0, 0, 0, .48));
}

.zippy-swap-leave-to {
  opacity: 0;
  filter:
    brightness(1.25)
    saturate(1.2)
    drop-shadow(0 8px 12px rgba(0, 0, 0, .48));
}

.player-left,
.player-center,
.player-right {
  position: relative;
  z-index: 1;
}

.zippy-peek {
  animation: zippy-peek-sequence 2.25s both;
}

.zippy-hands {
  animation: zippy-hands-sequence 3.8s 1.45s both;
}

@keyframes zippy-peek-sequence {
  0% {
    opacity: 0;
    transform: translate3d(0, 105%, 0) rotate(-4deg) scale(.94);
    animation-timing-function: cubic-bezier(.16, 1.28, .32, 1);
  }
  8% { opacity: 1; }
  34% {
    opacity: 1;
    transform: translate3d(0, -3%, 0) rotate(2deg) scale(1.025);
    animation-timing-function: cubic-bezier(.4, 0, .2, 1);
  }
  46% { transform: translate3d(0, 0, 0) rotate(-.7deg) scale(1); }
  58% { transform: translate3d(0, -1.5%, 0) rotate(.8deg) scale(1.005); }
  70% {
    opacity: 1;
    transform: translate3d(0, 0, 0) rotate(0) scale(1);
    animation-timing-function: cubic-bezier(.55, .02, .72, .42);
  }
  100% {
    opacity: 0;
    transform: translate3d(0, 108%, 0) rotate(4deg) scale(.96);
  }
}

@keyframes zippy-hands-sequence {
  0% {
    opacity: 0;
    transform: translate3d(0, 110%, 0) rotate(0) scale(.9);
    animation-timing-function: cubic-bezier(.12, 1.22, .3, 1);
  }
  6% { opacity: 1; }
  24% {
    opacity: 1;
    transform: translate3d(0, -3%, 0) rotate(-2deg) scale(1.035);
    animation-timing-function: cubic-bezier(.38, 0, .2, 1);
  }
  34% { transform: translate3d(0, 0, 0) rotate(1deg) scale(1); }
  45% { transform: translate3d(0, -1.4%, 0) rotate(-1.1deg) scale(1.008); }
  56% { transform: translate3d(0, 0, 0) rotate(.7deg) scale(1); }
  67% { transform: translate3d(0, -.9%, 0) rotate(-.5deg) scale(1.004); }
  79% {
    opacity: 1;
    transform: translate3d(0, 0, 0) rotate(0) scale(1);
    animation-timing-function: cubic-bezier(.58, .02, .76, .4);
  }
  100% {
    opacity: 0;
    transform: translate3d(0, 112%, 0) rotate(-3deg) scale(.96);
  }
}

.player-left {
  display: flex;
  align-items: center;
  gap: 12px;
  min-width: 0;
}
.thumb {
  width: 56px;
  height: 56px;
  border-radius: 4px;
  flex-shrink: 0;
}
.track-meta { min-width: 0; }
.track-name {
  font-size: var(--fs-player-title);
  font-weight: 600;
  overflow: hidden;
  mask-image: linear-gradient(to right, transparent 0%, black 5%, black 85%, transparent 100%);
  -webkit-mask-image: linear-gradient(to right, transparent 0%, black 5%, black 85%, transparent 100%);
}
.track-artist {
  font-size: var(--fs-player-meta);
  color: #a7a7a7;
  margin-top: 3px;
}

.player-center {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 14px 0;
}
.ctrl-row {
  display: flex;
  align-items: center;
  gap: 16px;
}

.icon-btn {
  background: none;
  border: none;
  color: #b3b3b3;
  cursor: pointer;
  padding: 4px;
  display: flex;
  align-items: center;
  justify-content: center;
  position: relative;
  border-radius: 4px;
  transition: color .1s;
}
.icon-btn:hover { color: #fff; }
.icon-btn.green { color: #1db954; }
.icon-btn.green:hover { color: #1ed760; }
.icon-btn.dot::after {
  content: '';
  position: absolute;
  bottom: -3px;
  left: 50%;
  transform: translateX(-50%);
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: #1db954;
}

.play-btn {
  width: 36px;
  height: 36px;
  border-radius: 50%;
  background: #fff;
  border: none;
  color: #000;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: transform .1s, background .1s;
}
.play-btn:hover {
  transform: scale(1.06);
  background: #f0f0f0;
}

.progress-row {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
}
.time {
  font-size: var(--fs-body-sm);
  color: #a7a7a7;
  min-width: 34px;
  text-align: center;
}

.bar {
  flex: 1;
  height: 4px;
  background: #535353;
  border-radius: 2px;
  cursor: pointer;
  position: relative;
}
.bar:hover .bar-fill { background: #1db954; }
.bar:hover .bar-thumb { opacity: 1; }
.bar-fill {
  height: 100%;
  background: #fff;
  border-radius: 2px;
  position: relative;
  transition: background .1s;
}
.bar-buffer {
  position: absolute;
  left: 0;
  top: 50%;
  height: 2px;
  transform: translateY(-50%);
  background: rgba(255, 255, 255, 0.22);
  border-radius: 999px;
  pointer-events: none;
}
.bar-thumb {
  position: absolute;
  right: -6px;
  top: 50%;
  transform: translateY(-50%);
  width: 12px;
  height: 12px;
  border-radius: 50%;
  background: #fff;
  opacity: 0;
  transition: opacity .1s;
}

.player-right {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
}
.vol-wrap {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 130px;
}
.bar.vol { flex: 1; }

.dropdown-header {
  padding: 10px 16px 6px;
  font-size: var(--fs-dropdown-label);
  font-weight: 700;
  color: #a7a7a7;
  text-transform: uppercase;
  letter-spacing: .04em;
}

:global(.device-menu-wrapper),
:global(.queue-menu-wrapper) {
  position: relative;
}
:global(.device-dropdown),
:global(.queue-dropdown) {
  position: absolute;
  top: auto;
  bottom: calc(100% + 12px);
  right: 0;
}
:global(.device-dropdown) {
  min-width: 220px;
}
:global(.queue-dropdown) {
  min-width: 300px;
  max-width: 360px;
}

.device-list {
  max-height: 240px;
  overflow-y: auto;
}
.device-list::-webkit-scrollbar { width: 6px; }
.device-list::-webkit-scrollbar-thumb {
  background: #555;
  border-radius: 3px;
}
.device-list::-webkit-scrollbar-track { background: transparent; }
.device-section-label {
  padding: 10px 16px 6px;
  color: #8b8b8b;
  font-size: var(--fs-eyebrow);
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: .04em;
}
.device-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 16px;
  color: #fff;
  text-decoration: none;
  font-size: var(--fs-dropdown-item);
  cursor: pointer;
}
.device-item:hover { background: #3e3e3e; }
.device-item-local { gap: 10px; }
.device-item-main {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  min-width: 0;
}
.device-check {
  width: 18px;
  text-align: center;
  color: #1db954;
  font-size: var(--fs-dropdown-item);
  flex-shrink: 0;
}
.device-name {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
}
.device-sample-rate {
  width: 178px;
  flex: 0 0 178px;
  min-height: 30px;
  padding: 5px 32px 5px 12px;
  border: 1px solid #535353;
  border-radius: 20px;
  background-color: transparent;
  color: #a7a7a7;
  font-size: var(--fs-control);
  font-weight: 600;
  line-height: 1.2;
  cursor: pointer;
  appearance: none;
  -webkit-appearance: none;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='%23a7a7a7'%3E%3Cpath d='M7 10l5 5 5-5z'/%3E%3C/svg%3E");
  background-repeat: no-repeat;
  background-position: right 12px center;
  background-size: 12px 12px;
  outline: none;
  transition: border-color .15s, color .15s, background-color .15s;
}
.device-sample-rate:hover,
.device-sample-rate:focus {
  border-color: #fff;
  color: #fff;
}
.device-sample-rate option {
  background: #282828;
  color: #fff;
}
.device-error {
  padding: 0 16px 12px;
  color: #e9283e;
  font-size: var(--fs-body-sm);
}

.queue-now-label,
.queue-next-label {
  display: block;
  padding: 8px 16px 4px;
  font-size: var(--fs-eyebrow);
  font-weight: 700;
  color: #a7a7a7;
  text-transform: uppercase;
  letter-spacing: .04em;
}
.queue-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 16px;
  cursor: pointer;
  transition: background .1s;
}
.queue-item:hover { background: #3e3e3e; }
.queue-item.active {
  background: #333;
  cursor: default;
}
.queue-item-cover {
  width: 36px;
  height: 36px;
  border-radius: 3px;
  flex-shrink: 0;
}
.queue-item-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.queue-item-title {
  font-size: var(--fs-queue-title);
  font-weight: 600;
  color: #fff;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.queue-item.active .queue-item-title { color: #1db954; }
.queue-item-artist {
  font-size: var(--fs-queue-meta);
  color: #a7a7a7;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.queue-item-dur {
  font-size: var(--fs-queue-dur);
  color: #a7a7a7;
  flex-shrink: 0;
}
.queue-list {
  max-height: 260px;
  overflow-y: auto;
}
.queue-list::-webkit-scrollbar { width: 6px; }
.queue-list::-webkit-scrollbar-thumb {
  background: #555;
  border-radius: 3px;
}
.queue-list::-webkit-scrollbar-track { background: transparent; }
.queue-empty {
  padding: 16px;
  color: #a7a7a7;
  font-size: var(--fs-body-md);
  text-align: center;
}

.marquee-text {
  display: inline-block;
  white-space: nowrap;
  animation: marquee 14s linear 2s infinite;
}

@keyframes marquee {
  0% { transform: translateX(0); }
  15% { transform: translateX(0); }
  85% { transform: translateX(-50%); }
  100% { transform: translateX(-50%); }
}

@media (max-width: 768px) {
  .player-shell {
    --player-footer-height: calc(130px + env(safe-area-inset-bottom));
    --zippy-size: clamp(150px, 43vw, 190px);
    grid-column: 1;
    grid-row: 2 / 3;
  }
  .player {
    display: flex;
    flex-direction: column;
    padding: 0 10px env(safe-area-inset-bottom);
    justify-content: center;
    gap: 8px;
    overflow: hidden;
    min-width: 0;
  }
  .player-left {
    display: flex;
    align-items: center;
    gap: 8px;
    justify-content: flex-start;
    order: -1;
    min-width: 0;
    margin-top: 10px;
    padding: 0 15px;
    width: 100%;
  }
  .player-right { display: none; }
  .player-left .thumb {
    width: 40px;
    height: 40px;
  }
  .player-left .track-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex-grow: 1;
  }
  .track-name {
    font-size: var(--fs-player-title);
    font-weight: 600;
    color: #fff;
  }
  .track-artist {
    font-size: var(--fs-player-meta);
    color: #a7a7a7;
  }
  .player-center {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 0;
    align-items: stretch;
    order: 0;
  }
  .ctrl-row {
    gap: 8px;
    justify-content: center;
  }
  .icon-btn { padding: 6px; }
  .play-btn {
    width: 40px;
    height: 40px;
  }
  .progress-row .time { display: none; }
  .player-right .vol-wrap { display: none; }
  .progress-row { display: none; }
  .bar { flex: 1; }

  :global(.queue-dropdown) {
    min-width: 260px;
    max-width: 92vw;
  }
  :global(.device-dropdown) {
    min-width: 200px;
  }
}

@media (prefers-reduced-motion: reduce) {
  .zippy-peek {
    animation: zippy-peek-reduced 2.25s both;
  }
  .zippy-hands {
    animation: zippy-hands-reduced 3.8s 1.45s both;
  }
}

@keyframes zippy-peek-reduced {
  0%, 100% { opacity: 0; transform: translateY(100%); }
  18%, 72% { opacity: 1; transform: translateY(0); }
}

@keyframes zippy-hands-reduced {
  0%, 100% { opacity: 0; transform: translateY(100%); }
  18%, 78% { opacity: 1; transform: translateY(0); }
}

@media (max-width: 480px) {
  .player {
    grid-template-columns: 1fr auto 1fr;
    padding: 0 8px;
    padding-bottom: env(safe-area-inset-top);
  }
  .player-left { width: 100%; }
  .player-left .thumb {
    width: 40px;
    height: 40px;
  }
  .ctrl-row { gap: 8px; }
  .player-right :global(.device-menu-wrapper) { display: none; }
}
</style>
