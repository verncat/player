<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { openUrl } from "@tauri-apps/plugin-opener";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import {
  TRACK_RARITY_OPTIONS,
  type AudioDevice,
  type DeviceList,
  type SoulseekDownloadEvent,
  type SoulseekSearchResult,
  type SoulseekStatus,
  type Track,
  type TrackPage,
} from "./types";
import {
  formatBytes,
  formatDuration,
  formatSampleRate,
  formatTime,
  formatTransferRate,
} from "./utils/format";
import {
  normalizeNonNegativeInteger,
  normalizeOptionalInteger,
  normalizePath,
  normalizeTrackTagsInput,
  parseTrackDateInput,
  trackDateInputValue,
} from "./utils/tracks";
import { highlightedRarity, rarityClass, rarityColorName, rarityVars } from "./utils/rarity";
import { activeThemeColors, translucent } from "./theme";
import AppSidebar from "./components/AppSidebar.vue";
import AppIcon from "./components/AppIcon.vue";
import AppSelect from "./components/AppSelect.vue";
import PlaylistArtwork from "./components/PlaylistArtwork.vue";
import AppTopbar from "./components/AppTopbar.vue";
import PlayerBar from "./components/PlayerBar.vue";
import WebGLAlbumRenderer from "./components/WebGLAlbumRenderer.vue";
import ResponsivePopup from "./components/ResponsivePopup.vue";
import { useDiscordPresence } from "./composables/useDiscordPresence";
import SettingsView from "./components/SettingsView.vue";
import AboutPanel from "./components/AboutPanel.vue";
import { useAppearance } from "./composables/useAppearance";
import { SETTINGS_SECTIONS, type SettingsSection, type DeviceSettings, type AboutInfo, type AboutUpdateStatus } from "./types/settings";
import ArtistView from "./components/ArtistView.vue";
import AlbumView from "./components/AlbumView.vue";
import TrackRow from "./components/TrackRow.vue";
import CoverImage from "./components/CoverImage.vue";
import { provideTrackActions } from "./composables/useTrackActions";
import { useHistoryNavigation } from "./composables/useHistoryNavigation";
import { artistLabel, hasArtistPage } from "./utils/artists";
import { countAlbumTracks } from "./api";
import { coverUrl, placeholderCoverColors } from "./utils/covers";

const { preferences: appearance, reducedMotion } = useAppearance();
watch([reducedMotion, () => appearance.rarityAnimations], ([reduce, rarity]) => {
  document.documentElement.classList.toggle('player-reduced-motion', reduce);
  document.documentElement.classList.toggle('player-no-rarity-motion', !rarity);
}, { immediate: true });

const madeForYou = [
  { id: 7,  title: "The Logic Gate Sh...", artist: "John von Neumann", colors: ["#4527a0", "#283593"] },
  { id: 8,  title: "C++ Concerto", artist: "Bjarne Stroustrup", colors: ["#bf360c", "#b71c1c"] },
  { id: 9,  title: "Unix Serenade", artist: "Ken Thompson", colors: ["#ad1457", "#e91e8c"] },
  { id: 10, title: "The Art of Progra...", artist: "Donald Knuth", colors: ["#1565c0", "#0d47a1"] },
  { id: 11, title: "Python Polka", artist: "Guido van Rossum", colors: ["#2e7d32", "#1b5e20"] },
  { id: 12, title: "Graph Theory Gro...", artist: "Edsger Dijkstra", colors: ["#f57f17", "#e65100"] },
];

const isPlaying = ref(false);
const currentTime = ref(0);
const duration = ref(213);
const volume = ref(70);
const isShuffled = ref(false);
const repeatMode = ref(0);
const showUserMenu = ref(false);
const showDeviceMenu = ref(false);
const showMobileNav = ref(false);
const outputDevices = ref<AudioDevice[]>([]);
const currentDevice = ref<string | null>(null);
const selectedOutputDevice = ref<string | null>(null);
const audioLoading = ref(false);
const audioBusy = ref(false);
const currentSampleRate = ref<number | null>(null);
const nativeSampleRate = ref<number | null>(null);
const deviceMenuError = ref('');
const activeNav = ref("home");
const isSettingsNav = computed(() => activeNav.value === 'settings' || activeNav.value.startsWith('settings/'));
const settingsSection = computed<SettingsSection | null>(() => {
  const section = activeNav.value.split('/')[1];
  return SETTINGS_SECTIONS.find((item) => item.id === section)?.id ?? null;
});
function openSettings(section: SettingsSection | null = null) {
  activeNav.value = section ? `settings/${section}` : 'settings';
  showMobileNav.value = false;
  showDeviceMenu.value = false;
  showQueueMenu.value = false;
}
const navTarget = ref('');
const navTargetArtist = ref('');
interface NavEntry { nav: string; target: string; targetArtist: string }
let _navSkipHistory = false;
function showNavEntry(entry: NavEntry) {
  _navSkipHistory = true;
  navTarget.value = entry.target;
  navTargetArtist.value = entry.targetArtist;
  activeNav.value = entry.nav;
  nextTick(() => { _navSkipHistory = false; });
}
const navigation = useHistoryNavigation<NavEntry>({ nav: 'home', target: '', targetArtist: '' }, showNavEntry);
function pushNavHistory(entry: NavEntry) {
  const current = navigation.current();
  if (current.nav === entry.nav && current.target === entry.target
    && current.targetArtist === entry.targetArtist) return;
  navigation.push(entry);
}
watch(activeNav, (nav) => {
  if (_navSkipHistory) return;
  const hasTarget = nav === 'artist' || nav === 'album';
  pushNavHistory({
    nav,
    target: hasTarget ? navTarget.value : '',
    targetArtist: nav === 'album' ? navTargetArtist.value : '',
  });
});
const canNavBack = navigation.canBack;
const canNavForward = navigation.canForward;
const navBack = navigation.back;
const navForward = navigation.forward;
function openArtist(name: string) {
  navTarget.value = name;
  navTargetArtist.value = '';
  activeNav.value = 'artist';
  pushNavHistory({ nav: 'artist', target: name, targetArtist: '' });
  showMobileNav.value = false;
}
function replaceAlbumNavEntry(name: string, albumArtistOrArtist: string) {
  navTarget.value = name;
  navTargetArtist.value = albumArtistOrArtist;
  navigation.replace({ nav: 'album', target: name, targetArtist: albumArtistOrArtist });
}
function openAlbum(name: string, albumArtistOrArtist: string) {
  navTarget.value = name;
  navTargetArtist.value = albumArtistOrArtist;
  activeNav.value = 'album';
  pushNavHistory({ nav: 'album', target: name, targetArtist: albumArtistOrArtist });
  showMobileNav.value = false;
}
const libraryTracks = ref<Track[]>([]);
const showDuplicateTracks = ref(false);
const libraryLoading = ref(true);
const libraryQuery = ref("");
// The query that produced the currently retained pages. Playback can continue
// after navigating away from Library, so it must not depend on activeNav.
const libraryLoadedQuery = ref("");
const libraryTotal = ref(0);
const libraryOffset = ref(0);
const libraryHasMore = ref(false);
const libraryPageSize = ref(12);
const libraryLoadingMore = ref(false);
const searchQuery = ref("");
const localSearchTracks = ref<Track[]>([]);
const localSearchTotal = ref(0);
const localSearchOffset = ref(0);
const localSearchHasMore = ref(false);
const localSearchLoading = ref(false);
const editingTrack = ref<Track | null>(null);
const editForm = ref({
  title: '',
  artist: '',
  album: '',
  album_artist: '',
  track_number: null as number | null,
  disc_number: null as number | null,
  year: null as number | null,
  genre: '',
  tags: '',
  play_count: 0,
  is_liked: false,
  date_added: '',
  rarity: '',
});
const libraryVersion = ref(0);

const HOME_TRACK_PAGE_SIZE = 12;
const LIBRARY_TRACK_PAGE_SIZE = 50;

function filterDuplicateTracks(tracks: Track[]) {
  return showDuplicateTracks.value ? tracks : tracks.filter((track) => !track.is_duplicate);
}

/* ── Beat animation ── */
const beatScale = ref(1);
let beatRafId: number | null = null;
let beatStartTime = 0;
const scheduledBeatTimeoutIds = new Set<number>();
const BEAT_AMP = 0.10;    // max scale overshoot (1.10× at peak)
const BEAT_TAU = 130;     // exponential decay time-constant in ms

function scheduleBeatAnimation(delayMs: number) {
  const timeoutId = window.setTimeout(() => {
    scheduledBeatTimeoutIds.delete(timeoutId);
    if (!isPlaying.value) return;
    startBeatAnimation(0);
  }, delayMs);
  scheduledBeatTimeoutIds.add(timeoutId);
}

function startBeatAnimation(lagMs = 0) {
  if (reducedMotion.value) return;
  if (lagMs < -8) {
    scheduleBeatAnimation(-lagMs);
    return;
  }

  beatStartTime = performance.now() - Math.max(0, lagMs);
  if (beatRafId !== null) return; // already running — just reset start time
  function tick() {
    const elapsed = performance.now() - beatStartTime;
    const s = 1 + BEAT_AMP * Math.exp(-elapsed / BEAT_TAU);
    beatScale.value = s;
    if (s - 1 > 0.001) {
      beatRafId = requestAnimationFrame(tick);
    } else {
      beatScale.value = 1;
      beatRafId = null;
    }
  }
  beatRafId = requestAnimationFrame(tick);
}

/* ── Identify state ── */
const identifyRunning = ref(false);
const identifyMinimized = ref(false);
const identifyCurrent = ref(0);
const identifyTotal = ref(0);
const identifyDone = ref(false);
interface IdentifyItem { track_id: number; track_name: string | null; status: string; message: string | null }
const identifyResults = ref<IdentifyItem[]>([]);
const identifyLogRef = ref<HTMLElement | null>(null);
const contentRef = ref<HTMLElement | null>(null);

/* ── Dedup state ── */
interface DuplicateGroup {
  tracks: Track[];
  reasons: string[];
}
const dedupLoading = ref(false);
const dedupGroups = ref<DuplicateGroup[]>([]);
const dedupError = ref('');
// Map of groupIndex → selected track ids that should be flagged as duplicates.
const dedupDuplicateIds = ref<Map<number, Set<number>>>(new Map());
// Filter: 'all' | 'unresolved'
const dedupFilter = ref<'all' | 'unresolved'>('all');
// Tracks pending delete after preview confirmation
const dedupConfirmOpen = ref(false);
const dedupApplying = ref(false);

function buildInitialDedupSelections(groups: DuplicateGroup[]) {
  const initialSelections = new Map<number, Set<number>>();
  groups.forEach((group, groupIdx) => {
    const persistedDuplicates = group.tracks.filter((track) => track.is_duplicate).map((track) => track.id);
    if (persistedDuplicates.length > 0) {
      initialSelections.set(groupIdx, new Set(persistedDuplicates));
      return;
    }
    initialSelections.set(groupIdx, new Set(group.tracks.slice(1).map((track) => track.id)));
  });
  return initialSelections;
}

function dedupIsMarkedDuplicate(groupIdx: number, trackId: number) {
  return dedupDuplicateIds.value.get(groupIdx)?.has(trackId) ?? false;
}

function dedupMarkedCount(groupIdx: number, tracks: Track[]) {
  return tracks.reduce((count, track) => count + (dedupIsMarkedDuplicate(groupIdx, track.id) ? 1 : 0), 0);
}

function dedupToggleTrack(groupIdx: number, trackId: number) {
  const nextSelections = new Map(dedupDuplicateIds.value);
  const groupSelections = new Set(nextSelections.get(groupIdx) ?? []);
  if (groupSelections.has(trackId)) {
    groupSelections.delete(trackId);
  } else {
    groupSelections.add(trackId);
  }
  nextSelections.set(groupIdx, groupSelections);
  dedupDuplicateIds.value = nextSelections;
}

const dedupMarkedTotal = computed(() => dedupGroups.value.reduce(
  (count, group, groupIdx) => count + dedupMarkedCount(groupIdx, group.tracks),
  0,
));

const dedupHasPersistedFlags = computed(() => dedupGroups.value.some((group) => group.tracks.some((track) => track.is_duplicate)));

async function openDedup() {
  if (dedupLoading.value) return;
  dedupLoading.value = true;
  dedupError.value = '';
  dedupGroups.value = [];
  dedupDuplicateIds.value = new Map();
  try {
    const groups = await invoke<DuplicateGroup[]>('find_duplicates');
    dedupGroups.value = groups;
    dedupDuplicateIds.value = buildInitialDedupSelections(groups);
  } catch (e) {
    dedupError.value = String(e ?? 'Failed to scan for duplicates');
  } finally {
    dedupLoading.value = false;
  }
}

const dedupFilteredGroups = computed(() => {
  if (dedupFilter.value === 'unresolved') {
    return dedupGroups.value.map((g, i) => ({ g, i })).filter(({ g, i }) => {
      const markedCount = dedupMarkedCount(i, g.tracks);
      return markedCount > 0 && markedCount < g.tracks.length;
    });
  }
  return dedupGroups.value.map((g, i) => ({ g, i }));
});

async function applyDedup() {
  dedupApplying.value = true;
  dedupError.value = '';
  try {
    interface DedupeResolution { keep_ids: number[]; duplicate_ids: number[] }
    const resolutions: DedupeResolution[] = [];
    dedupGroups.value.forEach((g, i) => {
      const keepIds = g.tracks.filter((track) => !dedupIsMarkedDuplicate(i, track.id)).map((track) => track.id);
      const duplicateIds = g.tracks.filter((track) => dedupIsMarkedDuplicate(i, track.id)).map((track) => track.id);
      if (keepIds.length > 0 || duplicateIds.length > 0) {
        resolutions.push({ keep_ids: keepIds, duplicate_ids: duplicateIds });
      }
    });
    await invoke('apply_dedup', { resolutions });
    dedupConfirmOpen.value = false;
    await openDedup();
    await loadLibrary();
    await loadRecent();
  } catch (e) {
    dedupError.value = String(e ?? 'Failed to mark duplicates');
  } finally {
    dedupApplying.value = false;
  }
}

/* ── Index progress state ── */
const indexRunning = ref(false);
const libraryScanBusy = ref(false);
const identifyPreparing = ref(false);
const indexCurrent = ref(0);
const indexTotal = ref(0);
const indexAdded = ref(0);
const indexDone = ref(false);
let indexDismissTimer: ReturnType<typeof setTimeout> | null = null;

interface LogSession {
  id: string;           // unique per session
  kind: 'local' | 'sync';
  // local
  addedCount: number;
  // sync
  device?: string;
  emoji?: string;
  filesAdded: number;
  files: string[];
  total?: number;
  currentMessage?: string;
  phase?: string;
  // shared
  startedAt: number;
  finishedAt?: number;
  status: 'running' | 'done' | 'error';
  errorMsg?: string;
}
const indexLog = ref<LogSession[]>([]);
const indexLogRef = ref<HTMLElement | null>(null);
const indexLogOpen = ref(false);
let suppressLocalIndexLogUntil = 0;

/* ── Queue state ── */
type QueueSource = 'recent' | 'library' | 'playlist' | 'soulseek';
type QueuedPagedSource =
  | { kind: 'library'; query: string; includeDuplicates: boolean; hasMore: boolean }
  | { kind: 'regular-playlist'; playlistId: number; name: string; includeDuplicates: boolean; hasMore: boolean }
  | { kind: 'smart-playlist'; playlist: SmartPlaylist; includeDuplicates: boolean; hasMore: boolean };
const queueSource = ref<QueueSource>('library');
const queueSourceIndex = ref(0);       // index into source list of the LAST item pushed
const queueLibraryTracks = ref<Track[]>([]);
const queuePlaylistTracks = ref<Track[]>([]); // tracks for 'playlist' source
const queueSoulseekTracks = ref<Track[]>([]);
const queuedPagedSource = ref<QueuedPagedSource | null>(null);
const queuedPageLoading = ref(false);
const queue = ref<Track[]>([]);         // upcoming tracks (max 5 visible)
const nowPlaying = ref<Track | null>(null);
const pendingLikes = ref(new Set<number>());
const discord = useDiscordPresence(() => nowPlaying.value ? {
  title: nowPlaying.value.title || 'Unknown track',
  artist: nowPlaying.value.artist || 'Unknown artist',
  album: nowPlaying.value.album || '',
  position: currentTime.value,
  duration: duration.value,
  playing: isPlaying.value,
} : null);
const recentTracks = ref<Track[]>([]);
const showQueueMenu = ref(false);

interface PlayHistoryEntry {
  played_at: number; // unix timestamp (seconds)
  track: Track;
}
const historyEntries = ref<PlayHistoryEntry[]>([]);
const historyLoading = ref(false);
const aboutInfo = ref<AboutInfo | null>(null);
const aboutLoading = ref(false);
const aboutError = ref('');
const aboutUpdateStatus = ref<AboutUpdateStatus | null>(null);
const aboutCheckingUpdates = ref(false);
const aboutUpdateError = ref('');
const aboutUpdating = ref(false);
const aboutUpdateDone = ref('');
const aboutUpdateProgress = ref<number | null>(null);
const aboutDesktopUpdateReady = ref(false);
let pendingDesktopUpdate: Update | null = null;

const supportsTauriUpdater = computed(() => (
  aboutInfo.value !== null
  && ['linux', 'macos', 'windows'].includes(aboutInfo.value.platform)
));
const hasAvailableUpdate = computed(() => aboutUpdateStatus.value?.has_update === true);

// ── Playlists ──────────────────────────────────────────────────────────────
interface Playlist {
  id: number;
  name: string;
  created_at: number;
  track_count: number;
  pinned: boolean;
  pinned_at: number | null;
}
const playlists = ref<Playlist[]>([]);
interface PlaylistView {
  id: number;
  name: string;
  tracks: Track[];
  total: number;
  hasMore: boolean;
  loadingMore: boolean;
}
const playlistView = ref<PlaylistView | null>(null);
const showNewPlaylistInput = ref(false);
const newPlaylistName = ref('');
const homePinnedRegularTracks = ref<Record<number, Track[]>>({});
// context menu for "add to playlist"
const addToPlaylistMenu = ref<{ track: Track; x: number; y: number } | null>(null);
const trackContextMenu = ref<{ track: Track; x: number; y: number; playlistId: number | null } | null>(null);
const homePinnedContextMenu = ref<{ item: HomePinnedPlaylistItem; x: number; y: number } | null>(null);
const TRACK_MENU_MARGIN = 12;
const TRACK_MENU_WIDTH = 240;
const TRACK_LONG_PRESS_DELAY = 420;
const TRACK_LONG_PRESS_MOVE_TOLERANCE = 12;
let trackLongPressTimer: ReturnType<typeof setTimeout> | null = null;
let trackTouchStartX = 0;
let trackTouchStartY = 0;
let trackLongPressContext: { track: Track; x: number; y: number; playlistId: number | null } | null = null;
let homePinnedLongPressTimer: ReturnType<typeof setTimeout> | null = null;
let homePinnedTouchStartX = 0;
let homePinnedTouchStartY = 0;
let homePinnedLongPressContext: { item: HomePinnedPlaylistItem; x: number; y: number } | null = null;
let suppressTrackRowClickUntil = 0;

async function loadPlaylists() {
  try {
    playlists.value = await invoke<Playlist[]>('get_playlists');
  } catch (e) {
    console.error('Failed to load playlists:', e);
  }
}

async function createPlaylist() {
  const name = newPlaylistName.value.trim();
  await invoke('create_playlist', { name });
  newPlaylistName.value = '';
  showNewPlaylistInput.value = false;
  await loadPlaylists();
}

async function deletePlaylist(id: number) {
  await invoke('delete_playlist', { id });
  if (playlistView.value?.id === id) playlistView.value = null;
  await loadPlaylists();
}

async function togglePlaylistPinned(pl: Playlist) {
  await invoke('set_playlist_pinned', { id: pl.id, pinned: !pl.pinned });
  await loadPlaylists();
}

async function loadHomePinnedRegularTracks() {
  const pinnedPlaylists = playlists.value.filter((pl) => pl.pinned);
  if (pinnedPlaylists.length === 0) {
    homePinnedRegularTracks.value = {};
    return;
  }

  const entries = await Promise.all(
    pinnedPlaylists.map(async (pl) => [
      pl.id,
      filterDuplicateTracks(await invoke<Track[]>('get_playlist_tracks', { playlistId: pl.id })),
    ] as const),
  );

  homePinnedRegularTracks.value = Object.fromEntries(entries);
}

async function openPlaylist(pl: Playlist) {
  await loadPlaylistPage(pl.id, pl.name);
}

async function loadPlaylistPage(playlistId: number, name: string, append = false) {
  const current = playlistView.value;
  const offset = append && current?.id === playlistId ? current.tracks.length : 0;
  if (append && current) current.loadingMore = true;
  try {
    const page = await invoke<TrackPage>('get_playlist_tracks_page', {
      playlistId,
      limit: LIBRARY_TRACK_PAGE_SIZE,
      offset,
      includeDuplicates: showDuplicateTracks.value,
    });
    const tracks = append && current?.id === playlistId
      ? [...current.tracks, ...page.tracks]
      : page.tracks;
    playlistView.value = {
      id: playlistId,
      name,
      tracks,
      total: page.total,
      hasMore: page.has_more,
      loadingMore: false,
    };
  } catch (error) {
    if (current?.id === playlistId) current.loadingMore = false;
    console.error('Failed to load playlist:', error);
  }
}

function loadMorePlaylistTracks() {
  const view = playlistView.value;
  if (!view || !view.hasMore || view.loadingMore) return;
  void loadPlaylistPage(view.id, view.name, true);
}

async function addTrackToPlaylist(playlistId: number, trackId: number) {
  await invoke('add_track_to_playlist', { playlistId, trackId });
  await loadPlaylists();
  if (playlistView.value?.id === playlistId) {
    await loadPlaylistPage(playlistId, playlistView.value.name);
  }
  addToPlaylistMenu.value = null;
}

async function removeTrackFromPlaylist(playlistId: number, trackId: number) {
  await invoke('remove_track_from_playlist', { playlistId, trackId });
  if (playlistView.value?.id === playlistId) {
    playlistView.value.tracks = playlistView.value.tracks.filter(t => t.id !== trackId);
  }
  await loadPlaylists();
}

function clampMenuPosition(x: number, y: number) {
  const maxX = Math.max(TRACK_MENU_MARGIN, window.innerWidth - TRACK_MENU_WIDTH - TRACK_MENU_MARGIN);
  const maxY = Math.max(TRACK_MENU_MARGIN, window.innerHeight - 280);
  return {
    x: Math.max(TRACK_MENU_MARGIN, Math.min(x, maxX)),
    y: Math.max(TRACK_MENU_MARGIN, Math.min(y, maxY)),
  };
}

function openAddToPlaylistMenuAt(track: Track, x: number, y: number) {
  const pos = clampMenuPosition(x, y);
  homePinnedContextMenu.value = null;
  trackContextMenu.value = null;
  addToPlaylistMenu.value = { track, x: pos.x, y: pos.y };
}

function openTrackContextMenuAt(track: Track, x: number, y: number, playlistId: number | null = null) {
  const pos = clampMenuPosition(x, y);
  homePinnedContextMenu.value = null;
  addToPlaylistMenu.value = null;
  trackContextMenu.value = { track, x: pos.x, y: pos.y, playlistId };
  window.getSelection?.()?.removeAllRanges();
  void nextTick(() => window.getSelection?.()?.removeAllRanges());
}

function openTrackContextMenu(e: MouseEvent, track: Track, playlistId: number | null = null) {
  e.preventDefault();
  e.stopPropagation();
  openTrackContextMenuAt(track, e.clientX, e.clientY, playlistId);
}

function openHomePinnedContextMenuAt(item: HomePinnedPlaylistItem, x: number, y: number) {
  const pos = clampMenuPosition(x, y);
  addToPlaylistMenu.value = null;
  trackContextMenu.value = null;
  homePinnedContextMenu.value = { item, x: pos.x, y: pos.y };
}

function openHomePinnedContextMenu(e: MouseEvent, item: HomePinnedPlaylistItem) {
  e.preventDefault();
  e.stopPropagation();
  openHomePinnedContextMenuAt(item, e.clientX, e.clientY);
}

function clearTrackLongPress() {
  if (trackLongPressTimer) clearTimeout(trackLongPressTimer);
  trackLongPressTimer = null;
  trackLongPressContext = null;
}

function clearHomePinnedLongPress() {
  if (homePinnedLongPressTimer) clearTimeout(homePinnedLongPressTimer);
  homePinnedLongPressTimer = null;
  homePinnedLongPressContext = null;
}

function startTrackRowLongPress(e: TouchEvent, track: Track, playlistId: number | null = null) {
  if (e.touches.length !== 1) return;
  clearTrackLongPress();
  const touch = e.touches[0];
  trackTouchStartX = touch.clientX;
  trackTouchStartY = touch.clientY;
  trackLongPressContext = { track, x: touch.clientX, y: touch.clientY, playlistId };
  trackLongPressTimer = setTimeout(() => {
    const ctx = trackLongPressContext;
    if (!ctx) return;
    suppressTrackRowClickUntil = performance.now() + 500;
    openTrackContextMenuAt(ctx.track, ctx.x, ctx.y, ctx.playlistId);
    clearTrackLongPress();
  }, TRACK_LONG_PRESS_DELAY);
}

function moveTrackRowLongPress(e: TouchEvent) {
  if (!trackLongPressTimer || !e.touches.length) return;
  const touch = e.touches[0];
  const dx = touch.clientX - trackTouchStartX;
  const dy = touch.clientY - trackTouchStartY;
  if (Math.hypot(dx, dy) > TRACK_LONG_PRESS_MOVE_TOLERANCE) clearTrackLongPress();
}

function endTrackRowLongPress() {
  clearTrackLongPress();
}

function startHomePinnedCardLongPress(e: TouchEvent, item: HomePinnedPlaylistItem) {
  if (e.touches.length !== 1) return;
  clearHomePinnedLongPress();
  const touch = e.touches[0];
  homePinnedTouchStartX = touch.clientX;
  homePinnedTouchStartY = touch.clientY;
  homePinnedLongPressContext = { item, x: touch.clientX, y: touch.clientY };
  homePinnedLongPressTimer = setTimeout(() => {
    const ctx = homePinnedLongPressContext;
    if (!ctx) return;
    suppressTrackRowClickUntil = performance.now() + 500;
    openHomePinnedContextMenuAt(ctx.item, ctx.x, ctx.y);
    clearHomePinnedLongPress();
  }, TRACK_LONG_PRESS_DELAY);
}

function moveHomePinnedCardLongPress(e: TouchEvent) {
  if (!homePinnedLongPressTimer || !e.touches.length) return;
  const touch = e.touches[0];
  const dx = touch.clientX - homePinnedTouchStartX;
  const dy = touch.clientY - homePinnedTouchStartY;
  if (Math.hypot(dx, dy) > TRACK_LONG_PRESS_MOVE_TOLERANCE) clearHomePinnedLongPress();
}

function endHomePinnedCardLongPress() {
  clearHomePinnedLongPress();
}

function shouldSuppressTrackRowClick() {
  return performance.now() < suppressTrackRowClickUntil;
}

function playLibraryTrack(index: number) {
  if (index < 0 || shouldSuppressTrackRowClick()) return;
  queueLibraryTracks.value = [...libraryFlatList.value];
  queuedPagedSource.value = {
    kind: 'library',
    query: libraryLoadedQuery.value,
    includeDuplicates: showDuplicateTracks.value,
    hasMore: libraryHasMore.value,
  };
  playTrackFrom('library', index);
}

function playPlaylistTrack(tracks: Track[], index: number) {
  if (index < 0 || shouldSuppressTrackRowClick()) return;
  playFromPlaylist(tracks, index);
}

function playRegularPlaylistTrack(index: number) {
  const view = playlistView.value;
  if (!view || index < 0 || shouldSuppressTrackRowClick()) return;
  playFromPlaylist(view.tracks, index, {
    kind: 'regular-playlist',
    playlistId: view.id,
    name: view.name,
    includeDuplicates: showDuplicateTracks.value,
    hasMore: view.hasMore,
  });
}

function playSmartPlaylistTrack(index: number) {
  const playlist = smartView.value;
  if (!playlist || index < 0 || shouldSuppressTrackRowClick()) return;
  playFromPlaylist(smartViewTracks.value, index, {
    kind: 'smart-playlist',
    playlist,
    includeDuplicates: showDuplicateTracks.value,
    hasMore: smartViewHasMore.value,
  });
}

async function playRecentCard(index: number) {
  if (shouldSuppressTrackRowClick()) return;
  await playTrackFrom('recent', index);
}

async function toggleRecentCardPlayback(index: number, trackId: number) {
  if (shouldSuppressTrackRowClick()) return;
  await toggleCardPlayback('recent', index, trackId);
}

function toggleLikeFromTrackContext() {
  const track = trackContextMenu.value?.track;
  trackContextMenu.value = null;
  if (!track) return;
  toggleLike(track);
}

function editTrackFromTrackContext() {
  const track = trackContextMenu.value?.track;
  trackContextMenu.value = null;
  if (!track) return;
  openEditor(track);
}

function identifyTrackFromTrackContext() {
  const track = trackContextMenu.value?.track;
  trackContextMenu.value = null;
  if (!track) return;
  identifySingle(track);
}

async function shareTrackFromTrackContext() {
  const track = trackContextMenu.value?.track;
  trackContextMenu.value = null;
  if (!track) return;

  const bridge = (window as any).AndroidBridge;
  const dataDir = normalizePath(await ensureLibraryDataDir()).replace(/\/$/, '');
  const absolutePath = track.local_preview_path
    ? normalizePath(track.local_preview_path)
    : `${dataDir}/${normalizePath(track.path).replace(/^\/+/, '')}`;
  const dataDirPrefix = `${dataDir}/`;
  const relativePath = absolutePath.startsWith(dataDirPrefix)
    ? absolutePath.slice(dataDirPrefix.length)
    : normalizePath(track.path).replace(/^\/+/, '');

  if (bridge?.shareFile) {
    bridge.shareFile(relativePath);
    return;
  }

  try {
    await invoke('reveal_track_in_folder', {
      path: absolutePath,
      absolute: true,
    });
  } catch (error) {
    console.error('Failed to reveal shared track:', error);
    alert(`Failed to reveal track in folder.\n${String(error)}`);
  }
}

function addTrackToPlaylistFromTrackContext() {
  const menu = trackContextMenu.value;
  if (!menu) return;
  openAddToPlaylistMenuAt(menu.track, menu.x, menu.y);
}

function removeTrackFromPlaylistFromTrackContext() {
  const menu = trackContextMenu.value;
  trackContextMenu.value = null;
  if (!menu || menu.playlistId === null) return;
  removeTrackFromPlaylist(menu.playlistId, menu.track.id);
}

async function hideTrackAsDuplicateFromTrackContext() {
  const track = trackContextMenu.value?.track;
  trackContextMenu.value = null;
  if (!track) return;
  try {
    await invoke('mark_track_as_duplicate', { id: track.id });
    await loadLibrary();
    await loadRecent();
  } catch (e) {
    console.error('mark_track_as_duplicate failed', e);
  }
}

function buildTrackReplaceQuery(track: Track) {
  const title = track.title?.trim();
  const artist = track.artist?.trim();
  if (artist && title) return `${artist} ${title}`;
  if (title) return title;

  const basename = track.path.replace(/\\/g, '/').split('/').pop() || track.path;
  return basename.replace(/\.[^/.]+$/, '');
}

function clearPendingTrackReplacement(key: string) {
  if (!soulseekPendingTrackReplacement.value[key]) {
    if (trackReplaceApplyingKey.value === key) {
      trackReplaceApplyingKey.value = '';
    }
    return;
  }

  const next = { ...soulseekPendingTrackReplacement.value };
  delete next[key];
  soulseekPendingTrackReplacement.value = next;
  if (trackReplaceApplyingKey.value === key) {
    trackReplaceApplyingKey.value = '';
  }
}

function clearPendingTrackReplacementForTrack(trackId: number) {
  let changed = false;
  const next = { ...soulseekPendingTrackReplacement.value };

  for (const [key, pendingTrackId] of Object.entries(next)) {
    if (pendingTrackId !== trackId) continue;
    delete next[key];
    if (trackReplaceApplyingKey.value === key) {
      trackReplaceApplyingKey.value = '';
    }
    changed = true;
  }

  if (changed) {
    soulseekPendingTrackReplacement.value = next;
  }
}

function closeTrackReplaceDialog() {
  const shouldReturn = activeNav.value === 'track-replace';
  const returnNav = trackReplaceReturnNav.value || 'home';
  const trackId = trackReplaceDialog.value?.track.id;
  if (trackId != null) {
    clearPendingTrackReplacementForTrack(trackId);
  }
  trackReplaceDialog.value = null;
  trackReplaceActionError.value = '';
  resetTrackReplaceSearchState();
  if (shouldReturn) {
    activeNav.value = returnNav;
  }
}

async function openTrackReplaceDialog(track: Track) {
  trackReplaceReturnNav.value = activeNav.value === 'track-replace'
    ? (trackReplaceReturnNav.value || 'home')
    : activeNav.value;
  trackReplaceDialog.value = { track, query: buildTrackReplaceQuery(track) };
  trackReplaceActionError.value = '';
  resetTrackReplaceSearchState();
  activeNav.value = 'track-replace';
  showMobileNav.value = false;
  void nextTick(() => {
    if (contentRef.value) {
      contentRef.value.scrollTop = 0;
    }
  });

  if (trackReplaceDialog.value.query.trim() && soulseekReady.value) {
    await runTrackReplaceSearch();
  }
}

function openTrackReplaceDialogFromTrackContext() {
  const track = trackContextMenu.value?.track;
  trackContextMenu.value = null;
  if (!track) return;
  void openTrackReplaceDialog(track);
}

// ── Smart Playlists ──────────────────────────────────────────────────────────
type SPField = 'any' | 'title' | 'artist' | 'album' | 'album_artist_or_artist' | 'genre' | 'tags' | 'rarity' | 'path' | 'extension' | 'disc_number' | 'track_number' | 'duration_secs' | 'year' | 'play_count' | 'is_liked' | 'date_added' | 'sort';
type SPOp = 'contains' | 'in' | 'eq' | 'gte' | 'lte' | 'is_true' | 'is_false' | 'sort_asc' | 'sort_desc';
type SPSortField = Exclude<SPField, 'any' | 'sort'>;
type SPSortOp = Extract<SPOp, 'sort_asc' | 'sort_desc'>;
interface SPRule { id: string; field: SPField; op: SPOp; value: string; }
interface SmartPlaylist {
  id: string;
  name: string;
  match: 'all' | 'any';
  rules: SPRule[];
  pinned: boolean;
  pinned_at: number | null;
}

const SP_SORT_FIELD_OPTIONS: Array<{ value: SPSortField; label: string }> = [
  { value: 'title', label: 'Title' },
  { value: 'artist', label: 'Artist' },
  { value: 'album', label: 'Album' },
  { value: 'album_artist_or_artist', label: 'Album artist' },
  { value: 'genre', label: 'Genre' },
  { value: 'tags', label: 'Tags' },
  { value: 'rarity', label: 'Rarity' },
  { value: 'path', label: 'Path' },
  { value: 'extension', label: 'Extension' },
  { value: 'disc_number', label: 'Disc #' },
  { value: 'track_number', label: 'Track #' },
  { value: 'duration_secs', label: 'Duration (sec)' },
  { value: 'year', label: 'Year' },
  { value: 'play_count', label: 'Play count' },
  { value: 'is_liked', label: 'Is liked' },
  { value: 'date_added', label: 'Date added' },
];

const SP_SORT_FIELD_SET = new Set<SPSortField>(SP_SORT_FIELD_OPTIONS.map((option) => option.value));
const smartPlaylists = ref<SmartPlaylist[]>([]);
const smartView = ref<SmartPlaylist | null>(null);
const editingSP = ref<SmartPlaylist | null>(null);
const smartViewTracks = ref<Track[]>([]);
const smartViewTotal = ref(0);
const smartViewHasMore = ref(false);
const smartViewLoading = ref(false);
const smartViewLoadingMore = ref(false);
const editingSPTracks = ref<Track[]>([]);
const editingSPTotal = ref(0);
const editingSPLoading = ref(false);
const smartPlaylistPreviews = ref<Record<string, TrackPage>>({});
const smartPlaylistValues = ref<Partial<Record<SPField, string[]>>>({});
const showNewSPInput = ref(false);
const newSPName = ref('');
const playlistTab = ref<'regular' | 'smart'>('regular');
let smartViewRequest = 0;
let editingSPRequest = 0;
let editingSPTimer: ReturnType<typeof setTimeout> | null = null;

async function getSmartPlaylistPage(sp: SmartPlaylist, limit: number, offset = 0) {
  return invoke<TrackPage>('get_smart_playlist_tracks_page', {
    rulesJson: JSON.stringify(sp.rules),
    matchMode: sp.match,
    limit,
    offset,
    includeDuplicates: showDuplicateTracks.value,
  });
}

async function loadSmartViewPage(sp: SmartPlaylist, append = false) {
  const request = ++smartViewRequest;
  const offset = append ? smartViewTracks.value.length : 0;
  if (append) smartViewLoadingMore.value = true;
  else smartViewLoading.value = true;
  try {
    const page = await getSmartPlaylistPage(sp, LIBRARY_TRACK_PAGE_SIZE, offset);
    if (request !== smartViewRequest || smartView.value?.id !== sp.id) return;
    smartViewTracks.value = append ? [...smartViewTracks.value, ...page.tracks] : page.tracks;
    smartViewTotal.value = page.total;
    smartViewHasMore.value = page.has_more;
  } catch (error) {
    if (request === smartViewRequest) console.error('Failed to load flexible playlist:', error);
  } finally {
    if (request === smartViewRequest) {
      smartViewLoading.value = false;
      smartViewLoadingMore.value = false;
    }
  }
}

function loadMoreSmartViewTracks() {
  const sp = smartView.value;
  if (!sp || !smartViewHasMore.value || smartViewLoading.value || smartViewLoadingMore.value) return;
  void loadSmartViewPage(sp, true);
}

async function loadEditingSmartPreview(sp: SmartPlaylist) {
  const request = ++editingSPRequest;
  editingSPLoading.value = true;
  try {
    const page = await getSmartPlaylistPage(sp, 50);
    if (request !== editingSPRequest || editingSP.value?.id !== sp.id) return;
    editingSPTracks.value = page.tracks;
    editingSPTotal.value = page.total;
  } catch (error) {
    if (request === editingSPRequest) console.error('Failed to preview flexible playlist:', error);
  } finally {
    if (request === editingSPRequest) editingSPLoading.value = false;
  }
}

async function loadSmartPlaylistPreviews() {
  if (!smartPlaylists.value.length) {
    smartPlaylistPreviews.value = {};
    return;
  }
  const pages = await Promise.all(smartPlaylists.value.map(async (sp) => [sp.id, await getSmartPlaylistPage(sp, 4)] as const));
  smartPlaylistPreviews.value = Object.fromEntries(pages);
}

async function loadSmartPlaylistValues(field: SPField) {
  if (spFieldType(field) !== 'multiselect') return;
  if (smartPlaylistValues.value[field]) return;
  try {
    const values = await invoke<string[]>('get_smart_playlist_values', { field });
    smartPlaylistValues.value = { ...smartPlaylistValues.value, [field]: values };
  } catch (error) {
    console.error('Failed to load flexible playlist values:', error);
  }
}

function loadSmartPlaylistValuesForRules(sp: SmartPlaylist) {
  for (const rule of sp.rules) void loadSmartPlaylistValues(rule.field);
}

async function loadSmartPlaylists() {
  try {
    // Migrate from localStorage to DB (one-time)
    const old = localStorage.getItem('smart_playlists');
    if (old) {
      const arr: SmartPlaylist[] = JSON.parse(old);
      for (const sp of arr) {
        await invoke('save_smart_playlist', {
          id: sp.id, name: sp.name, matchMode: sp.match,
          rulesJson: JSON.stringify(sp.rules),
        });
      }
      localStorage.removeItem('smart_playlists');
    }
    const rows = await invoke<{
      id: string;
      name: string;
      match_mode: string;
      rules_json: string;
      pinned: boolean;
      pinned_at: number | null;
    }[]>('get_smart_playlists');
    smartPlaylists.value = rows.map(r => ({
      id: r.id, name: r.name, match: r.match_mode as 'all' | 'any',
      rules: JSON.parse(r.rules_json || '[]'),
      pinned: !!r.pinned,
      pinned_at: r.pinned_at ?? null,
    }));
    await loadSmartPlaylistPreviews();
  } catch { smartPlaylists.value = []; }
}
async function createSmartPlaylist() {
  const name = newSPName.value.trim();
  const id = crypto.randomUUID();
  const savedName = await invoke<string>('save_smart_playlist', {
    id, name, matchMode: 'all', rulesJson: '[]',
  });
  const sp: SmartPlaylist = { id, name: savedName, match: 'all', rules: [], pinned: false, pinned_at: null };
  smartPlaylists.value.push(sp);
  newSPName.value = '';
  showNewSPInput.value = false;
  editingSP.value = { ...sp };
}
async function deleteSmartPlaylist(id: string) {
  await invoke('delete_smart_playlist', { id });
  smartPlaylists.value = smartPlaylists.value.filter(sp => sp.id !== id);
  delete smartPlaylistPreviews.value[id];
  if (editingSP.value?.id === id) editingSP.value = null;
  if (smartView.value?.id === id) smartView.value = null;
}

async function toggleSmartPlaylistPinned(sp: SmartPlaylist) {
  await invoke('set_smart_playlist_pinned', { id: sp.id, pinned: !sp.pinned });
  await loadSmartPlaylists();
}
function saveSP() {
  if (!editingSP.value) return;
  const idx = smartPlaylists.value.findIndex(p => p.id === editingSP.value!.id);
  if (idx !== -1) smartPlaylists.value[idx] = { ...editingSP.value };
  const sp = editingSP.value;
  invoke<string>('save_smart_playlist', {
    id: sp.id, name: sp.name, matchMode: sp.match,
    rulesJson: JSON.stringify(sp.rules),
  }).then((savedName) => {
    if (editingSP.value?.id === sp.id) {
      editingSP.value.name = savedName;
    }
    const playlistIndex = smartPlaylists.value.findIndex((playlist) => playlist.id === sp.id);
    if (playlistIndex !== -1) {
      smartPlaylists.value[playlistIndex] = {
        ...smartPlaylists.value[playlistIndex],
        name: savedName,
      };
    }
    void loadSmartPlaylistPreviews();
  });
}

watch(smartView, (sp) => {
  if (!sp) {
    smartViewRequest++;
    smartViewTracks.value = [];
    smartViewTotal.value = 0;
    smartViewHasMore.value = false;
    return;
  }
  void loadSmartViewPage(sp);
});

watch(editingSP, (sp) => {
  if (editingSPTimer) clearTimeout(editingSPTimer);
  if (!sp) {
    editingSPRequest++;
    editingSPTracks.value = [];
    editingSPTotal.value = 0;
    editingSPLoading.value = false;
    return;
  }
  loadSmartPlaylistValuesForRules(sp);
  // Rule controls update on every keystroke. Debouncing avoids a SQLite query
  // for each character while keeping the preview current.
  editingSPTimer = setTimeout(() => void loadEditingSmartPreview(sp), 200);
}, { deep: true });

function addSPRule() {
  if (!editingSP.value) return;
  editingSP.value.rules.push({ id: crypto.randomUUID(), field: 'any', op: 'contains', value: '' });
  saveSP();
}
function removeSPRule(ruleId: string) {
  if (!editingSP.value) return;
  editingSP.value.rules = editingSP.value.rules.filter(r => r.id !== ruleId);
  saveSP();
}
function onSPRuleFieldChange(rule: SPRule) {
  switch (spFieldType(rule.field)) {
    case 'sort': rule.op = 'sort_asc'; rule.value = 'title'; break;
    case 'bool': rule.op = 'is_true'; rule.value = ''; break;
    case 'number': rule.op = 'gte'; rule.value = '0'; break;
    case 'date': rule.op = 'gte'; rule.value = new Date().toISOString().slice(0, 10); break;
    case 'multiselect': rule.op = 'in'; rule.value = '[]'; break;
    default: rule.op = 'contains'; rule.value = '';
  }
  saveSP();
}
function spFieldType(field: SPField): 'text' | 'multiselect' | 'number' | 'bool' | 'date' | 'sort' {
  switch (field) {
    case 'sort': return 'sort';
    case 'is_liked': return 'bool';
    case 'year': case 'play_count': case 'disc_number': case 'track_number': case 'duration_secs': return 'number';
    case 'date_added': return 'date';
    case 'genre': case 'tags': case 'rarity': case 'extension': case 'artist': case 'album': case 'album_artist_or_artist':
      return 'multiselect';
    default: return 'text';
  }
}
function spUniqueValues(field: SPField): string[] {
  return smartPlaylistValues.value[field] ?? [];
}
function spToggleValue(rule: SPRule, v: string) {
  const sel: string[] = JSON.parse(rule.value || '[]');
  const idx = sel.indexOf(v);
  if (idx === -1) sel.push(v); else sel.splice(idx, 1);
  rule.value = JSON.stringify(sel);
  saveSP();
}
function spIsSelected(rule: SPRule, v: string): boolean {
  try { return (JSON.parse(rule.value) as string[]).includes(v); } catch { return false; }
}

function isSPSortField(value: string): value is SPSortField {
  return SP_SORT_FIELD_SET.has(value as SPSortField);
}

function normalizeSPSortField(value: string): SPSortField {
  return isSPSortField(value) ? value : 'title';
}

function normalizeSPSortOp(value: string): SPSortOp {
  return value === 'sort_desc' ? 'sort_desc' : 'sort_asc';
}

type HomePinnedPlaylistItem = {
  key: string;
  kind: 'regular' | 'smart';
  name: string;
  subtitle: string;
  trackCount: number;
  pinnedAt: number;
  playlist: Playlist | SmartPlaylist;
};

type HomePinnedPlaylistSection = HomePinnedPlaylistItem & {
  tracks: Track[];
  previewTracks: Track[];
};

const hasHomePlaylistCandidates = computed(() => playlists.value.length > 0 || smartPlaylists.value.length > 0);
const homePinnedItems = computed<HomePinnedPlaylistItem[]>(() => {
  const regular = playlists.value
    .filter((pl) => pl.pinned)
    .map((pl) => ({
      key: `playlist:${pl.id}`,
      kind: 'regular' as const,
      name: pl.name,
      subtitle: `Playlist · ${pl.track_count} track${pl.track_count !== 1 ? 's' : ''}`,
      trackCount: pl.track_count,
      pinnedAt: pl.pinned_at ?? 0,
      playlist: pl,
    }));
  const smart = smartPlaylists.value
    .filter((sp) => sp.pinned)
    .map((sp) => {
      const trackCount = smartPlaylistPreviews.value[sp.id]?.total ?? 0;
      return {
        key: `smart:${sp.id}`,
        kind: 'smart' as const,
        name: sp.name,
        subtitle: `Flexible playlist · ${trackCount} track${trackCount !== 1 ? 's' : ''}`,
        trackCount,
        pinnedAt: sp.pinned_at ?? 0,
        playlist: sp,
      };
    });

  return [...regular, ...smart].sort((left, right) => {
    if (right.pinnedAt !== left.pinnedAt) return right.pinnedAt - left.pinnedAt;
    return left.name.localeCompare(right.name);
  });
});
const HOME_PLAYLIST_PREVIEW_LIMIT = 4;
const homePinnedSections = computed<HomePinnedPlaylistSection[]>(() => {
  return homePinnedItems.value.map((item) => {
    const tracks = item.kind === 'regular'
      ? homePinnedRegularTracks.value[(item.playlist as Playlist).id] ?? []
      : smartPlaylistPreviews.value[(item.playlist as SmartPlaylist).id]?.tracks ?? [];

    return {
      ...item,
      tracks,
      previewTracks: tracks.slice(0, HOME_PLAYLIST_PREVIEW_LIMIT),
    };
  });
});

function homePinnedCoverStyle(item: HomePinnedPlaylistItem) {
  const [left, right] = placeholderCoverColors(item.key);
  return `background: linear-gradient(135deg, ${left}, ${right})`;
}

async function openHomePinnedItem(item: HomePinnedPlaylistItem) {
  activeNav.value = 'playlists';
  playlistView.value = null;
  editingSP.value = null;

  if (item.kind === 'regular') {
    playlistTab.value = 'regular';
    smartView.value = null;
    await openPlaylist(item.playlist as Playlist);
    return;
  }

  playlistTab.value = 'smart';
  smartView.value = item.playlist as SmartPlaylist;
}

async function playHomePinnedItem(item: HomePinnedPlaylistItem) {
  if (item.trackCount === 0) return;

  if (item.kind === 'regular') {
    const playlist = item.playlist as Playlist;
    const tracks = filterDuplicateTracks(await invoke<Track[]>('get_playlist_tracks', { playlistId: playlist.id }));
    if (tracks.length) playFromPlaylist(tracks, 0);
    return;
  }

  const tracks = (await getSmartPlaylistPage(item.playlist as SmartPlaylist, 100)).tracks;
  if (tracks.length) playFromPlaylist(tracks, 0);
}

async function openHomePinnedCard(item: HomePinnedPlaylistItem) {
  if (shouldSuppressTrackRowClick()) return;
  await openHomePinnedItem(item);
}

async function playHomePinnedCard(item: HomePinnedPlaylistItem) {
  if (shouldSuppressTrackRowClick()) return;
  await playHomePinnedItem(item);
}

async function openHomePinnedFromContextMenu() {
  const item = homePinnedContextMenu.value?.item;
  homePinnedContextMenu.value = null;
  if (!item) return;
  await openHomePinnedItem(item);
}

async function playHomePinnedFromContextMenu() {
  const item = homePinnedContextMenu.value?.item;
  homePinnedContextMenu.value = null;
  if (!item) return;
  await playHomePinnedItem(item);
}

async function unpinHomePinnedFromContextMenu() {
  const item = homePinnedContextMenu.value?.item;
  homePinnedContextMenu.value = null;
  if (!item) return;
  if (item.kind === 'regular') {
    await togglePlaylistPinned(item.playlist as Playlist);
    return;
  }
  await toggleSmartPlaylistPinned(item.playlist as SmartPlaylist);
}

function editHomePinnedFromContextMenu() {
  const item = homePinnedContextMenu.value?.item;
  homePinnedContextMenu.value = null;
  if (!item || item.kind !== 'smart') return;
  activeNav.value = 'playlists';
  playlistTab.value = 'smart';
  smartView.value = null;
  editingSP.value = { ...(item.playlist as SmartPlaylist) };
}

async function deleteHomePinnedFromContextMenu() {
  const item = homePinnedContextMenu.value?.item;
  homePinnedContextMenu.value = null;
  if (!item) return;
  if (item.kind === 'regular') {
    await deletePlaylist((item.playlist as Playlist).id);
    return;
  }
  await deleteSmartPlaylist((item.playlist as SmartPlaylist).id);
}

watch(
  playlists,
  () => {
    loadHomePinnedRegularTracks().catch(() => {
      homePinnedRegularTracks.value = {};
    });
  },
  { immediate: true },
);

watch(showDuplicateTracks, async (show) => {
  if (!show) {
    queue.value = queue.value.filter((track) => !track.is_duplicate);
    queuePlaylistTracks.value = queuePlaylistTracks.value.filter((track) => !track.is_duplicate);
  }

  refillQueue();

  await loadHomePinnedRegularTracks().catch(() => {
    homePinnedRegularTracks.value = {};
  });
  await loadSmartPlaylistPreviews().catch(() => {
    smartPlaylistPreviews.value = {};
  });

  if (playlistView.value) {
    await loadPlaylistPage(playlistView.value.id, playlistView.value.name);
  }
  if (smartView.value) {
    await loadSmartViewPage(smartView.value);
  }

  await loadLibraryPage({
    offset: 0,
    limit: activeNav.value === 'library' ? LIBRARY_TRACK_PAGE_SIZE : HOME_TRACK_PAGE_SIZE,
  });
  if (searchQuery.value.trim()) await loadLocalSearchPage(0);
});
// ────────────────────────────────────────────────────────────────────────────


interface PeerPlayback {
  state: 'playing' | 'paused' | 'stopped' | 'ended';
  hash?: string | null;
  title?: string | null;
  artist?: string | null;
  album?: string | null;
  position?: number;
  duration?: number;
  volume?: number | null;
  beat_seq?: number | null;
}

interface RemoteDeviceStatus {
  version: string;
  device_name?: string | null;
  device_emoji?: string | null;
  playback?: PeerPlayback | null;
}

interface Peer {
  name: string;
  host: string;
  port: number;
  addresses: string[];
  device_name?: string;
  device_emoji?: string;
  playback?: PeerPlayback | null;
}
const peers = ref<Peer[]>([]);
const remoteOutputPeer = ref<Peer | null>(null);
const remoteControlled = ref(false);
let lastRemoteBeatSeq = 0;

interface SyncProgress {
  peer: string;
  device_name?: string | null;
  device_emoji?: string | null;
  phase: string;
  total: number;
  done: number;
  message?: string;
  item_done?: number | null;
  item_total?: number | null;
}
const syncEnabled = ref(false);
const syncProgress = ref<Record<string, SyncProgress>>({});
const syncActive = computed(() => Object.values(syncProgress.value).some((progress) => (
  ['index', 'download', 'reindex', 'merging'].includes(progress.phase)
)));
const peerDeviceNames = ref<Record<string, string>>({});
const deviceEmoji = ref('🎵');
const syncBusy = ref(false);
const syncError = ref('');
const settingsActionError = ref('');
const settingsActionMessage = ref('');
const soulseekStatus = ref<SoulseekStatus | null>(null);
const soulseekResults = ref<SoulseekSearchResult[]>([]);
const soulseekPreviews = ref<Record<string, SoulseekDownloadEvent>>({});
const soulseekVisibleCount = ref(0);
const soulseekSubmittedQuery = ref('');
const soulseekLoading = ref(false);
const soulseekSearching = ref(false);
const soulseekError = ref('');
const soulseekDownloads = ref<Record<string, SoulseekDownloadEvent>>({});
const soulseekCoverHashes = ref<Record<string, string | null>>({});
const libraryDataDir = ref<string | null>(null);
let soulseekSearchSeq = 0;
let unlistenSoulseekDownload: (() => void) | null = null;
let unlistenSoulseekPreview: (() => void) | null = null;
let unlistenDragDrop: (() => void) | null = null;
const SOULSEEK_RESULTS_PAGE_SIZE = 120;
const SOULSEEK_SCROLL_LOAD_THRESHOLD = 180;
const SOULSEEK_PREVIEW_MIN_BUFFER_BYTES = 384 * 1024;
const SOULSEEK_PREVIEW_TARGET_BUFFER_SECONDS = 18;
const SOULSEEK_PREVIEW_DEFAULT_BUFFER_BYTES = 1_500_000;
const soulseekCoverLoading = new Set<string>();
const soulseekPendingPlayback = new Set<string>();
const soulseekPendingPreviewPlayback = new Set<string>();
const soulseekPendingPreviewPromotion = new Set<string>();
const trackReplaceDialog = ref<{ track: Track; query: string } | null>(null);
const trackReplaceReturnNav = ref('home');
const trackReplaceResults = ref<SoulseekSearchResult[]>([]);
const trackReplaceSubmittedQuery = ref('');
const trackReplaceVisibleCount = ref(0);
const trackReplaceLoading = ref(false);
const trackReplaceSearching = ref(false);
const trackReplaceError = ref('');
const trackReplaceActionError = ref('');
const trackReplaceApplyingKey = ref('');
const soulseekPendingTrackReplacement = ref<Record<string, number>>({});
let trackReplaceSearchSeq = 0;

const tracksByHash = computed<Record<string, Track>>(() => {
  const index: Record<string, Track> = {};
  for (const track of libraryTracks.value) {
    if (track.file_hash) index[track.file_hash] = track;
  }
  return index;
});

const remoteOutputDevices = computed(() => peers.value);

function openDeviceSettings() { openSettings('devices'); }
function openSoulseekSettings() { openSettings('soulseek'); }
function onDeviceSettingsSaved(config: DeviceSettings) {
  deviceEmoji.value = config.emoji;
}

const soulseekReady = computed(() => !!soulseekStatus.value?.enabled && !!soulseekStatus.value?.configured);

function soulseekResultKey(username: string, filename: string) {
  return `${username}\u0000${filename}`;
}

function soulseekCoverKey(username: string, filename: string) {
  return soulseekResultKey(username, filename);
}

function soulseekCoverHash(result: SoulseekSearchResult) {
  if (!result.coverFilename) return null;
  return soulseekCoverHashes.value[soulseekCoverKey(result.username, result.coverFilename)] || null;
}

function soulseekCoverUrl(result: SoulseekSearchResult) {
  return coverUrl(soulseekCoverHash(result), 'small');
}

function soulseekTrackIdFromParts(username: string, filename: string) {
  const key = soulseekResultKey(username, filename);
  let hash = 0;
  for (let index = 0; index < key.length; index += 1) {
    hash = ((hash << 5) - hash) + key.charCodeAt(index);
    hash |= 0;
  }
  return -1 - Math.abs(hash);
}

function soulseekTrackId(result: Pick<SoulseekSearchResult, 'username' | 'filename'>) {
  return soulseekTrackIdFromParts(result.username, result.filename);
}

function soulseekDownloadState(result: SoulseekSearchResult) {
  return soulseekDownloads.value[soulseekResultKey(result.username, result.filename)] || null;
}

function soulseekPreviewState(result: SoulseekSearchResult) {
  return soulseekPreviews.value[soulseekResultKey(result.username, result.filename)] || null;
}

function soulseekTransferFailed(state: string | null | undefined) {
  return state === 'failed' || state === 'timed_out' || state === 'cancelled';
}

function soulseekEffectivePreviewState(result: SoulseekSearchResult) {
  const downloadState = soulseekDownloadState(result);
  if (downloadState?.state === 'completed' && downloadState.localPath) {
    return downloadState;
  }

  const previewState = soulseekPreviewState(result);
  if (previewState && !soulseekTransferFailed(previewState.state)) {
    return previewState;
  }
  if (downloadState && !soulseekTransferFailed(downloadState.state)) {
    return downloadState;
  }

  return previewState || downloadState;
}

function soulseekPreviewThresholdBytes(result: SoulseekSearchResult) {
  const bytesPerSecond = result.bitrate != null
    ? Math.round((result.bitrate * 1000) / 8)
    : null;
  const target = bytesPerSecond != null
    ? bytesPerSecond * SOULSEEK_PREVIEW_TARGET_BUFFER_SECONDS
    : SOULSEEK_PREVIEW_DEFAULT_BUFFER_BYTES;

  return Math.min(result.size, Math.max(SOULSEEK_PREVIEW_MIN_BUFFER_BYTES, target));
}

function soulseekPreviewCanStart(result: SoulseekSearchResult, state: SoulseekDownloadEvent | null | undefined) {
  if (!state?.localPath) return false;
  if (state.state === 'completed') return true;
  if (state.state !== 'progress') return false;
  return (state.bytesDownloaded ?? 0) >= soulseekPreviewThresholdBytes(result);
}

function soulseekPreviewActionLabel(result: SoulseekSearchResult) {
  const state = soulseekEffectivePreviewState(result);
  if (isCurrentTrack(soulseekTrackId(result)) && !!nowPlaying.value?.soulseek_preview && isPlaying.value) {
    return 'Previewing…';
  }

  switch (state?.state) {
    case 'starting':
    case 'queued_local':
    case 'queued_remote':
      return 'Buffering…';
    case 'progress':
      return soulseekPreviewCanStart(result, state) ? 'Starting…' : 'Buffering…';
    case 'completed':
      return 'Replay';
    case 'failed':
    case 'timed_out':
    case 'cancelled':
      return 'Retry preview';
    default:
      return 'Preview';
  }
}

function soulseekPreviewBuffering(result: SoulseekSearchResult) {
  const key = soulseekResultKey(result.username, result.filename);
  const state = soulseekEffectivePreviewState(result);
  if (!soulseekPendingPreviewPlayback.has(key) || !state) {
    return false;
  }

  return state.state !== 'completed' && !soulseekTransferFailed(state.state);
}

function soulseekTransferError(state: SoulseekDownloadEvent | null | undefined) {
  if (!state?.error || state.state === 'cancelled') {
    return null;
  }

  return state.error;
}

async function cancelSoulseekPreviewBuffering(result: SoulseekSearchResult) {
  const key = soulseekResultKey(result.username, result.filename);
  soulseekPendingPreviewPlayback.delete(key);

  const previewState = soulseekPreviewState(result);
  if (!previewState || previewState.state === 'completed' || soulseekTransferFailed(previewState.state)) {
    return;
  }

  try {
    const cancelled = await invoke<boolean>('soulseek_cancel_preview', {
      request: {
        username: result.username,
        filename: result.filename,
      },
    });

    if (!cancelled) {
      return;
    }

    soulseekPreviews.value = {
      ...soulseekPreviews.value,
      [key]: {
        ...previewState,
        state: 'cancelled',
        speedBytesPerSec: null,
        queuePosition: null,
        error: null,
      },
    };
  } catch (error) {
    console.error('Failed to cancel Soulseek preview buffering:', error);
  }
}

async function stopActiveSoulseekPreview(result: SoulseekSearchResult) {
  const previewTrackId = soulseekTrackId(result);
  const currentTrack = nowPlaying.value;
  if (!currentTrack || currentTrack.id !== previewTrackId || !currentTrack.soulseek_preview) {
    return false;
  }

  const previewState = soulseekPreviewState(result);
  if (previewState && previewState.state !== 'completed' && !soulseekTransferFailed(previewState.state)) {
    await cancelSoulseekPreviewBuffering(result);
  } else {
    soulseekPendingPreviewPlayback.delete(soulseekResultKey(result.username, result.filename));
  }

  try {
    await stopCurrentOutput();
  } catch (error) {
    console.error('Failed to stop Soulseek preview playback:', error);
  }

  nowPlaying.value = null;
  queueSoulseekTracks.value = [];
  queue.value = [];
  queueSource.value = 'library';
  queueSourceIndex.value = 0;
  duration.value = 0;
  currentTime.value = 0;
  isPlaying.value = false;

  stopTicker();
  syncAndroid();
  return true;
}

function updateSoulseekPreviewTrackState(username: string, filename: string, previewGrowing: boolean) {
  const previewTrackId = soulseekTrackIdFromParts(username, filename);
  const updateTrack = (track: Track) => (
    track.id === previewTrackId && track.soulseek_preview
      ? { ...track, preview_growing: previewGrowing }
      : track
  );

  if (nowPlaying.value?.id === previewTrackId && nowPlaying.value.soulseek_preview) {
    nowPlaying.value = { ...nowPlaying.value, preview_growing: previewGrowing };
  }

  queueSoulseekTracks.value = queueSoulseekTracks.value.map(updateTrack);
  queue.value = queue.value.map(updateTrack);
}

function soulseekDownloadActionLabel(result: SoulseekSearchResult) {
  const previewState = soulseekPreviewState(result);
  const state = soulseekDownloadState(result)?.state;
  if (
    previewState?.state === 'completed'
    && !!previewState.localPath
    && state !== 'completed'
    && state !== 'promoting'
  ) {
    return 'Save';
  }
  switch (state) {
    case 'starting':
      return 'Starting…';
    case 'promoting':
      return 'Saving…';
    case 'queued_local':
    case 'queued_remote':
      return 'Queued';
    case 'progress':
      return 'Downloading…';
    case 'completed':
      return 'Saved';
    case 'failed':
    case 'timed_out':
    case 'cancelled':
      return 'Retry';
    default:
      return 'Download';
  }
}

function soulseekDownloadBusy(result: SoulseekSearchResult) {
  const state = soulseekDownloadState(result)?.state;
  return state === 'starting' || state === 'queued_local' || state === 'queued_remote' || state === 'progress' || state === 'promoting';
}

function nativeSampleRateLabel() {
  const formatted = formatSampleRate(nativeSampleRate.value);
  return formatted ? `Native rate (${formatted})` : 'Native rate';
}

async function loadSoulseekStatus() {
  try {
    soulseekStatus.value = await invoke<SoulseekStatus>('soulseek_get_status');
  } catch (e) {
    console.error('Failed to load Soulseek status:', e);
    soulseekStatus.value = null;
  }
}

async function ensureLibraryDataDir() {
  if (!libraryDataDir.value) {
    libraryDataDir.value = await invoke<string>('get_data_dir');
  }
  return libraryDataDir.value;
}

async function soulseekLocalRelativePath(localPath: string) {
  const dataDir = normalizePath(await ensureLibraryDataDir()).replace(/\/$/, '');
  const normalizedLocalPath = normalizePath(localPath);
  const prefix = `${dataDir}/`;
  if (normalizedLocalPath.startsWith(prefix)) {
    return normalizedLocalPath.slice(prefix.length);
  }
  return null;
}

function makeSoulseekTrack(result: SoulseekSearchResult, relativePath: string): Track {
  return {
    id: soulseekTrackId(result),
    path: relativePath,
    title: result.basename,
    artist: result.username,
    album: null,
    album_artist: null,
    disc_number: null,
    album_artist_or_artist: null,
    cover_hash: soulseekCoverHash(result),
    track_number: null,
    duration_secs: result.duration,
    file_hash: null,
    rarity: null,
    manually_edited: false,
    is_liked: false,
    play_count: 0,
    year: null,
    genre: null,
    tags: null,
    date_added: null,
    is_duplicate: false,
    local_preview_path: null,
    preview_growing: false,
    soulseek_preview: false,
    soulseek_username: result.username,
    soulseek_filename: result.filename,
    soulseek_size: result.size,
  };
}

function makeSoulseekPreviewTrack(result: SoulseekSearchResult, localPath: string, previewGrowing: boolean): Track {
  return {
    ...makeSoulseekTrack(result, `Soulseek Preview/${result.basename}`),
    local_preview_path: localPath,
    preview_growing: previewGrowing,
    soulseek_preview: true,
    soulseek_username: result.username,
    soulseek_filename: result.filename,
    soulseek_size: result.size,
  };
}

async function playDownloadedSoulseekResult(result: SoulseekSearchResult, localPath: string) {
  const relativePath = await soulseekLocalRelativePath(localPath);
  if (!relativePath) {
    throw new Error('Downloaded Soulseek file is outside the library data directory');
  }

  if (remoteOutputPeer.value) {
    try {
      await invoke('remote_playback_pause', remotePeerArgs(remoteOutputPeer.value));
    } catch (_) {}
    remoteOutputPeer.value = null;
  }

  bumpPlaybackTransitionSequence();
  const contentScrollTop = contentRef.value?.scrollTop ?? null;
  const track = makeSoulseekTrack(result, relativePath);
  queueSoulseekTracks.value = [track];
  queueSource.value = 'soulseek';
  queueSourceIndex.value = 0;
  nowPlaying.value = track;
  duration.value = track.duration_secs || 0;
  currentTime.value = 0;
  queue.value = [];
  refillQueue();
  await playTrackLocally(track, true, 0);
  isPlaying.value = true;
  await refreshPlaybackState();
  startTicker();
  syncAndroid();
  restoreContentScroll(contentScrollTop);
}

async function playPreviewedSoulseekResult(result: SoulseekSearchResult, localPath: string, previewGrowing: boolean) {
  if (remoteOutputPeer.value) {
    try {
      await invoke('remote_playback_pause', remotePeerArgs(remoteOutputPeer.value));
    } catch (_) {}
    remoteOutputPeer.value = null;
  }

  bumpPlaybackTransitionSequence();
  const contentScrollTop = contentRef.value?.scrollTop ?? null;
  const track = makeSoulseekPreviewTrack(result, localPath, previewGrowing);
  queueSoulseekTracks.value = [track];
  queueSource.value = 'soulseek';
  queueSourceIndex.value = 0;
  nowPlaying.value = track;
  duration.value = track.duration_secs || 0;
  currentTime.value = 0;
  queue.value = [];
  refillQueue();
  await playTrackLocally(track, true, 0);
  isPlaying.value = true;
  await refreshPlaybackState();
  startTicker();
  syncAndroid();
  restoreContentScroll(contentScrollTop);
}

function replaceSoulseekTrackInPlaybackState(previousTrackId: number, nextTrack: Track) {
  const replaceTrack = (track: Track) => (
    track.id === previousTrackId
      ? { ...nextTrack }
      : track
  );

  if (nowPlaying.value?.id === previousTrackId) {
    nowPlaying.value = { ...nextTrack };
  }

  queueSoulseekTracks.value = queueSoulseekTracks.value.map(replaceTrack);
  queue.value = queue.value.map(replaceTrack);
}

async function maybeReplaceActiveSoulseekPreviewWithLibraryTrack(
  username: string,
  filename: string,
  localPath: string,
) {
  const previewTrackId = soulseekTrackIdFromParts(username, filename);
  const currentTrack = nowPlaying.value;
  if (!currentTrack || currentTrack.id !== previewTrackId || !currentTrack.soulseek_preview) {
    return;
  }

  const relativePath = await soulseekLocalRelativePath(localPath);
  if (!relativePath) {
    return;
  }

  const indexedTrack = await invoke<Track | null>('index_track_by_path', { path: relativePath });
  if (!indexedTrack) {
    return;
  }

  // The preview file is renamed into the library, so the active decoder can
  // continue reading the same file handle without an audible restart.
  replaceSoulseekTrackInPlaybackState(previewTrackId, indexedTrack);

  duration.value = indexedTrack.duration_secs || duration.value;
  syncAndroid();
}

async function maybeStartSoulseekPreviewPlayback(result: SoulseekSearchResult) {
  const key = soulseekResultKey(result.username, result.filename);
  const state = soulseekEffectivePreviewState(result);
  if (!soulseekPendingPreviewPlayback.has(key) || !state?.localPath) {
    return;
  }

  const downloadState = soulseekDownloadState(result);
  if (downloadState?.state === 'completed' && downloadState.localPath === state.localPath) {
    soulseekPendingPreviewPlayback.delete(key);
    await playDownloadedSoulseekResult(result, state.localPath);
    return;
  }

  if (!soulseekPreviewCanStart(result, state)) {
    return;
  }

  soulseekPendingPreviewPlayback.delete(key);
  await playPreviewedSoulseekResult(result, state.localPath, state.state !== 'completed');
}

async function promoteSoulseekPreviewToLibrary(
  result: Pick<SoulseekSearchResult, 'username' | 'filename' | 'basename' | 'size' | 'coverFilename'>,
  localPath: string,
) {
  const key = soulseekResultKey(result.username, result.filename);
  if (soulseekDownloadState(result as SoulseekSearchResult)?.state === 'promoting') {
    return;
  }

  soulseekPendingPreviewPromotion.add(key);
  const currentState = soulseekPreviewState(result as SoulseekSearchResult) || soulseekDownloadState(result as SoulseekSearchResult);
  soulseekDownloads.value = {
    ...soulseekDownloads.value,
    [key]: {
      transferId: currentState?.transferId || '',
      username: result.username,
      filename: result.filename,
      basename: result.basename,
      state: 'promoting',
      bytesDownloaded: result.size,
      totalBytes: result.size,
      speedBytesPerSec: null,
      queuePosition: null,
      localPath,
      error: null,
    },
  };

  let promotedPath: string | null = null;
  try {
    promotedPath = await invoke<string>('soulseek_promote_preview', {
      request: {
        localPath,
        coverFilename: result.coverFilename,
      },
    });

    soulseekDownloads.value = {
      ...soulseekDownloads.value,
      [key]: {
        transferId: currentState?.transferId || '',
        username: result.username,
        filename: result.filename,
        basename: result.basename,
        state: 'completed',
        bytesDownloaded: result.size,
        totalBytes: result.size,
        speedBytesPerSec: null,
        queuePosition: null,
        localPath: promotedPath,
        error: null,
      },
    };
    await maybeReplaceActiveSoulseekPreviewWithLibraryTrack(result.username, result.filename, promotedPath);
    if (soulseekPreviews.value[key]) {
      soulseekPreviews.value = {
        ...soulseekPreviews.value,
        [key]: {
          ...soulseekPreviews.value[key],
          state: 'completed',
          bytesDownloaded: result.size,
          totalBytes: result.size,
          speedBytesPerSec: null,
          queuePosition: null,
          localPath: promotedPath,
          error: null,
        },
      };
    }
  } catch (e: any) {
    soulseekDownloads.value = {
      ...soulseekDownloads.value,
      [key]: {
        transferId: currentState?.transferId || '',
        username: result.username,
        filename: result.filename,
        basename: result.basename,
        state: 'failed',
        bytesDownloaded: null,
        totalBytes: result.size,
        speedBytesPerSec: null,
        queuePosition: null,
        localPath,
        error: String(e ?? 'Save failed'),
      },
    };
    throw e;
  } finally {
    soulseekPendingPreviewPromotion.delete(key);
  }

  const pendingTrackId = soulseekPendingTrackReplacement.value[key];
  if (pendingTrackId != null && promotedPath) {
    await performTrackReplacement(pendingTrackId, result, promotedPath);
  }
}

type SoulseekSearchTarget = 'page' | 'replace';

function isCurrentSoulseekSearchRequest(target: SoulseekSearchTarget, requestId: number) {
  return target === 'page'
    ? requestId === soulseekSearchSeq
    : requestId === trackReplaceSearchSeq;
}

async function runSoulseekSearch(query: string, requestId: number, target: SoulseekSearchTarget) {
  try {
    const results = await invoke<SoulseekSearchResult[]>('soulseek_search', { query });
    if (!isCurrentSoulseekSearchRequest(target, requestId)) return;

    if (target === 'page') {
      soulseekResults.value = results;
      soulseekVisibleCount.value = Math.min(SOULSEEK_RESULTS_PAGE_SIZE, results.length);
      soulseekError.value = '';
      void fetchSoulseekCovers(results.slice(0, soulseekVisibleCount.value), requestId, target);
    } else {
      trackReplaceResults.value = results;
      trackReplaceVisibleCount.value = Math.min(SOULSEEK_RESULTS_PAGE_SIZE, results.length);
      trackReplaceError.value = '';
      void fetchSoulseekCovers(results.slice(0, trackReplaceVisibleCount.value), requestId, target);
    }
  } catch (e: any) {
    if (!isCurrentSoulseekSearchRequest(target, requestId)) return;

    if (target === 'page') {
      soulseekResults.value = [];
      soulseekVisibleCount.value = 0;
      soulseekError.value = String(e ?? 'Soulseek search failed');
    } else {
      trackReplaceResults.value = [];
      trackReplaceVisibleCount.value = 0;
      trackReplaceError.value = String(e ?? 'Soulseek search failed');
    }
  } finally {
    if (!isCurrentSoulseekSearchRequest(target, requestId)) {
      return;
    }

    if (target === 'page') {
      soulseekLoading.value = false;
      soulseekSearching.value = false;
    } else {
      trackReplaceLoading.value = false;
      trackReplaceSearching.value = false;
    }
  }
}

function resetSoulseekSearchState(clearSubmittedQuery = true) {
  soulseekSearchSeq += 1;
  soulseekLoading.value = false;
  soulseekSearching.value = false;
  soulseekError.value = '';
  soulseekResults.value = [];
  soulseekVisibleCount.value = 0;
  if (clearSubmittedQuery) {
    soulseekSubmittedQuery.value = '';
  }
}

function resetTrackReplaceSearchState(clearSubmittedQuery = true) {
  trackReplaceSearchSeq += 1;
  trackReplaceLoading.value = false;
  trackReplaceSearching.value = false;
  trackReplaceError.value = '';
  trackReplaceResults.value = [];
  trackReplaceVisibleCount.value = 0;
  if (clearSubmittedQuery) {
    trackReplaceSubmittedQuery.value = '';
  }
}

async function fetchSoulseekCovers(
  results: SoulseekSearchResult[],
  requestId: number,
  target: SoulseekSearchTarget = 'page',
) {
  const uniqueResults = new Map<string, SoulseekSearchResult>();

  for (const result of results) {
    if (!result.coverFilename || !result.coverSize) continue;
    const key = soulseekCoverKey(result.username, result.coverFilename);
    if (soulseekCoverHashes.value[key] !== undefined || soulseekCoverLoading.has(key)) continue;
    uniqueResults.set(key, result);
  }

  for (const [key, result] of uniqueResults) {
    if (!isCurrentSoulseekSearchRequest(target, requestId)) return;

    soulseekCoverLoading.add(key);
    try {
      const hash = await invoke<string | null>('soulseek_fetch_cover', {
        request: {
          username: result.username,
          coverFilename: result.coverFilename,
          coverSize: result.coverSize,
        },
      });
      soulseekCoverHashes.value = {
        ...soulseekCoverHashes.value,
        [key]: hash,
      };
    } catch (error) {
      console.error('Failed to fetch Soulseek cover preview:', error);
      soulseekCoverHashes.value = {
        ...soulseekCoverHashes.value,
        [key]: null,
      };
    } finally {
      soulseekCoverLoading.delete(key);
    }
  }
}

function loadMoreSoulseekResults() {
  if (soulseekVisibleCount.value >= soulseekResults.value.length) return;

  const previousCount = soulseekVisibleCount.value;
  const nextCount = Math.min(soulseekResults.value.length, previousCount + SOULSEEK_RESULTS_PAGE_SIZE);
  soulseekVisibleCount.value = nextCount;

  if (nextCount > previousCount) {
    void fetchSoulseekCovers(soulseekResults.value.slice(previousCount, nextCount), soulseekSearchSeq, 'page');
    void nextTick(() => {
      maybeLoadMoreSoulseekResults();
    });
  }
}

function loadMoreTrackReplaceResults() {
  if (trackReplaceVisibleCount.value >= trackReplaceResults.value.length) return;

  const previousCount = trackReplaceVisibleCount.value;
  const nextCount = Math.min(trackReplaceResults.value.length, previousCount + SOULSEEK_RESULTS_PAGE_SIZE);
  trackReplaceVisibleCount.value = nextCount;

  if (nextCount > previousCount) {
    void fetchSoulseekCovers(trackReplaceResults.value.slice(previousCount, nextCount), trackReplaceSearchSeq, 'replace');
  }
}

function maybeLoadMoreSoulseekResults() {
  const content = contentRef.value;
  if (!content) return;

  const distanceToBottom = content.scrollHeight - (content.scrollTop + content.clientHeight);
  if (distanceToBottom > SOULSEEK_SCROLL_LOAD_THRESHOLD) return;

  if (activeNav.value === 'library') {
    loadMoreLibraryTracks();
    return;
  }

  if (activeNav.value === 'playlists' && playlistTab.value === 'regular' && playlistView.value) {
    loadMorePlaylistTracks();
    return;
  }

  if (activeNav.value === 'playlists' && playlistTab.value === 'smart' && smartView.value) {
    loadMoreSmartViewTracks();
    return;
  }

  if (activeNav.value !== 'search' && activeNav.value !== 'track-replace') return;

  if (activeNav.value === 'search') {
    if (
      soulseekLoading.value
      || soulseekSearching.value
      || soulseekQueryDirty.value
      || soulseekVisibleCount.value >= soulseekResults.value.length
    ) {
      return;
    }
  } else if (
    !trackReplaceDialog.value
    || trackReplaceLoading.value
    || trackReplaceSearching.value
    || trackReplaceQueryDirty.value
    || trackReplaceVisibleCount.value >= trackReplaceResults.value.length
  ) {
    return;
  }

  if (activeNav.value === 'search') {
    loadMoreSoulseekResults();
  } else {
    loadMoreTrackReplaceResults();
  }
}

async function runExplicitSoulseekSearch() {
  const trimmed = searchQuery.value.trim();
  const enabled = soulseekStatus.value?.enabled;
  const configured = soulseekStatus.value?.configured;

  if (activeNav.value !== 'search' || !trimmed || !enabled || !configured || soulseekLoading.value) {
    return;
  }

  soulseekSearchSeq += 1;
  const requestId = soulseekSearchSeq;
  soulseekSubmittedQuery.value = trimmed;
  soulseekSearching.value = true;
  soulseekLoading.value = true;
  soulseekError.value = '';
  soulseekResults.value = [];
  soulseekVisibleCount.value = 0;
  await runSoulseekSearch(trimmed, requestId, 'page');
}

async function runTrackReplaceSearch() {
  const query = trackReplaceDialog.value?.query ?? '';
  const trimmed = query.trim();

  if (!trackReplaceDialog.value || !trimmed || !soulseekReady.value || trackReplaceLoading.value) {
    return;
  }

  trackReplaceSearchSeq += 1;
  const requestId = trackReplaceSearchSeq;
  trackReplaceSubmittedQuery.value = trimmed;
  trackReplaceSearching.value = true;
  trackReplaceLoading.value = true;
  trackReplaceError.value = '';
  trackReplaceResults.value = [];
  trackReplaceVisibleCount.value = 0;
  await runSoulseekSearch(trimmed, requestId, 'replace');
}

function handleSearchInput() {
  const trimmed = searchQuery.value.trim();
  if (localSearchTimer) clearTimeout(localSearchTimer);
  if (!trimmed) {
    localSearchTracks.value = [];
    localSearchTotal.value = 0;
    localSearchOffset.value = 0;
    localSearchHasMore.value = false;
    localSearchLoading.value = false;
  } else {
    localSearchLoading.value = true;
    localSearchTimer = setTimeout(() => {
      localSearchTimer = null;
      void loadLocalSearchPage(0);
    }, 250);
  }

  if (!trimmed) {
    resetSoulseekSearchState();
    return;
  }

  if (trimmed === soulseekSubmittedQuery.value) {
    return;
  }

  resetSoulseekSearchState();
}

function handleTrackReplaceInput() {
  const trimmed = trackReplaceDialog.value?.query.trim() ?? '';
  if (!trimmed) {
    resetTrackReplaceSearchState();
    return;
  }

  if (trimmed === trackReplaceSubmittedQuery.value) {
    return;
  }

  resetTrackReplaceSearchState();
}

watch(
  [activeNav, () => soulseekStatus.value?.enabled, () => soulseekStatus.value?.configured],
  ([nav, enabled, configured]) => {
    if (nav !== 'search' || !enabled || !configured) {
      resetSoulseekSearchState();
    }
  }
);

async function downloadSoulseekResult(result: SoulseekSearchResult) {
  const key = soulseekResultKey(result.username, result.filename);
  const currentDownload = soulseekDownloadState(result);
  if (currentDownload?.state === 'completed' || currentDownload?.state === 'promoting') return;
  if (soulseekDownloadBusy(result)) return;

  const previewState = soulseekPreviewState(result);
  if (previewState && !soulseekTransferFailed(previewState.state)) {
    if (previewState.state === 'completed' && previewState.localPath) {
      void promoteSoulseekPreviewToLibrary(result, previewState.localPath).catch((error) => {
        console.error('Failed to promote Soulseek preview to library:', error);
      });
      return;
    }

    soulseekPendingPreviewPromotion.add(key);
    soulseekDownloads.value = {
      ...soulseekDownloads.value,
      [key]: {
        ...previewState,
        totalBytes: previewState.totalBytes ?? result.size,
        error: null,
      },
    };
    return;
  }

  soulseekDownloads.value = {
    ...soulseekDownloads.value,
    [key]: {
      transferId: soulseekDownloads.value[key]?.transferId || '',
      username: result.username,
      filename: result.filename,
      basename: result.basename,
      state: 'starting',
      bytesDownloaded: 0,
      totalBytes: result.size,
      speedBytesPerSec: null,
      queuePosition: null,
      localPath: null,
      error: null,
    },
  };

  try {
    const transferId = await invoke<string>('soulseek_download', {
      request: {
        username: result.username,
        filename: result.filename,
        coverFilename: result.coverFilename,
        coverSize: result.coverSize,
        size: result.size,
      },
    });
    const current = soulseekDownloads.value[key];
    if (!current) return;
    soulseekDownloads.value = {
      ...soulseekDownloads.value,
      [key]: {
        ...current,
        transferId,
      },
    };
  } catch (e: any) {
    soulseekDownloads.value = {
      ...soulseekDownloads.value,
      [key]: {
        transferId: '',
        username: result.username,
        filename: result.filename,
        basename: result.basename,
        state: 'failed',
        bytesDownloaded: null,
        totalBytes: result.size,
        speedBytesPerSec: null,
        queuePosition: null,
        localPath: null,
        error: String(e ?? 'Download failed'),
      },
    };
  }
}

async function startSoulseekPreview(result: SoulseekSearchResult) {
  const key = soulseekResultKey(result.username, result.filename);
  const downloadState = soulseekDownloadState(result);
  if (downloadState?.state === 'completed' && downloadState.localPath) {
    await playDownloadedSoulseekResult(result, downloadState.localPath);
    return;
  }

  const currentState = soulseekEffectivePreviewState(result);

  if (currentState?.localPath && soulseekPreviewCanStart(result, currentState)) {
    await playPreviewedSoulseekResult(result, currentState.localPath, currentState.state !== 'completed');
    return;
  }

  soulseekPendingPreviewPlayback.clear();
  soulseekPendingPreviewPlayback.add(key);

  if (currentState && !soulseekTransferFailed(currentState.state)) {
    return;
  }

  soulseekPreviews.value = {
    ...soulseekPreviews.value,
    [key]: {
      transferId: soulseekPreviewState(result)?.transferId || '',
      username: result.username,
      filename: result.filename,
      basename: result.basename,
      state: 'starting',
      bytesDownloaded: 0,
      totalBytes: result.size,
      speedBytesPerSec: null,
      queuePosition: null,
      localPath: currentState?.localPath || null,
      error: null,
    },
  };

  try {
    const transferId = await invoke<string>('soulseek_preview', {
      request: {
        username: result.username,
        filename: result.filename,
        coverFilename: result.coverFilename,
        coverSize: result.coverSize,
        size: result.size,
      },
    });
    const current = soulseekPreviews.value[key];
    if (!current) return;
    soulseekPreviews.value = {
      ...soulseekPreviews.value,
      [key]: {
        ...current,
        transferId,
      },
    };
  } catch (e: any) {
    soulseekPendingPreviewPlayback.delete(key);
    soulseekPreviews.value = {
      ...soulseekPreviews.value,
      [key]: {
        transferId: '',
        username: result.username,
        filename: result.filename,
        basename: result.basename,
        state: 'failed',
        bytesDownloaded: null,
        totalBytes: result.size,
        speedBytesPerSec: null,
        queuePosition: null,
        localPath: currentState?.localPath || null,
        error: String(e ?? 'Preview failed'),
      },
    };
  }
}

async function activateSoulseekResult(result: SoulseekSearchResult) {
  if (shouldSuppressTrackRowClick()) return;

  if (await stopActiveSoulseekPreview(result)) {
    return;
  }

  if (soulseekPreviewBuffering(result)) {
    await cancelSoulseekPreviewBuffering(result);
    return;
  }

  const currentState = soulseekDownloadState(result);

  if (currentState?.state === 'completed' && currentState.localPath) {
    await playDownloadedSoulseekResult(result, currentState.localPath);
    return;
  }

  await startSoulseekPreview(result);
}

function findSoulseekResultByKey(key: string) {
  return soulseekResults.value.find((result) => soulseekResultKey(result.username, result.filename) === key)
    || trackReplaceResults.value.find((result) => soulseekResultKey(result.username, result.filename) === key)
    || null;
}

function replaceSourceLocalPath(result: SoulseekSearchResult) {
  const downloadState = soulseekDownloadState(result);
  if (downloadState?.state === 'completed' && downloadState.localPath) {
    return downloadState.localPath;
  }

  const previewState = soulseekPreviewState(result);
  if (previewState?.state === 'completed' && previewState.localPath) {
    return previewState.localPath;
  }

  return null;
}

async function performTrackReplacement(
  trackId: number,
  result: Pick<SoulseekSearchResult, 'username' | 'filename'>,
  localPath: string,
) {
  const key = soulseekResultKey(result.username, result.filename);
  trackReplaceApplyingKey.value = key;
  trackReplaceActionError.value = '';

  try {
    const replacedTrack = await invoke<Track>('replace_track_with_file', { id: trackId, localPath });
    const dataDir = normalizePath(await ensureLibraryDataDir()).replace(/\/$/, '');
    const replacementLocalPath = `${dataDir}/${normalizePath(replacedTrack.path)}`;
    const knownResult = findSoulseekResultByKey(key);
    const totalBytes = soulseekDownloads.value[key]?.totalBytes ?? soulseekPreviews.value[key]?.totalBytes ?? null;

    soulseekDownloads.value = {
      ...soulseekDownloads.value,
      [key]: {
        transferId: soulseekDownloads.value[key]?.transferId || soulseekPreviews.value[key]?.transferId || '',
        username: result.username,
        filename: result.filename,
        basename: knownResult?.basename || soulseekDownloads.value[key]?.basename || soulseekPreviews.value[key]?.basename || result.filename,
        state: 'completed',
        bytesDownloaded: totalBytes,
        totalBytes,
        speedBytesPerSec: null,
        queuePosition: null,
        localPath: replacementLocalPath,
        error: null,
      },
    };

    if (soulseekPreviews.value[key]) {
      soulseekPreviews.value = {
        ...soulseekPreviews.value,
        [key]: {
          ...soulseekPreviews.value[key],
          state: 'completed',
          localPath: replacementLocalPath,
          bytesDownloaded: totalBytes,
          totalBytes,
          speedBytesPerSec: null,
          queuePosition: null,
          error: null,
        },
      };
    }

    const previewTrackId = soulseekTrackId(result);
    if (nowPlaying.value?.id === previewTrackId && !!nowPlaying.value.soulseek_preview) {
      replaceSoulseekTrackInPlaybackState(previewTrackId, replacedTrack);

      duration.value = replacedTrack.duration_secs || duration.value;
      syncAndroid();
    } else if (nowPlaying.value?.id === replacedTrack.id) {
      nowPlaying.value = { ...replacedTrack };

      duration.value = replacedTrack.duration_secs || duration.value;
      syncAndroid();
    }

    clearPendingTrackReplacement(key);
    if (trackReplaceDialog.value?.track.id === trackId) {
      closeTrackReplaceDialog();
    }
  } catch (e: any) {
    clearPendingTrackReplacement(key);
    trackReplaceActionError.value = String(e ?? 'Failed to replace track');
    throw e;
  } finally {
    if (trackReplaceApplyingKey.value === key) {
      trackReplaceApplyingKey.value = '';
    }
  }
}

function trackReplaceActionLabel(result: SoulseekSearchResult) {
  const key = soulseekResultKey(result.username, result.filename);
  if (trackReplaceApplyingKey.value !== key) {
    return 'Replace';
  }

  if (replaceSourceLocalPath(result)) {
    return 'Replacing…';
  }

  if (soulseekDownloadState(result)?.state === 'promoting') {
    return 'Saving…';
  }

  return 'Preparing…';
}

async function replaceTrackWithSoulseekResult(track: Track, result: SoulseekSearchResult) {
  if (trackReplaceApplyingKey.value) return;

  const key = soulseekResultKey(result.username, result.filename);
  trackReplaceActionError.value = '';

  const localPath = replaceSourceLocalPath(result);
  if (localPath) {
    await performTrackReplacement(track.id, result, localPath);
    return;
  }

  soulseekPendingTrackReplacement.value = {
    ...soulseekPendingTrackReplacement.value,
    [key]: track.id,
  };
  trackReplaceApplyingKey.value = key;
  await downloadSoulseekResult(result);

  const downloadState = soulseekDownloadState(result);
  if (downloadState?.state === 'failed') {
    trackReplaceActionError.value = downloadState.error || 'Failed to download replacement track';
    clearPendingTrackReplacement(key);
  }
}

async function toggleSync() {
  if (syncBusy.value) return;
  syncBusy.value = true;
  syncError.value = '';
  const nextEnabled = !syncEnabled.value;
  syncEnabled.value = nextEnabled;
  try {
    await invoke('sync_set_enabled', { enabled: nextEnabled });
    if (nextEnabled) {
      // Kick off sync with all currently known peers
      for (const peer of peers.value) {
        invoke('sync_with_peer', { peerHost: peer.host, peerName: peer.name, peerAddresses: peer.addresses, peerPort: peer.port }).catch(() => {});
      }
    }
  } catch (error) {
    syncEnabled.value = !nextEnabled;
    syncError.value = 'Could not change LAN sync. Please try again.';
    console.error('Failed to update sync setting:', error);
  } finally { syncBusy.value = false; }
}

function syncPeer(peer: Peer) {
  invoke('sync_with_peer', { peerHost: peer.host, peerName: peer.name, peerAddresses: peer.addresses, peerPort: peer.port }).catch(() => {});
}

async function startOnboardingSync() {
  activeNav.value = 'discovery';
  showMobileNav.value = false;

  if (!syncEnabled.value) {
    await toggleSync();
    return;
  }

  peers.value.forEach((peer) => syncPeer(peer));
}

function percent(value: number, total: number) {
  if (!Number.isFinite(value) || !Number.isFinite(total) || total <= 0) return 0;
  return Math.max(0, Math.min(100, (value / total) * 100));
}

function syncOverallPercent(progress: SyncProgress) {
  return percent(progress.done, progress.total);
}

function syncItemPercent(progress: SyncProgress) {
  if (progress.phase !== 'download') return syncOverallPercent(progress);
  const itemDone = progress.item_done ?? 0;
  const itemTotal = progress.item_total ?? 0;
  return itemTotal > 0 ? percent(itemDone, itemTotal) : 0;
}

function syncStatusText(progress: SyncProgress) {
  if (progress.phase === 'index') return progress.message || 'Fetching index';
  if (progress.phase === 'download') return progress.message || 'Downloading track';
  if (progress.phase === 'reindex') return 'Indexing downloaded tracks';
  if (progress.phase === 'merging') return progress.message || 'Merging library data';
  if (progress.phase === 'done') return progress.message || 'Sync complete';
  if (progress.phase === 'error') return progress.message || 'Sync failed';
  return progress.message || progress.phase;
}

function syncCountText(progress: SyncProgress) {
  if (progress.total <= 0) return '';
  return `${progress.done}/${progress.total}`;
}

function parseSyncAddedCount(message?: string | null) {
  const match = message?.match(/^(\d+)\s+new track/i);
  return match ? Number(match[1]) : null;
}

function addSessionFile(session: LogSession, file: string | null | undefined) {
  const value = file?.trim();
  if (!value) return;
  if (session.files.some((existing) => existing.trim().toLowerCase() === value.toLowerCase())) return;
  session.files.push(value);
}

function peerLabel(peer: Peer) {
  return peer.device_name || peerDeviceNames.value[peer.name] || peer.name;
}

function samePeer(a: Peer | null | undefined, b: Peer | null | undefined) {
  return !!a && !!b && a.host === b.host && a.port === b.port;
}

function remotePeerArgs(peer: Peer) {
  return { peerHost: peer.host, peerAddresses: peer.addresses, peerPort: peer.port };
}

function isRemoteOutputPeer(peer: Peer) {
  return samePeer(remoteOutputPeer.value, peer);
}

function peerPlaybackLabel(peer: Peer) {
  switch (peer.playback?.state) {
    case 'playing': return 'Playing';
    case 'paused': return 'Paused';
    case 'stopped': return 'Stopped';
    case 'ended': return 'Finished';
    default: return 'Idle';
  }
}

function peerPlaybackClass(peer: Peer) {
  return `peer-status-${peer.playback?.state ?? 'idle'}`;
}

function peerNowPlayingText(peer: Peer) {
  const playback = peer.playback;
  if (!playback) return '';
  const title = playback.title?.trim();
  const artist = playback.artist?.trim();
  const album = playback.album?.trim();
  if (title) return title;
  if (artist) return artist;
  return album || '';
}

watch(peers, (list) => {
  if (!remoteOutputPeer.value) return;
  const updated = list.find((peer) => samePeer(peer, remoteOutputPeer.value));
  if (updated) {
    remoteOutputPeer.value = updated;
    return;
  }
  remoteOutputPeer.value = null;
  stopTicker();
  refreshPlaybackState().catch(() => {});
});

const currentTrack = computed<{ title: string; artist: string; colors: [string, string] }>(() => {
  if (nowPlaying.value) {
    return {
      title: nowPlaying.value.title || nowPlaying.value.path,
      artist: artistLabel(nowPlaying.value.artist),
      colors: placeholderCoverColors(nowPlaying.value.file_hash),
    };
  }
  const rp = remoteOutputPeer.value?.playback;
  if (rp && (rp.title || rp.hash)) {
    return {
      title: rp.title || 'Unknown',
      artist: artistLabel(rp.artist),
      colors: placeholderCoverColors(rp.hash ?? null),
    };
  }
  return { title: 'No track', artist: '', colors: placeholderCoverColors(null) };
});

const beatIntensity = computed(() => {
  if (BEAT_AMP <= 0) return 0;
  return Math.max(0, Math.min(1.4, (beatScale.value - 1) / BEAT_AMP));
});

const currentCoverUrl = computed(() => coverUrl(nowPlaying.value?.cover_hash, 'medium'));

const currentRarityColor = computed(() => {
  const rarity = highlightedRarity(nowPlaying.value?.rarity ?? null);
  return rarity ? activeThemeColors.value[rarityColorName(rarity)] : null;
});

const detailBackdropWashStyle = computed(() => {
  const [colorA, colorB] = currentTrack.value.colors;
  const pulse = beatIntensity.value;
  return {
    background: `radial-gradient(circle at 18% 20%, ${colorA} 0%, transparent ${Math.max(28, 46 - pulse * 5)}%), radial-gradient(circle at 82% 18%, ${colorB} 0%, transparent ${Math.max(26, 44 - pulse * 4)}%), linear-gradient(180deg, ${translucent('bg', 22)}, ${translucent('bg', 74)} 38%, ${translucent('bg', 95)})`,
    opacity: (0.62 + pulse * 0.18).toFixed(2),
  };
});

const detailAuraStyle = computed(() => {
  const [colorA, colorB] = currentTrack.value.colors;
  const pulse = beatIntensity.value;
  return {
    background: `radial-gradient(circle at 34% 34%, ${colorA} 0%, transparent 48%), radial-gradient(circle at 66% 62%, ${colorB} 0%, transparent 46%), radial-gradient(circle at 50% 50%, ${translucent('text', 20)} 0%, transparent 34%)`,
    opacity: (0.42 + pulse * 0.18).toFixed(2),
    transform: `scale(${(1.01 + pulse * 0.05).toFixed(3)})`,
    filter: `blur(${(24 + pulse * 10).toFixed(0)}px) saturate(${(1.02 + pulse * 0.2).toFixed(2)})`,
  };
});

const SPECTRUM_SEGMENT_COUNT = 32;

function createEmptySpectrum() {
  return Array.from({ length: SPECTRUM_SEGMENT_COUNT }, () => 0);
}

const spectrumLevels = ref<number[]>(createEmptySpectrum());

const spectrumSegments = Array.from({ length: SPECTRUM_SEGMENT_COUNT }, (_, index) => {
  const seed = (Math.sin(index * 12.9898) + 1) * 0.5;
  return {
    index,
    angle: (index / SPECTRUM_SEGMENT_COUNT) * 360,
    width: `${(3.2 + seed * 1.7).toFixed(1)}px`,
    height: `${(12 + seed * 16).toFixed(1)}px`,
    glow: `${(6 + seed * 10).toFixed(1)}px`,
  };
});

const detailSpectrumStyle = computed(() => ({
  '--ring-pulse': beatIntensity.value.toFixed(3),
}));

function spectrumSpokeStyle(segment: (typeof spectrumSegments)[number]) {
  return {
    transform: `rotate(${segment.angle}deg)`,
  };
}

function spectrumBarStyle(segment: (typeof spectrumSegments)[number]) {
  const level = spectrumLevels.value[segment.index] ?? 0;
  const pulse = beatIntensity.value;
  const colorA = currentTrack.value.colors[segment.index % 2];
  const colorB = currentTrack.value.colors[(segment.index + 1) % 2];
  const scale = 0.2 + level * 1.9 + pulse * 0.14;
  const lift = 3 + level * 18 + pulse * 5;
  const opacity = Math.min(1, 0.16 + level * 0.88 + pulse * 0.06);

  return {
    '--bar-width': segment.width,
    '--bar-height': segment.height,
    '--bar-scale': scale.toFixed(3),
    '--bar-lift': `${lift.toFixed(1)}px`,
    background: `linear-gradient(180deg, ${colorA} 0%, ${translucent('text', 95)} 58%, ${colorB} 100%)`,
    boxShadow: `0 0 ${segment.glow} ${translucent('text', Number((8 + level * 36).toFixed(1)))}`,
    filter: `saturate(${(1 + level * 0.7).toFixed(2)})`,
    opacity: opacity.toFixed(3),
  };
}

const mobileSeekActive = ref(false);
const seekPreviewPos = ref<number | null>(null);
const showDetail = ref(false);

let spectrumPollTimer: ReturnType<typeof setInterval> | null = null;
let spectrumPollPending = false;
let spectrumPollVersion = 0;

function normalizeSpectrum(values: number[]) {
  return Array.from({ length: SPECTRUM_SEGMENT_COUNT }, (_, index) => {
    const value = values[index] ?? 0;
    return Math.max(0, Math.min(1, value));
  });
}

function resetSpectrum() {
  spectrumLevels.value = createEmptySpectrum();
}

async function refreshSpectrum(version = spectrumPollVersion) {
  if (version !== spectrumPollVersion || spectrumPollPending || reducedMotion.value || !showDetail.value || !nowPlaying.value) {
    return;
  }

  spectrumPollPending = true;
  try {
    const next = await invoke<number[]>('playback_spectrum');
    if (version !== spectrumPollVersion) return;
    spectrumLevels.value = normalizeSpectrum(next);
  } catch (_) {
    if (version !== spectrumPollVersion) return;
    resetSpectrum();
  } finally {
    if (version === spectrumPollVersion) {
      spectrumPollPending = false;
    }
  }
}

function startSpectrumPolling() {
  if (spectrumPollTimer || reducedMotion.value || !showDetail.value || !nowPlaying.value) return;
  spectrumPollVersion += 1;
  const version = spectrumPollVersion;
  void refreshSpectrum(version);
  spectrumPollTimer = setInterval(() => {
    void refreshSpectrum(version);
  }, 50);
}

function stopSpectrumPolling(reset = true) {
  spectrumPollVersion += 1;
  spectrumPollPending = false;
  if (spectrumPollTimer) {
    clearInterval(spectrumPollTimer);
    spectrumPollTimer = null;
  }
  if (reset) resetSpectrum();
}

// ── 3D cover card ──────────────────────────────────────────────────────────
const cardRotX = ref(0);
const cardRotY = ref(0);
const cardTX = ref(0);   // translation X in px
const cardTY = ref(0);   // translation Y in px
const cardInteracting = ref(false);
const cardDragging = ref(false);
let cardAmbientRaf = 0;
let cardSpringRaf = 0;

const MAX_TILT = 18;      // max tilt degrees from hover
const MAX_TILT_DRAG = 4;  // max tilt degrees while dragging
const MAX_TRANS = 100;    // max translate px from interaction
const AMBIENT_R = 10;    // ambient rotation radius in degrees
const AMBIENT_PERIOD = 8000; // ms for one ambient orbit

function startAmbient() {
  const t0 = performance.now();
  function tick(now: number) {
    if (cardInteracting.value) { cardAmbientRaf = requestAnimationFrame(tick); return; }
    const phase = ((now - t0) / AMBIENT_PERIOD) * Math.PI * 2;
    cardRotY.value = AMBIENT_R * Math.sin(phase);
    cardRotX.value = AMBIENT_R * 0.5 * Math.sin(phase * 2);
    cardAmbientRaf = requestAnimationFrame(tick);
  }
  cardAmbientRaf = requestAnimationFrame(tick);
}

function stopAmbient() {
  cancelAnimationFrame(cardAmbientRaf);
}

function springBack() {
  if (reducedMotion.value || !appearance.album3d) return;
  cancelAnimationFrame(cardSpringRaf);
  const FACTOR = 0.15;
  function tick() {
    cardRotX.value *= (1 - FACTOR);
    cardRotY.value *= (1 - FACTOR);
    cardTX.value   *= (1 - FACTOR);
    cardTY.value   *= (1 - FACTOR);
    if (Math.abs(cardRotX.value) > 0.05 || Math.abs(cardRotY.value) > 0.05 ||
        Math.abs(cardTX.value) > 0.05 || Math.abs(cardTY.value) > 0.05) {
      cardSpringRaf = requestAnimationFrame(tick);
    } else {
      cardRotX.value = 0; cardRotY.value = 0;
      cardTX.value = 0;   cardTY.value = 0;
    }
  }
  cardSpringRaf = requestAnimationFrame(tick);
}


// Plain (non-reactive) vars for pending mouse/touch values — flushed to refs via RAF
let _pendX = 0;
let _pendY = 0;
let _pendTX = 0;
let _pendTY = 0;
let _mouseRafId = 0;

function _flushCardRot() {
  cardRotX.value = _pendX;
  cardRotY.value = _pendY;
  cardTX.value = _pendTX;
  cardTY.value = _pendTY;
  _mouseRafId = 0;
}

function _scheduleFlush() {
  if (!_mouseRafId) _mouseRafId = requestAnimationFrame(_flushCardRot);
}

let _mouseDown = false;

function onCardMouseMove(e: MouseEvent) {
  if (reducedMotion.value || !appearance.album3d) return;
  const el = e.currentTarget as HTMLElement;
  const r = el.getBoundingClientRect();
  const nx = (e.clientX - (r.left + r.width / 2)) / (r.width / 2);   // -1..1
  const ny = (e.clientY - (r.top + r.height / 2)) / (r.height / 2);  // -1..1
  const tiltMax = _mouseDown ? MAX_TILT_DRAG : MAX_TILT;
  _pendY = nx * tiltMax;
  _pendX = -ny * tiltMax;
  _pendTX = _mouseDown ? nx * MAX_TRANS : 0;
  _pendTY = _mouseDown ? ny * MAX_TRANS : 0;
  _scheduleFlush();
}

function onCardMouseDown() {
  if (reducedMotion.value || !appearance.album3d) return;
  _mouseDown = true;
  cardDragging.value = true;
}

function onCardMouseUp() {
  _mouseDown = false;
  cardDragging.value = false;
}

function onCardMouseLeave() {
  _mouseDown = false;
  cardDragging.value = false;
  cardInteracting.value = false;
  springBack();
}

function onCardMouseEnter() {
  if (reducedMotion.value || !appearance.album3d) return;
  cancelAnimationFrame(cardSpringRaf);
  cardInteracting.value = true;
}

let cardTouchStartX = 0;
let cardTouchStartY = 0;

function onCardTouchStart(e: TouchEvent) {
  if (reducedMotion.value || !appearance.album3d) return;
  cardInteracting.value = true;
  cardDragging.value = true;
  cancelAnimationFrame(cardSpringRaf);
  cardTouchStartX = e.touches[0].clientX;
  cardTouchStartY = e.touches[0].clientY;
}

function onCardTouchMove(e: TouchEvent) {
  if (reducedMotion.value || !appearance.album3d) return;
  e.preventDefault();
  const dx = e.touches[0].clientX - cardTouchStartX;
  const dy = e.touches[0].clientY - cardTouchStartY;
  _pendY  = Math.max(-MAX_TILT_DRAG, Math.min(MAX_TILT_DRAG, dx * 0.1));
  _pendX  = Math.max(-MAX_TILT_DRAG, Math.min(MAX_TILT_DRAG, -dy * 0.1));
  _pendTX = Math.max(-MAX_TRANS, Math.min(MAX_TRANS, dx * 0.25));
  _pendTY = Math.max(-MAX_TRANS, Math.min(MAX_TRANS, dy * 0.25));
  _scheduleFlush();
}

function onCardTouchEnd() {
  cardInteracting.value = false;
  cardDragging.value = false;
  springBack();
}

watch([showDetail, reducedMotion, () => appearance.album3d], ([open, reduce, album3d]) => {
  stopAmbient();
  cancelAnimationFrame(cardSpringRaf);
  if (_mouseRafId) cancelAnimationFrame(_mouseRafId);
  _mouseRafId = 0;
  cardRotX.value = 0;
  cardRotY.value = 0;
  cardTX.value = 0;
  cardTY.value = 0;
  _mouseDown = false;
  cardInteracting.value = false;
  cardDragging.value = false;
  if (open && !reduce) {
    if (album3d) startAmbient();
    startSpectrumPolling();
  } else stopSpectrumPolling();
});
watch(reducedMotion, (reduce) => {
  if (!reduce) return;
  if (beatRafId !== null) cancelAnimationFrame(beatRafId);
  beatRafId = null;
  beatScale.value = 1;
  for (const id of scheduledBeatTimeoutIds) clearTimeout(id);
  scheduledBeatTimeoutIds.clear();
});

watch(() => nowPlaying.value?.id, () => {
  resetSpectrum();
  if (showDetail.value) {
    void refreshSpectrum(spectrumPollVersion);
  }
});

watch(isPlaying, (playing) => {
  if (!playing) {
    resetSpectrum();
  } else if (showDetail.value) {
    startSpectrumPolling();
    void refreshSpectrum(spectrumPollVersion);
  }
});
// ────────────────────────────────────────────────────────────────────────────

const displayProgressPercent = computed(() => {
  const pos = seekPreviewPos.value ?? currentTime.value;
  return duration.value > 0 ? Math.max(0, Math.min(100, (pos / duration.value) * 100)) : 0;
});

const soulseekPreviewBufferPercent = computed(() => {
  const track = nowPlaying.value;
  if (!track?.soulseek_preview || !track.preview_growing) {
    return 0;
  }

  const username = track.soulseek_username || track.artist || '';
  const filename = track.soulseek_filename || '';
  if (!username || !filename) {
    return 0;
  }

  const state = soulseekPreviews.value[soulseekResultKey(username, filename)];
  const downloaded = state?.bytesDownloaded ?? 0;
  const total = state?.totalBytes ?? track.soulseek_size ?? 0;
  if (total <= 0 || downloaded <= 0) {
    return 0;
  }

  return Math.max(0, Math.min(100, (downloaded / total) * 100));
});

const showSoulseekPreviewBuffer = computed(() => soulseekPreviewBufferPercent.value > 0);

function queuedSourceHasMore() {
  return !!queuedPagedSource.value?.hasMore;
}

/**
 * Extends the playback source at the moment the queue reaches the loaded
 * boundary.  It deliberately uses the same page commands as scrolling, but
 * keeps its own track array so changing the Library filter cannot redirect an
 * already playing queue.
 */
async function loadNextQueuedSourcePage(): Promise<boolean> {
  const source = queuedPagedSource.value;
  if (!source || !source.hasMore || queuedPageLoading.value) return false;
  queuedPageLoading.value = true;
  if (source.kind === 'library') libraryLoadingMore.value = true;
  if (source.kind === 'regular-playlist' && playlistView.value?.id === source.playlistId) {
    playlistView.value.loadingMore = true;
  }
  if (source.kind === 'smart-playlist' && smartView.value?.id === source.playlist.id) {
    smartViewLoadingMore.value = true;
  }
  try {
    if (source.kind === 'library') {
      const offset = queueLibraryTracks.value.length;
      const page = await invoke<TrackPage>('get_tracks_page', {
        query: source.query || null,
        limit: LIBRARY_TRACK_PAGE_SIZE,
        offset,
        includeDuplicates: source.includeDuplicates,
      });
      // Playback may have switched sources while the request was in flight.
      if (queuedPagedSource.value !== source) return false;
      queueLibraryTracks.value = [...queueLibraryTracks.value, ...page.tracks];
      source.hasMore = page.has_more;
      if (libraryLoadedQuery.value === source.query) {
        libraryTracks.value = [...libraryTracks.value, ...page.tracks];
        libraryHasMore.value = page.has_more;
      }
      return page.tracks.length > 0;
    }

    if (source.kind === 'regular-playlist') {
      const offset = queuePlaylistTracks.value.length;
      const page = await invoke<TrackPage>('get_playlist_tracks_page', {
        playlistId: source.playlistId,
        limit: LIBRARY_TRACK_PAGE_SIZE,
        offset,
        includeDuplicates: source.includeDuplicates,
      });
      if (queuedPagedSource.value !== source) return false;
      queuePlaylistTracks.value = [...queuePlaylistTracks.value, ...page.tracks];
      source.hasMore = page.has_more;
      if (playlistView.value?.id === source.playlistId) {
        playlistView.value = {
          ...playlistView.value,
          tracks: [...playlistView.value.tracks, ...page.tracks],
          total: page.total,
          hasMore: page.has_more,
          loadingMore: false,
        };
      }
      return page.tracks.length > 0;
    }

    const offset = queuePlaylistTracks.value.length;
    const page = await invoke<TrackPage>('get_smart_playlist_tracks_page', {
      rulesJson: JSON.stringify(source.playlist.rules),
      matchMode: source.playlist.match,
      limit: LIBRARY_TRACK_PAGE_SIZE,
      offset,
      includeDuplicates: source.includeDuplicates,
    });
    if (queuedPagedSource.value !== source) return false;
    queuePlaylistTracks.value = [...queuePlaylistTracks.value, ...page.tracks];
    source.hasMore = page.has_more;
    if (smartView.value?.id === source.playlist.id) {
      smartViewTracks.value = [...smartViewTracks.value, ...page.tracks];
      smartViewTotal.value = page.total;
      smartViewHasMore.value = page.has_more;
    }
    return page.tracks.length > 0;
  } catch (error) {
    console.error('Failed to load the next playback page:', error);
    return false;
  } finally {
    queuedPageLoading.value = false;
    if (source.kind === 'library') libraryLoadingMore.value = false;
    if (source.kind === 'regular-playlist' && playlistView.value?.id === source.playlistId) {
      playlistView.value.loadingMore = false;
    }
    if (source.kind === 'smart-playlist' && smartView.value?.id === source.playlist.id) {
      smartViewLoadingMore.value = false;
    }
  }
}

/** Return the full ordered list currently available for a given source */
function sourceList(src: QueueSource): Track[] {
  if (src === 'recent') {
    const recent = filterDuplicateTracks(recentTracks.value);
    return recent.length ? recent : filterDuplicateTracks(libraryTracks.value).slice(0, 12);
  }
  if (src === 'playlist') return queuePlaylistTracks.value;
  if (src === 'soulseek') return queueSoulseekTracks.value;
  // 'library' – flattened in grouped order (same as libraryFlatList). Once
  // playback starts, use its stable paged copy rather than the visible filter.
  return queuedPagedSource.value?.kind === 'library'
    ? queueLibraryTracks.value
    : filterDuplicateTracks(libraryFlatList.value);
}

/** Fill the queue up to 5 upcoming tracks from the source */
function refillQueue() {
  if (repeatMode.value === 2 && nowPlaying.value) {
    // repeat-one: queue is just the current track repeated
    queue.value = Array(5).fill(nowPlaying.value);
    return;
  }
  const list = sourceList(queueSource.value);
  if (!list.length) return;
  let attempts = 0;
  const maxAttempts = Math.max(list.length * 2, 8);
  while (queue.value.length < 5 && attempts < maxAttempts) {
    attempts += 1;
    const nextIdx = queueSourceIndex.value + 1;
    if (nextIdx >= list.length) {
      if (queuedSourceHasMore()) break;
      if ((queueSource.value === 'playlist' || queueSource.value === 'soulseek') && repeatMode.value !== 1) break; // stop at end of finite queue sources
      queueSourceIndex.value = -1; // wrap (library or repeat-all)
    } else {
      queueSourceIndex.value = nextIdx;
    }
    if (queueSourceIndex.value >= 0) {
      const candidate = list[queueSourceIndex.value];
      if (showDuplicateTracks.value || !candidate.is_duplicate) {
        queue.value.push(candidate);
      }
    }
  }
}

/** Notify Android foreground service of current playback state. */
function syncAndroid() {
  const bridge = (window as any).AndroidBridge;
  if (!bridge) return;
  if (nowPlaying.value) {
    bridge.updatePlayback(
      currentTrack.value.title,
      currentTrack.value.artist,
      isPlaying.value,
      currentTime.value,
      duration.value
    );
  } else {
    bridge.stopPlayback?.();
  }
}

function seekPosFromEvent(e: MouseEvent | PointerEvent, el: HTMLElement): number {
  const rect = el.getBoundingClientRect();
  const ratio = (e.clientX - rect.left) / rect.width;
  return Math.max(0, Math.min(1, ratio)) * duration.value;
}

function mobileSeekHoldStart(e: PointerEvent) {
  const el = e.currentTarget as HTMLElement;
  mobileSeekActive.value = true;
  el.setPointerCapture?.(e.pointerId);
  const pos = seekPosFromEvent(e, el);
  seekPreviewPos.value = pos;
  currentTime.value = Math.round(pos);
}

function mobileSeekHoldMove(e: PointerEvent) {
  if (!mobileSeekActive.value) return;
  const el = e.currentTarget as HTMLElement;
  const pos = seekPosFromEvent(e, el);
  seekPreviewPos.value = pos;
  currentTime.value = Math.round(pos);
}

async function mobileSeekHoldEnd(e: PointerEvent) {
  if (!mobileSeekActive.value) return;
  const el = e.currentTarget as HTMLElement;
  // On pointercancel the browser reports clientX/clientY = 0, so use the last
  // known preview position instead of recalculating from the (bogus) event.
  const pos = e.type === 'pointercancel'
    ? (seekPreviewPos.value ?? currentTime.value)
    : seekPosFromEvent(e, el);
  seekPreviewPos.value = pos;
  currentTime.value = Math.round(pos);
  await seekCurrentOutput(pos);
  const st = await refreshPlaybackState();
  if (st.playing) {
    startTicker();
  } else {
    stopTicker();
  }
  syncAndroid();
  seekPreviewPos.value = null;
  mobileSeekActive.value = false;
  el.releasePointerCapture?.(e.pointerId);
}

/** Start playing a specific track from a given source list at a given index */
function playFromPlaylist(tracks: Track[], index: number, pagedSource: QueuedPagedSource | null = null) {
  queuePlaylistTracks.value = [...tracks];
  queuedPagedSource.value = pagedSource;
  playTrackFrom('playlist', index);
}

function isCurrentTrack(trackId: number) {
  return nowPlaying.value?.id === trackId;
}

function isTrackPlaying(trackId: number) {
  return isCurrentTrack(trackId) && isPlaying.value;
}

async function toggleCardPlayback(src: QueueSource, index: number, trackId: number) {
  if (isCurrentTrack(trackId)) {
    await togglePlay();
    return;
  }
  await playTrackFrom(src, index);
}

function isNextTrack(trackId: number) {
  const nextTrackId = queue.value[0]?.id;
  return nextTrackId !== undefined && nextTrackId !== null && nextTrackId !== nowPlaying.value?.id && nextTrackId === trackId;
}

function restoreContentScroll(scrollTop: number | null) {
  if (scrollTop === null) return;
  nextTick(() => {
    requestAnimationFrame(() => {
      if (contentRef.value) contentRef.value.scrollTop = scrollTop;
    });
  });
}

async function playTrackFrom(src: QueueSource, index: number) {
  if (src !== 'library' && src !== 'playlist') {
    queuedPagedSource.value = null;
  }
  const list = sourceList(src);
  if (!list.length) return;
  bumpPlaybackTransitionSequence();
  const contentScrollTop = contentRef.value?.scrollTop ?? null;
  const wasPlaying = isPlaying.value;
  queueSource.value = src;
  const track = list[index];
  nowPlaying.value = track;
  duration.value = track.duration_secs || 0;
  currentTime.value = 0;
  // rebuild queue starting after this index
  queueSourceIndex.value = index;
  queue.value = [];
  refillQueue();
  await playTrackOnCurrentOutput(track, wasPlaying, 0);
  if (!wasPlaying) {
    isPlaying.value = false;
    stopTicker();
  } else {
    await refreshPlaybackState();
    recordPlayIfTracked(track);
    startTicker();
  }
  syncAndroid();
  restoreContentScroll(contentScrollTop);
}

/** Advance to next track in queue */
async function playNext() {
  bumpPlaybackTransitionSequence();
  if (!queue.value.length) {
    if (await loadNextQueuedSourcePage()) {
      refillQueue();
    }
  }
  if (!queue.value.length) {
    isPlaying.value = false;
    stopTicker();
    await stopCurrentOutput();
    syncAndroid();
    return;
  }
  const next = queue.value.shift()!;
  nowPlaying.value = next;
  duration.value = next.duration_secs || 0;
  currentTime.value = 0;
  refillQueue();
  await playTrackOnCurrentOutput(next, true, 0);
  isPlaying.value = true;
  await refreshPlaybackState();
  isPlaying.value = true;
  recordPlayIfTracked(next);
  startTicker();
  syncAndroid();
}

/** Go to previous track (restart current if >3s in, else go back in source) */
async function playPrev() {
  bumpPlaybackTransitionSequence();
  if (currentTime.value > 3) {
    currentTime.value = 0;
    await seekCurrentOutput(0);
    return;
  }
  const list = sourceList(queueSource.value);
  if (!list.length) return;
  const curIdx = list.findIndex(t => t.id === nowPlaying.value?.id);
  const prevIdx = curIdx > 0 ? curIdx - 1 : list.length - 1;
  const track = list[prevIdx];
  nowPlaying.value = track;
  duration.value = track.duration_secs || 0;
  currentTime.value = 0;
  queueSourceIndex.value = prevIdx;
  queue.value = [];
  refillQueue();
  isPlaying.value = true;
  await playTrackOnCurrentOutput(track, true, 0);
  await refreshPlaybackState();
  isPlaying.value = true;
  recordPlayIfTracked(track);
  startTicker();
  syncAndroid();
}

function jumpToQueueItem(index: number) {
  // remove everything before that item, play it
  queue.value.splice(0, index);
  playNext();
  showQueueMenu.value = false;
}

interface PlaybackStatus { playing: boolean; finished: boolean; position: number; duration: number; source_sample_rate?: number | null; }

function trackForHash(hash?: string | null) {
  if (!hash) return null;
  return tracksByHash.value[hash] ?? null;
}

async function playTrackLocally(track: Track, autoplay: boolean, position = 0) {
  remoteControlled.value = false; // local action takes control back
  if (track.soulseek_preview) {
    await invoke('soulseek_play_preview', {
      request: {
        username: track.soulseek_username || track.artist || '',
        filename: track.soulseek_filename || track.path,
        size: track.soulseek_size || 0,
      },
    });
  } else if (track.id <= 0) {
    await invoke('playback_play', { path: track.path });
  } else {
    await invoke('playback_play_track', { id: track.id });
  }
  if (position > 0) {
    await invoke('playback_seek', { position });
  }
  if (!autoplay) {
    await invoke('playback_pause');
  }
}

async function playTrackRemotely(track: Track, autoplay: boolean, position = 0, peer = remoteOutputPeer.value) {
  if (!peer) throw new Error('No remote player selected');
  if (!track.file_hash) throw new Error('Track is not indexed yet and cannot be transferred');
  await invoke('remote_playback_transfer', {
    ...remotePeerArgs(peer),
    hash: track.file_hash,
    position,
    autoplay,
  });
}

async function playTrackOnCurrentOutput(track: Track, autoplay: boolean, position = 0) {
  if (track.soulseek_preview) {
    if (remoteOutputPeer.value) {
      try {
        await invoke('remote_playback_pause', remotePeerArgs(remoteOutputPeer.value));
      } catch (_) {}
      remoteOutputPeer.value = null;
    }
    await playTrackLocally(track, autoplay, position);
    return;
  }

  if (remoteOutputPeer.value) {
    await playTrackRemotely(track, autoplay, position, remoteOutputPeer.value);
  } else {
    await playTrackLocally(track, autoplay, position);
  }
}

function recordPlayIfTracked(track: Track) {
  const isActualRecentQueue = queueSource.value === 'recent' && filterDuplicateTracks(recentTracks.value).length > 0;
  if (track.id <= 0 || isActualRecentQueue) return;
  track.play_count++;
  invoke('record_play', { id: track.id }).then(() => loadRecent());
}

async function pauseCurrentOutput() {
  if (remoteOutputPeer.value) {
    await invoke('remote_playback_pause', remotePeerArgs(remoteOutputPeer.value));
  } else {
    await invoke('playback_pause');
  }
}

async function resumeCurrentOutput() {
  if (remoteOutputPeer.value) {
    await invoke('remote_playback_resume', remotePeerArgs(remoteOutputPeer.value));
  } else {
    await invoke('playback_resume');
  }
}

async function stopCurrentOutput() {
  if (remoteOutputPeer.value) {
    await invoke('remote_playback_stop', remotePeerArgs(remoteOutputPeer.value));
  } else {
    await invoke('playback_stop');
  }
}

async function seekCurrentOutput(position: number) {
  if (remoteOutputPeer.value) {
    await invoke('remote_playback_seek', { ...remotePeerArgs(remoteOutputPeer.value), position });
  } else {
    await invoke('playback_seek', { position });
  }
}

function applyRemotePlaybackStatus(status: RemoteDeviceStatus): PlaybackStatus {
  const playback = status.playback;
  if (status.device_name && remoteOutputPeer.value) {
    peerDeviceNames.value = { ...peerDeviceNames.value, [remoteOutputPeer.value.name]: status.device_name };
  }
  const remoteTrack = trackForHash(playback?.hash) ?? nowPlaying.value;
  if (remoteTrack && playback?.hash && nowPlaying.value?.id !== remoteTrack.id) {
    nowPlaying.value = remoteTrack;
  }
  isPlaying.value = playback?.state === 'playing';
  currentTime.value = Math.floor(playback?.position ?? 0);
  if ((playback?.duration ?? 0) > 0) {
    duration.value = Math.floor(playback?.duration ?? 0);
  }
  if (playback?.volume != null) {
    volume.value = Math.round(playback.volume * 100);
  }
  if (playback?.beat_seq != null && playback.beat_seq !== lastRemoteBeatSeq) {
    lastRemoteBeatSeq = playback.beat_seq;
    if (playback.state === 'playing') {
      startBeatAnimation(0);
    }
  }
  return {
    playing: isPlaying.value,
    finished: playback?.state === 'ended',
    position: playback?.position ?? 0,
    duration: playback?.duration ?? 0,
  };
}

async function capturePlaybackSnapshot() {
  if (remoteOutputPeer.value || isPlaying.value) {
    try {
      await refreshPlaybackState();
    } catch (_) {
      // Keep the current UI snapshot if the active output cannot be refreshed.
    }
  }
  return {
    track: nowPlaying.value,
    position: currentTime.value,
    playing: isPlaying.value,
  };
}

async function refreshPlaybackState() {
  if (remoteOutputPeer.value) {
    const st = await invoke<RemoteDeviceStatus>('remote_playback_status', remotePeerArgs(remoteOutputPeer.value));
    return applyRemotePlaybackStatus(st);
  }
  const st = await invoke<PlaybackStatus>('playback_status');
  const currentTrack = nowPlaying.value;
  const metadataDuration = Math.floor(currentTrack?.duration_secs ?? 0);
  const reportedDuration = st.duration > 0 ? Math.floor(st.duration) : 0;
  isPlaying.value = st.playing;
  currentTime.value = Math.floor(st.position);
  nativeSampleRate.value = st.source_sample_rate ?? nativeSampleRate.value;
  if (currentTrack?.preview_growing) {
    duration.value = Math.max(duration.value, metadataDuration, reportedDuration, currentTime.value);
  } else if (reportedDuration > 0) {
    duration.value = reportedDuration;
  }
  return st;
}

let ticker: ReturnType<typeof setInterval> | null = null;
let androidSyncCounter = 0;
let playbackTransitionSequence = 0;
let handlingFinishedPlayback = false;

function bumpPlaybackTransitionSequence() {
  playbackTransitionSequence += 1;
  return playbackTransitionSequence;
}

function stopTicker() {
  if (ticker) { clearInterval(ticker); ticker = null; }
}

async function handleFinishedPlayback(expectedSequence = playbackTransitionSequence) {
  if (remoteControlled.value) return; // queue advancement is owned by the controlling device
  if (expectedSequence !== playbackTransitionSequence || handlingFinishedPlayback) return;
  handlingFinishedPlayback = true;
  try {
    if (repeatMode.value === 2) {
      if (nowPlaying.value) {
        bumpPlaybackTransitionSequence();
        await playTrackOnCurrentOutput(nowPlaying.value, true, 0);
        startTicker();
        syncAndroid();
      }
    } else {
      await playNext();
    }
  } finally {
    handlingFinishedPlayback = false;
  }
}

(window as any)._playbackFinished = () => handleFinishedPlayback(playbackTransitionSequence);

function startTicker() {
  stopTicker();
  androidSyncCounter = 0;
  ticker = setInterval(async () => {
    try {
      const transitionSequence = playbackTransitionSequence;
      const st = await refreshPlaybackState();
      // sync Android notification progress ~every 4 ticks (≈1 s)
      if (++androidSyncCounter >= 4) { androidSyncCounter = 0; syncAndroid(); }
      if (st.finished) {
        await handleFinishedPlayback(transitionSequence);
      }
    } catch (_) { /* ignore polling errors */ }
  }, 250);
}

async function togglePlay() {
  if (remoteOutputPeer.value && !nowPlaying.value) {
    await refreshPlaybackState();
  }
  if (!nowPlaying.value) {
    if (remoteOutputPeer.value) {
      if (isPlaying.value) {
        await pauseCurrentOutput();
        stopTicker();
      } else {
        await resumeCurrentOutput();
        startTicker();
      }
      await refreshPlaybackState();
      syncAndroid();
      return;
    }
    if (libraryTracks.value.length) {
      queueLibraryTracks.value = [...libraryFlatList.value];
      queuedPagedSource.value = {
        kind: 'library',
        query: libraryLoadedQuery.value,
        includeDuplicates: showDuplicateTracks.value,
        hasMore: libraryHasMore.value,
      };
      await playTrackFrom('library', 0);
    }
    return;
  }
  if (isPlaying.value) {
    await pauseCurrentOutput();
    stopTicker();
  } else {
    await resumeCurrentOutput();
    startTicker();
  }
  await refreshPlaybackState();
  syncAndroid();
}

async function syncPlaybackStateForMediaControl() {
  try {
    await refreshPlaybackState();
  } catch (_) {}
}

async function toggleFromMediaControl() {
  await syncPlaybackStateForMediaControl();
  await togglePlay();
}

async function playFromMediaControl() {
  await syncPlaybackStateForMediaControl();
  if (!isPlaying.value) {
    await togglePlay();
  } else {
    syncAndroid();
  }
}

async function pauseFromMediaControl() {
  await syncPlaybackStateForMediaControl();
  if (isPlaying.value) {
    await togglePlay();
  } else {
    syncAndroid();
  }
}

async function toggleLike(track: Track, e?: Event) {
  e?.stopPropagation();
  if (track.id <= 0 || pendingLikes.value.has(track.id)) return;
  pendingLikes.value.add(track.id);
  try {
    const liked = await invoke<boolean>('toggle_like', { id: track.id });
    track.is_liked = liked;
    // The same track can have separate objects after a library refresh or in another view.
    const lists = [
      libraryTracks.value, recentTracks.value, localSearchTracks.value,
      queueLibraryTracks.value, queuePlaylistTracks.value, queueSoulseekTracks.value, queue.value,
      playlistView.value?.tracks ?? [], smartViewTracks.value, editingSPTracks.value,
      ...Object.values(homePinnedRegularTracks.value), historyEntries.value.map(entry => entry.track),
    ];
    for (const list of lists) for (const item of list) {
      if (item.id === track.id) item.is_liked = liked;
    }
    if (nowPlaying.value?.id === track.id) nowPlaying.value.is_liked = liked;
  } catch (error) {
    console.error('Failed to toggle like', error);
  } finally {
    pendingLikes.value.delete(track.id);
  }
}

async function seek(e: MouseEvent) {
  const el = e.currentTarget as HTMLElement;
  const pos = seekPosFromEvent(e, el);
  currentTime.value = Math.round(pos);
  await seekCurrentOutput(pos);
  const st = await refreshPlaybackState();
  if (st.playing) {
    startTicker();
  } else {
    stopTicker();
  }
  syncAndroid();
}

function setVolume(e: MouseEvent) {
  const el = e.currentTarget as HTMLElement;
  volume.value = Math.round(Math.max(0, Math.min(1, (e.clientX - el.getBoundingClientRect().left) / el.offsetWidth)) * 100);
  if (remoteOutputPeer.value) {
    invoke('remote_playback_volume', { ...remotePeerArgs(remoteOutputPeer.value), value: volume.value / 100 });
  } else {
    invoke('playback_set_volume', { value: volume.value / 100 });
  }
}

async function refreshOutputDevices() {
  if (audioLoading.value) return;
  audioLoading.value = true;
  deviceMenuError.value = '';
  try {
    const res = await invoke<DeviceList>('get_output_devices');
    outputDevices.value = res.devices;
    currentDevice.value = res.current;
    currentSampleRate.value = res.current_sample_rate;
    const status = await invoke<PlaybackStatus>('playback_status');
    nativeSampleRate.value = status.source_sample_rate ?? null;
  } catch {
    deviceMenuError.value = 'Could not load audio outputs. Please try again.';
  } finally { audioLoading.value = false; }
}
async function toggleDeviceMenu() {
  const willOpen = !showDeviceMenu.value;
  if (willOpen) {
    showQueueMenu.value = false;
    showUserMenu.value = false;
    await refreshOutputDevices();
  }
  showDeviceMenu.value = willOpen;
}

function toggleQueueMenu() {
  const willOpen = !showQueueMenu.value;
  if (willOpen) {
    showDeviceMenu.value = false;
    showUserMenu.value = false;
  }
  showQueueMenu.value = !showQueueMenu.value;
}

async function applyLocalOutput(name: string | null, sampleRate: number | null, keepOpen = false) {
  if (audioBusy.value) return;
  audioBusy.value = true;
  deviceMenuError.value = '';
  try {
    const previousRemotePeer = remoteOutputPeer.value;
    const snapshot = previousRemotePeer ? await capturePlaybackSnapshot() : null;
    await invoke('set_output_device', { name, sampleRate });
    selectedOutputDevice.value = name;
    if (previousRemotePeer && snapshot) {
      if (snapshot.track) await playTrackLocally(snapshot.track, false, snapshot.position);
      if (snapshot.playing) await invoke('remote_playback_pause', remotePeerArgs(previousRemotePeer));
      remoteOutputPeer.value = null;
      if (snapshot.playing && snapshot.track) await invoke('playback_resume');
      const state = await refreshPlaybackState();
      if (state.playing) startTicker(); else stopTicker();
      syncAndroid();
    }
    await refreshOutputDevices();
    if (!keepOpen) showDeviceMenu.value = false;
  } catch {
    deviceMenuError.value = 'Could not switch audio output. Please try again.';
  } finally { audioBusy.value = false; }
}
async function pickLocalDevice(name: string, sampleRate?: number | null, keepOpen = false) {
  const nextName = name === currentDevice.value && sampleRate === undefined && !remoteOutputPeer.value ? null : name;
  await applyLocalOutput(nextName, sampleRate ?? null, keepOpen);
}
async function pickLocalSampleRate(device: AudioDevice, sampleRate: number | null) {
  await applyLocalOutput(device.name, sampleRate, true);
}
function selectSettingsOutput(name: string) {
  void applyLocalOutput(name || null, null, true);
}
function selectSettingsSampleRate(sampleRate: number | null) {
  void applyLocalOutput(selectedOutputDevice.value, sampleRate, true);
}

async function pickRemoteDevice(peer: Peer) {
  deviceMenuError.value = '';
  try {
    if (isRemoteOutputPeer(peer)) {
      showDeviceMenu.value = false;
      return;
    }
    const previousRemotePeer = remoteOutputPeer.value;
    const snapshot = await capturePlaybackSnapshot();
    if (snapshot.track) {
      await playTrackRemotely(snapshot.track, false, snapshot.position, peer);
    }
    if (snapshot.playing) {
      if (previousRemotePeer) {
        await invoke('remote_playback_pause', remotePeerArgs(previousRemotePeer));
      } else {
        await invoke('playback_pause');
      }
      if (snapshot.track) {
        await invoke('remote_playback_resume', remotePeerArgs(peer));
      }
    }
    remoteOutputPeer.value = peer;
    const st = await refreshPlaybackState();
    if (st.playing) startTicker(); else stopTicker();
    syncAndroid();
    showDeviceMenu.value = false;
  } catch (e: any) {
    deviceMenuError.value = String(e ?? 'Failed to switch to remote player');
  }
}

function onDocClick(e: MouseEvent) {
  if (!(e.target as HTMLElement).closest(".user-menu-wrapper")) {
    showUserMenu.value = false;
  }
  if (!(e.target as HTMLElement).closest(".device-menu-wrapper")) {
    showDeviceMenu.value = false;
  }
  if (!(e.target as HTMLElement).closest(".queue-menu-wrapper")) {
    showQueueMenu.value = false;
  }
  if (!(e.target as HTMLElement).closest(".track-context-menu-wrapper")) {
    trackContextMenu.value = null;
  }
  if (!(e.target as HTMLElement).closest(".home-pinned-context-menu-wrapper")) {
    homePinnedContextMenu.value = null;
  }
  if (!(e.target as HTMLElement).closest(".add-to-playlist-menu-wrapper")) {
    addToPlaylistMenu.value = null;
  }
}

function onKeyDown(e: KeyboardEvent) {
  const target = e.target as HTMLElement | null;
  if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.closest('select, button, a, [role=switch]') || target.isContentEditable)) {
    return; // Don't trigger if typing
  }

  if (e.code === 'Escape' && trackReplaceDialog.value) {
    e.preventDefault();
    closeTrackReplaceDialog();
  } else if (e.code === 'Space') {
    e.preventDefault();
    togglePlay();
  } else if (e.code === 'ArrowLeft') {
    e.preventDefault();
    playPrev();
  } else if (e.code === 'ArrowRight') {
    e.preventDefault();
    playNext();
  }
}

function countUniqueArtists(tracks: Track[]) {
  return new Set(
    tracks
      .map((track) => track.artist?.trim())
      .filter((artist): artist is string => !!artist)
  ).size;
}

function countUniqueAlbums(tracks: Track[]) {
  return new Set(
    tracks
      .filter((track) => !!track.album?.trim())
      .map((track) => `${track.artist?.trim() ?? ''}\u0000${track.album?.trim()}`)
  ).size;
}

const filteredTracks = computed(() => {
  // Filtering is performed by SQLite before the page crosses the Tauri bridge.
  return libraryTracks.value;
});

const searchResults = computed(() => {
  return localSearchTracks.value;
});

const visibleRecentTracks = computed(() => filterDuplicateTracks(recentTracks.value));

const searchRecentTracks = computed(() => {
  if (visibleRecentTracks.value.length) return visibleRecentTracks.value.slice(0, 10);
  return filterDuplicateTracks(libraryTracks.value).slice(0, 10);
});

const searchArtistCount = computed(() => countUniqueArtists(searchResults.value));
const searchAlbumCount = computed(() => countUniqueAlbums(searchResults.value));
const visibleSoulseekResults = computed(() => soulseekResults.value.slice(0, soulseekVisibleCount.value));
const soulseekQueryDirty = computed(() => {
  const trimmed = searchQuery.value.trim();
  return !!trimmed && trimmed !== soulseekSubmittedQuery.value;
});
const canRunSoulseekSearch = computed(() => !!soulseekReady.value && !!searchQuery.value.trim() && !soulseekLoading.value);
const searchSoulseekCountLabel = computed(() => (soulseekSearching.value ? '?' : String(soulseekResults.value.length)));
const visibleTrackReplaceResults = computed(() => trackReplaceResults.value.slice(0, trackReplaceVisibleCount.value));
const trackReplaceQueryDirty = computed(() => {
  const trimmed = trackReplaceDialog.value?.query.trim() ?? '';
  return !!trimmed && trimmed !== trackReplaceSubmittedQuery.value;
});
const canRunTrackReplaceSearch = computed(() => !!soulseekReady.value && !!trackReplaceDialog.value?.query.trim() && !trackReplaceLoading.value);
const localSearchPageLabel = computed(() => {
  if (localSearchTotal.value === 0 || localSearchTracks.value.length === 0) return '0 tracks';
  const first = localSearchOffset.value + 1;
  const last = localSearchOffset.value + localSearchTracks.value.length;
  return `${first}–${last} of ${localSearchTotal.value}`;
});
const libraryPageLoading = computed(() => libraryLoading.value || localSearchLoading.value);

const groupedByArtist = computed(() => {
  const map = new Map<string, Track[]>();
  for (const t of filteredTracks.value) {
    const key = t.artist || ''; // '' = unknown artist
    if (!map.has(key)) map.set(key, []);
    map.get(key)!.push(t);
  }
  return map;
});

// Flat ordered list matching sourceList('library') – used to get correct indices for playTrackFrom
const libraryFlatList = computed<Track[]>(() => {
  const flat: Track[] = [];
  for (const [, tracks] of groupedByArtist.value) flat.push(...tracks);
  return flat.length ? flat : libraryTracks.value;
});

let libraryFilterTimer: ReturnType<typeof setTimeout> | null = null;
let localSearchTimer: ReturnType<typeof setTimeout> | null = null;
let libraryPageRequest = 0;
let localSearchRequest = 0;

watch(libraryQuery, () => {
  if (libraryFilterTimer) clearTimeout(libraryFilterTimer);
  libraryFilterTimer = setTimeout(() => {
    void loadLibraryPage({ offset: 0, limit: LIBRARY_TRACK_PAGE_SIZE });
  }, 250);
});

async function loadLibrary() {
  return loadLibraryPage({
    offset: 0,
    limit: activeNav.value === 'library' ? LIBRARY_TRACK_PAGE_SIZE : HOME_TRACK_PAGE_SIZE,
  });
}

async function loadLibraryPage({
  offset = 0,
  limit = LIBRARY_TRACK_PAGE_SIZE,
  append = false,
}: { offset?: number; limit?: number; append?: boolean } = {}) {
  const request = ++libraryPageRequest;
  const query = append
    ? libraryLoadedQuery.value
    : (activeNav.value === 'library' ? libraryQuery.value.trim() : '');
  if (append) libraryLoadingMore.value = true;
  else libraryLoading.value = true;
  try {
    const page = await invoke<TrackPage>('get_tracks_page', {
      query: query || null,
      limit,
      offset,
      includeDuplicates: showDuplicateTracks.value,
    });
    if (request !== libraryPageRequest) return;
    libraryTracks.value = append
      ? [...libraryTracks.value, ...page.tracks]
      : page.tracks;
    libraryTotal.value = page.total;
    if (!append) {
      libraryOffset.value = page.offset;
      libraryLoadedQuery.value = query;
    }
    libraryHasMore.value = page.has_more;
    libraryPageSize.value = limit;
  } catch (e) {
    if (request === libraryPageRequest) console.error('Failed to load library:', e);
  } finally {
    if (request === libraryPageRequest) {
      libraryLoading.value = false;
      libraryLoadingMore.value = false;
    }
  }
}

async function loadLocalSearchPage(offset = 0) {
  const query = searchQuery.value.trim();
  if (!query) {
    localSearchTracks.value = [];
    localSearchTotal.value = 0;
    localSearchOffset.value = 0;
    localSearchHasMore.value = false;
    return;
  }

  const request = ++localSearchRequest;
  localSearchLoading.value = true;
  try {
    const page = await invoke<TrackPage>('get_tracks_page', {
      query,
      limit: LIBRARY_TRACK_PAGE_SIZE,
      offset,
      includeDuplicates: showDuplicateTracks.value,
    });
    // Ignore a response for a query the user has already replaced.
    if (request !== localSearchRequest || query !== searchQuery.value.trim()) return;
    localSearchTracks.value = page.tracks;
    localSearchTotal.value = page.total;
    localSearchOffset.value = page.offset;
    localSearchHasMore.value = page.has_more;
  } catch (error) {
    console.error('Failed to search local library:', error);
  } finally {
    if (request === localSearchRequest && query === searchQuery.value.trim()) localSearchLoading.value = false;
  }
}

function loadMoreLibraryTracks() {
  if (!libraryHasMore.value || libraryLoading.value || libraryLoadingMore.value) return;
  void loadLibraryPage({
    offset: libraryOffset.value + libraryTracks.value.length,
    limit: libraryPageSize.value,
    append: true,
  });
}

function previousLocalSearchPage() {
  void loadLocalSearchPage(Math.max(0, localSearchOffset.value - LIBRARY_TRACK_PAGE_SIZE));
}

function nextLocalSearchPage() {
  if (localSearchHasMore.value) void loadLocalSearchPage(localSearchOffset.value + localSearchTracks.value.length);
}

function scheduleInitialAppDataLoad() {
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      void loadAppData('mount');
      // silent update check — no blocking UI, populates aboutUpdateStatus in background
      setTimeout(() => { void checkAboutUpdates(); }, 3000);
    });
  });
}

function onWindowFocus() {
  void loadAppData('resume');
}

function onDocumentVisibilityChange() {
  if (document.visibilityState === 'visible') {
    void loadAppData('resume');
  }
}

async function loadAppData(reason: 'mount' | 'resume' | 'sync' = 'mount') {
  const shouldLoadLibrary = reason !== 'resume' || libraryTracks.value.length === 0;
  await Promise.allSettled([
    shouldLoadLibrary ? loadLibrary() : Promise.resolve(),
    loadRecent(),
    loadPlaylists(),
    loadSmartPlaylists(),
  ]);

}

async function loadRecent() {
  try {
    recentTracks.value = await invoke<Track[]>('get_recent_tracks', { limit: 12 });
  } catch (_) {}
}

async function loadHistory() {
  historyLoading.value = true;
  try {
    historyEntries.value = await invoke<PlayHistoryEntry[]>('get_play_history');
  } catch (_) {} finally {
    historyLoading.value = false;
  }
}

async function loadAbout(force = false) {
  if (aboutLoading.value || (aboutInfo.value && !force)) return;

  aboutLoading.value = true;
  aboutError.value = '';
  try {
    aboutInfo.value = await invoke<AboutInfo>('about_info');
  } catch (error) {
    aboutError.value = String(error ?? 'Failed to load About data');
  } finally {
    aboutLoading.value = false;
  }
}

async function checkAboutUpdates() {
  await loadAbout();
  aboutCheckingUpdates.value = true;
  aboutUpdateError.value = '';
  aboutUpdateDone.value = '';
  aboutDesktopUpdateReady.value = false;

  try {
    if (supportsTauriUpdater.value) {
      if (pendingDesktopUpdate) {
        await pendingDesktopUpdate.close();
        pendingDesktopUpdate = null;
      }

      const update = await check({ timeout: 10_000 });
      pendingDesktopUpdate = update;
      aboutDesktopUpdateReady.value = update !== null;

      const currentVersion = aboutInfo.value?.current_version ?? update?.currentVersion ?? '';
      const repo = aboutInfo.value?.release_repo;
      aboutUpdateStatus.value = update
        ? {
            current_version: update.currentVersion,
            latest_version: update.version,
            has_update: true,
            release_url: repo
              ? `https://github.com/${repo}/releases/tag/v${update.version}`
              : null,
            release_notes: update.body ?? null,
            asset_url: null,
            checked_repo: repo ?? null,
            message: `Update available: ${update.version}`,
          }
        : {
            current_version: currentVersion,
            latest_version: currentVersion,
            has_update: false,
            release_url: null,
            release_notes: null,
            asset_url: null,
            checked_repo: repo ?? null,
            message: 'You already have the latest release.',
          };
    } else {
      aboutUpdateStatus.value = await invoke<AboutUpdateStatus>('about_check_updates');
    }
  } catch (error) {
    if (pendingDesktopUpdate) {
      await pendingDesktopUpdate.close().catch(() => {});
      pendingDesktopUpdate = null;
    }
    aboutUpdateStatus.value = null;
    aboutUpdateError.value = String(error ?? 'Failed to check updates');
  } finally {
    aboutCheckingUpdates.value = false;
  }
}

async function openAboutDownloadLink() {
  const url = aboutUpdateStatus.value?.release_url;
  if (!url) return;

  try {
    await openUrl(url);
  } catch (error) {
    console.error('Failed to open update link:', error);
  }
}

async function doAboutUpdate() {
  if (supportsTauriUpdater.value) {
    const update = pendingDesktopUpdate;
    if (!update) {
      aboutUpdateError.value = 'Update metadata expired. Check for updates again.';
      return;
    }

    aboutUpdating.value = true;
    aboutUpdateDone.value = '';
    aboutUpdateError.value = '';
    aboutUpdateProgress.value = 0;
    let downloaded = 0;
    let total: number | undefined;

    try {
      await update.downloadAndInstall((event) => {
        if (event.event === 'Started') {
          total = event.data.contentLength;
          aboutUpdateProgress.value = 0;
        } else if (event.event === 'Progress') {
          downloaded += event.data.chunkLength;
          aboutUpdateProgress.value = total
            ? Math.min(100, Math.round((downloaded / total) * 100))
            : null;
        } else {
          aboutUpdateProgress.value = 100;
        }
      }, { timeout: 5 * 60_000 });

      pendingDesktopUpdate = null;
      aboutDesktopUpdateReady.value = false;
      aboutUpdateDone.value = 'Update installed. Restarting…';
      await relaunch();
    } catch (error) {
      aboutUpdateError.value = String(error ?? 'Update failed');
    } finally {
      aboutUpdating.value = false;
      aboutUpdateProgress.value = null;
    }
    return;
  }

  const assetUrl = aboutUpdateStatus.value?.asset_url;
  if (!assetUrl) {
    await openAboutDownloadLink();
    return;
  }
  aboutUpdating.value = true;
  aboutUpdateDone.value = '';
  aboutUpdateError.value = '';
  aboutUpdateProgress.value = null;
  const unlisten = await listen<{ downloaded: number; total: number | null; percent: number | null }>(
    'about-update-progress',
    (event) => { aboutUpdateProgress.value = event.payload.percent ?? null; }
  );
  try {
    const path = await invoke<string>('about_do_update', { assetUrl });
    aboutUpdateDone.value = path;
  } catch (error) {
    aboutUpdateError.value = String(error ?? 'Update failed');
  } finally {
    unlisten();
    aboutUpdating.value = false;
    aboutUpdateProgress.value = null;
  }
}

function formatHistoryDate(ts: number): string {
  const d = new Date(ts * 1000);
  const now = new Date();
  const isToday = d.toDateString() === now.toDateString();
  const yesterday = new Date(now); yesterday.setDate(now.getDate() - 1);
  const isYesterday = d.toDateString() === yesterday.toDateString();
  const time = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  if (isToday) return `Today, ${time}`;
  if (isYesterday) return `Yesterday, ${time}`;
  return d.toLocaleDateString([], { day: 'numeric', month: 'short' }) + ', ' + time;
}

provideTrackActions({
  isCurrent: isCurrentTrack,
  isNext: isNextTrack,
  toggleLike: (track) => void toggleLike(track),
  identify: (track) => void identifySingle(track),
  edit: openEditor,
  openAddToPlaylist: openAddToPlaylistMenuAt,
  openMenu: openTrackContextMenuAt,
  openArtist,
  openAlbum,
});


function openEditor(track: Track) {
  editForm.value = {
    title: track.title || '',
    artist: track.artist || '',
    album: track.album || '',
    album_artist: track.album_artist || '',
    track_number: track.track_number,
    disc_number: track.disc_number,
    year: track.year,
    genre: track.genre || '',
    tags: track.tags || '',
    play_count: track.play_count,
    is_liked: track.is_liked,
    date_added: trackDateInputValue(track.date_added),
    rarity: track.rarity || '',
  };
  editingTrack.value = track;
}

async function saveTrack() {
  const activeTrack = editingTrack.value;
  if (!activeTrack) return;
  const activePlaylistId = playlistView.value?.id ?? null;
  const savedTrack = await invoke<Track>('update_track', {
    id: activeTrack.id,
    title: editForm.value.title || null,
    artist: editForm.value.artist || null,
    album: editForm.value.album || null,
    albumArtist: editForm.value.album_artist.trim(),
    trackNumber: normalizeOptionalInteger(editForm.value.track_number),
    discNumber: normalizeOptionalInteger(editForm.value.disc_number),
    year: normalizeOptionalInteger(editForm.value.year),
    genre: editForm.value.genre.trim() || null,
    tags: normalizeTrackTagsInput(editForm.value.tags),
    playCount: normalizeNonNegativeInteger(editForm.value.play_count),
    isLiked: editForm.value.is_liked,
    dateAdded: parseTrackDateInput(editForm.value.date_added),
    rarity: editForm.value.rarity.trim() || null,
  });
  editingTrack.value = null;
  await followEditedTrackFromAlbumPage(activeTrack, savedTrack);
  libraryVersion.value++; // so an open artist/album page reloads too
  await loadLibrary();
  await loadRecent();
  if (activePlaylistId !== null && playlistView.value?.id === activePlaylistId) {
    await loadPlaylistPage(activePlaylistId, playlistView.value.name);
  }
  if (nowPlaying.value?.id === activeTrack.id) {
    nowPlaying.value = savedTrack;

  }
}

async function openSettingsFolder() {
  settingsActionError.value = '';
  try {
    await ensureLibraryDataDir();
    await openDataDir();
  } catch { settingsActionError.value = 'Could not open the music folder. Please try again.'; }
}
// Editing a track on an album page can move it to another album (a new album
// name or album artist).
// The page stays while its album still has tracks; once it's empty, it
// follows the edited track instead of showing nothing.
async function followEditedTrackFromAlbumPage(before: Track, after: Track) {
  if (activeNav.value !== 'album') return;
  const oldAlbum = before.album ?? '';
  const oldArtist = before.album_artist_or_artist ?? '';
  const newAlbum = after.album ?? '';
  const newArtist = after.album_artist_or_artist ?? '';
  if (oldAlbum !== navTarget.value || oldArtist !== navTargetArtist.value) return;
  if ((oldAlbum === newAlbum && oldArtist === newArtist) || !newAlbum) return;
  if (await countAlbumTracks(oldAlbum, oldArtist, showDuplicateTracks.value) > 0) return;
  replaceAlbumNavEntry(newAlbum, newArtist);
}

// Leaving the album artist field empty means the track artist.
const albumArtistPlaceholder = computed(() => editForm.value.artist || 'Same as artist');

async function openDataDir() {
  const path = await invoke<string>('get_data_dir');
  // On Android, use the native JavascriptInterface bridge that calls
  // Intent(ACTION_VIEW).setDataAndType(safUri, "vnd.android.document/directory")
  const sdcardPrefixes = ['/sdcard/', '/storage/emulated/0/'];
  const matched = sdcardPrefixes.find(p => path.startsWith(p));
  if (matched && (window as any).AndroidBridge) {
    const relative = path.slice(matched.length);
    (window as any).AndroidBridge.openFolder(relative);
    return;
  }
  // Desktop fallback
  try {
    await invoke('reveal_track_in_folder', {
      path,
      absolute: true,
    });
  } catch (_) {
    try { await navigator.clipboard.writeText(path); } catch (_2) {}
    alert(`Data folder:\n${path}`);
  }
}

async function doReindex() {
  if (libraryScanBusy.value) return;
  libraryScanBusy.value = true;
  settingsActionError.value = '';
  indexRunning.value = true;
  indexDone.value = false;
  indexCurrent.value = 0;
  indexTotal.value = 0;
  try { await invoke('reindex'); }
  catch {
    indexRunning.value = false;
    libraryScanBusy.value = false;
    settingsActionError.value = 'Could not start the library scan. Please try again.';
  }
}
async function identifyFromSettings() {
  settingsActionError.value = '';
  settingsActionMessage.value = '';
  try { await startIdentify(); }
  catch { settingsActionError.value = 'Could not load library tracks for identification.'; }
}

async function startIdentify() {
  if (identifyPreparing.value || (identifyRunning.value && !identifyDone.value)) return;
  identifyPreparing.value = true;
  let tracks: Track[];
  try { tracks = await invoke<Track[]>('get_all_tracks'); }
  finally { identifyPreparing.value = false; }
  const ids = tracks.filter(t => !t.manually_edited).map(t => t.id);
  if (!ids.length) {
    settingsActionMessage.value = 'There are no tracks to identify. Manually edited tracks are kept as they are.';
    return;
  }
  identifyResults.value = [];
  identifyCurrent.value = 0;
  identifyTotal.value = ids.length;
  identifyRunning.value = true;
  identifyMinimized.value = false;
  identifyDone.value = false;
  try {
    await invoke('identify_tracks', { ids });
  } catch (e: any) {
    identifyDone.value = true;
    identifyResults.value.push({
      track_id: 0,
      track_name: null,
      status: 'error',
      message: String(e ?? 'Failed to start identify'),
    });
  }
}

async function identifySingle(track: Track) {
  identifyResults.value = [];
  identifyCurrent.value = 0;
  identifyTotal.value = 1;
  identifyRunning.value = true;
  identifyMinimized.value = false;
  identifyDone.value = false;
  try {
    await invoke('identify_tracks', { ids: [track.id] });
  } catch (e: any) {
    identifyDone.value = true;
    identifyResults.value.push({
      track_id: track.id,
      track_name: track.title || track.path,
      status: 'error',
      message: String(e ?? 'Failed to start identify'),
    });
  }
}

function identifyStatusIcon(status: string) {
  if (status === 'found') return '\u2714';
  if (status === 'not_found') return '\u2013';
  if (status === 'error') return '\u2716';
  if (status === 'fingerprinting') return '\u266B';
  if (status === 'looking_up') return '\u21BB';
  if (status === 'done') return '\u2605';
  return '\u00B7';
}

watch(activeNav, (nav) => {
  if (nav === 'settings/playback') void refreshOutputDevices();
  if (isSettingsNav.value) {
    settingsActionError.value = '';
    settingsActionMessage.value = '';
    void ensureLibraryDataDir().catch(() => { settingsActionError.value = 'Could not load the music folder. Open it to try again.'; });
    nextTick(() => { if (contentRef.value) contentRef.value.scrollTop = 0; });
  }
  if (nav === 'about' || nav === 'settings/about') {
    void loadAbout();
  }
  if (nav === 'library' && libraryPageSize.value !== LIBRARY_TRACK_PAGE_SIZE) {
    void loadLibraryPage({ offset: 0, limit: LIBRARY_TRACK_PAGE_SIZE });
  }
});

const pageTitle = computed(() => {
  if (isSettingsNav.value || activeNav.value === 'about') return 'Settings';
  switch (activeNav.value) {
    case 'home': return 'Home';
    case 'library': return 'Your Library';
    case 'search': return 'Search';
    case 'track-replace': return 'Search and replace';
    case 'history': return 'Play History';
    case 'playlists':
      return playlistTab.value === 'regular'
        ? playlistView.value?.name || 'Playlists'
        : editingSP.value?.name || smartView.value?.name || 'Flexible Playlists';
    case 'dedup': return 'Duplicates';
    case 'discovery': return 'Devices on Network';
    default: return undefined;
  }
});
const pageSubtitle = computed(() => activeNav.value === 'track-replace' && trackReplaceDialog.value
  ? `Replace ${trackReplaceDialog.value.track.title || trackReplaceDialog.value.track.path} with a Soulseek match.`
  : undefined);

onMounted(() => {
  document.addEventListener('click', onDocClick);
  document.addEventListener('keydown', onKeyDown);
  window.addEventListener('focus', onWindowFocus);
  document.addEventListener('visibilitychange', onDocumentVisibilityChange);
  void loadSoulseekStatus();
  invoke<DeviceSettings>('get_device_settings')
    .then((cfg) => {
      if (cfg?.emoji) deviceEmoji.value = cfg.emoji;
      syncEnabled.value = !!cfg?.sync_enabled;
    })
    .catch(() => {});
  ensureLibraryDataDir().catch(() => {});
  void getCurrentWebviewWindow()
    .onDragDropEvent((event) => {
      if (event.payload.type !== 'drop' || event.payload.paths.length === 0) return;
      invoke<{ imported: number; skipped: number }>('import_dropped_files', {
        paths: event.payload.paths,
      }).catch((error) => {
        console.error('Failed to import dropped files:', error);
      });
    })
    .then((unlisten) => {
      unlistenDragDrop = unlisten;
    })
    .catch((error) => {
      console.error('Failed to register drag-drop listener:', error);
    });
  listen<SoulseekDownloadEvent>('soulseek-download', (e) => {
    const payload = e.payload;
    const key = soulseekResultKey(payload.username, payload.filename);
    const matchingResult = findSoulseekResultByKey(key);
    soulseekDownloads.value = {
      ...soulseekDownloads.value,
      [key]: payload,
    };

    if (payload.state === 'completed' && payload.localPath && soulseekPendingPlayback.has(key)) {
      soulseekPendingPlayback.delete(key);
      if (matchingResult) {
        void playDownloadedSoulseekResult(matchingResult, payload.localPath).catch((error) => {
          console.error('Failed to start Soulseek playback:', error);
        });
      }
    }

    if (payload.state === 'completed' && payload.localPath) {
      void maybeReplaceActiveSoulseekPreviewWithLibraryTrack(payload.username, payload.filename, payload.localPath).catch((error) => {
        console.error('Failed to replace active Soulseek preview with library track:', error);
      });
    } else if (['failed', 'timed_out', 'cancelled'].includes(payload.state)) {
      soulseekPendingPlayback.delete(key);
    }

    const pendingTrackId = soulseekPendingTrackReplacement.value[key];
    if (pendingTrackId != null) {
      if (payload.state === 'completed' && payload.localPath) {
        const resultForReplacement = matchingResult ?? {
          username: payload.username,
          filename: payload.filename,
        };
        void performTrackReplacement(pendingTrackId, resultForReplacement, payload.localPath).catch((error) => {
          console.error('Failed to replace track from Soulseek download:', error);
        });
      } else if (['failed', 'timed_out', 'cancelled'].includes(payload.state)) {
        trackReplaceActionError.value = payload.error || 'Failed to download replacement track';
        clearPendingTrackReplacement(key);
      }
    }

    if (matchingResult && soulseekPendingPreviewPlayback.has(key)) {
      void maybeStartSoulseekPreviewPlayback(matchingResult).catch((error) => {
        console.error('Failed to start Soulseek preview from download:', error);
      });
    }
  })
    .then((unlisten) => {
      unlistenSoulseekDownload = unlisten;
    })
    .catch(() => {});

  listen<SoulseekDownloadEvent>('soulseek-preview', (e) => {
    const payload = e.payload;
    const key = soulseekResultKey(payload.username, payload.filename);
    const matchingResult = findSoulseekResultByKey(key);
    soulseekPreviews.value = {
      ...soulseekPreviews.value,
      [key]: payload,
    };

    if (payload.state === 'completed') {
      updateSoulseekPreviewTrackState(payload.username, payload.filename, false);
    }

    if (soulseekPendingPreviewPromotion.has(key)) {
      if (payload.state === 'completed' && payload.localPath) {
        const resultForPromotion = matchingResult ?? {
          username: payload.username,
          filename: payload.filename,
          basename: payload.basename,
          size: payload.totalBytes ?? 0,
          coverFilename: null,
        };
        void promoteSoulseekPreviewToLibrary(resultForPromotion, payload.localPath).catch((error) => {
          if (soulseekPendingTrackReplacement.value[key] != null) {
            trackReplaceActionError.value = String(error ?? 'Failed to save replacement track');
            clearPendingTrackReplacement(key);
          }
          console.error('Failed to promote Soulseek preview to library:', error);
        });
      } else if (['failed', 'timed_out', 'cancelled'].includes(payload.state)) {
        soulseekPendingPreviewPromotion.delete(key);
        soulseekDownloads.value = {
          ...soulseekDownloads.value,
          [key]: payload,
        };
        if (soulseekPendingTrackReplacement.value[key] != null) {
          trackReplaceActionError.value = payload.error || 'Failed to save replacement track';
          clearPendingTrackReplacement(key);
        }
      } else {
        soulseekDownloads.value = {
          ...soulseekDownloads.value,
          [key]: {
            ...payload,
            totalBytes: payload.totalBytes ?? soulseekDownloads.value[key]?.totalBytes ?? payload.totalBytes,
            error: null,
          },
        };
      }
    }

    if (matchingResult && soulseekPendingPreviewPlayback.has(key)) {
      void maybeStartSoulseekPreviewPlayback(matchingResult).catch((error) => {
        console.error('Failed to start Soulseek preview:', error);
      });
    }

    if (['failed', 'timed_out', 'cancelled'].includes(payload.state)) {
      soulseekPendingPreviewPlayback.delete(key);
      if (nowPlaying.value?.id === soulseekTrackIdFromParts(payload.username, payload.filename) && nowPlaying.value.soulseek_preview) {
        void stopCurrentOutput()
          .then(() => {
            isPlaying.value = false;
            stopTicker();
            syncAndroid();
          })
          .catch(() => {});
      }
    }
  })
    .then((unlisten) => {
      unlistenSoulseekPreview = unlisten;
    })
    .catch(() => {});
  
  // Listen for app coming back to foreground (Android)
  listen('tauri://resumed', async () => {
    void loadAppData('resume');
    const transitionSequence = playbackTransitionSequence;
    const st = await refreshPlaybackState();
    if (st.finished) {
      await handleFinishedPlayback(transitionSequence);
      return;
    }
    if (st.playing && !ticker) {
      startTicker();
    }
  });

  // Rust emits this when decode thread reaches EOF — works even when JS timers are throttled (Android background)
  listen('playback-finished', async () => {
    await handleFinishedPlayback(playbackTransitionSequence);
  });

  // Another device took control of this one: mirror its playback state in the UI
  listen<PeerPlayback | null>('remote-playback', (e) => {
    const p = e.payload;
    if (!p || p.state === 'stopped') {
      remoteControlled.value = false;
      isPlaying.value = false;
      stopTicker();
      return;
    }
    remoteControlled.value = true;
    const track = trackForHash(p.hash);
    if (track && nowPlaying.value?.id !== track.id) {
      nowPlaying.value = track;
    }
    isPlaying.value = p.state === 'playing';
    currentTime.value = Math.floor(p.position ?? 0);
    if ((p.duration ?? 0) > 0) {
      duration.value = Math.floor(p.duration ?? 0);
    }
    if (p.volume != null) {
      volume.value = Math.round(p.volume * 100);
    }
    if (p.state === 'playing') {
      startTicker();
    } else {
      stopTicker();
    }
  });
  
  listen('library-changed', () => {
    libraryVersion.value++;
    void loadAppData('sync');
  })
    // Do the first fetch only after this subscription is active. Otherwise a
    // short initial indexing run can finish between the fetch and subscription,
    // leaving Home with a stale empty snapshot until the user navigates.
    .then(() => scheduleInitialAppDataLoad())
    .catch(() => scheduleInitialAppDataLoad());
  listen<number>('beat', (e) => {
    const lag = Math.max(0, Date.now() - e.payload);
    startBeatAnimation(lag);
  });
  listen<Peer[]>('discovery-peers', (e) => {
    const prev = new Set(peers.value.map(p => p.host));
    peers.value = e.payload;
    if (syncEnabled.value) {
      for (const peer of e.payload) {
        if (!prev.has(peer.host)) {
          invoke('sync_with_peer', { peerHost: peer.host, peerName: peer.name, peerAddresses: peer.addresses, peerPort: peer.port }).catch(() => {});
        }
      }
    }
  });
  const syncSessions: Record<string, LogSession> = {};

  listen<SyncProgress>('sync-progress', (e) => {
    const p = e.payload;
    syncProgress.value = { ...syncProgress.value, [p.peer]: p };
    if (p.device_name) peerDeviceNames.value = { ...peerDeviceNames.value, [p.peer]: p.device_name! };

    // Create a session entry on first event for this peer
    if (!syncSessions[p.peer]) {
      const session: LogSession = {
        id: crypto.randomUUID(), kind: 'sync',
        device: p.device_name || p.peer, emoji: p.device_emoji || undefined,
        addedCount: 0, filesAdded: 0, files: [],
        startedAt: Date.now(), status: 'running',
      };
      syncSessions[p.peer] = session;
      indexLog.value.push(session);
    }
    const session = syncSessions[p.peer];

    // Update device label once we get it
    if (p.device_name) { session.device = p.device_name; session.emoji = p.device_emoji || undefined; }
    session.phase = p.phase;
    session.total = p.total;
    session.currentMessage = syncStatusText(p);
    if (p.phase === 'reindex') {
      suppressLocalIndexLogUntil = Date.now() + 30_000;
    }

    if (p.phase === 'download') {
      const previousDone = session.filesAdded;
      session.filesAdded = p.done;
      if (p.message && p.done > previousDone && session.files[session.files.length - 1] !== p.message) {
        addSessionFile(session, p.message);
      }
      indexLog.value = [...indexLog.value]; // trigger reactivity
    } else if (p.phase === 'done') {
      const addedCount = parseSyncAddedCount(p.message);
      session.filesAdded = addedCount ?? (p.message?.includes('Library metadata synced') ? 0 : session.files.length);
      session.addedCount = session.filesAdded;
      if (session.filesAdded === 0) session.files = [];
      session.currentMessage = p.message || 'Sync complete';
      session.status = 'done';
      session.finishedAt = Date.now();
      suppressLocalIndexLogUntil = 0;
      indexLog.value = [...indexLog.value];
      delete syncSessions[p.peer];
      void loadAppData('sync');
      loadHistory();
    } else if (p.phase === 'error') {
      session.status = 'error';
      session.errorMsg = p.message || 'Connection failed';
      session.currentMessage = session.errorMsg;
      session.finishedAt = Date.now();
      suppressLocalIndexLogUntil = 0;
      indexLog.value = [...indexLog.value];
      delete syncSessions[p.peer];
    } else {
      indexLog.value = [...indexLog.value];
    }
    nextTick(() => { if (indexLogRef.value) indexLogRef.value.scrollTop = indexLogRef.value.scrollHeight; });
  });

  // Android media controls: _mediaControl is called by the native notification buttons
  (window as any)._mediaControl = async (action: string) => {
    if (action === 'toggle')     { await toggleFromMediaControl(); }
    else if (action === 'play')       { await playFromMediaControl(); }
    else if (action === 'pause') { await pauseFromMediaControl(); }
    else if (action === 'next')  { await playNext(); }
    else if (action === 'prev')  { await playPrev(); }
    else if (action.startsWith('seek:')) {
      const pos = parseFloat(action.slice(5));
      if (!isNaN(pos)) {
        currentTime.value = Math.round(pos);
        await seekCurrentOutput(pos);
        syncAndroid();
      }
    }
  };
  let localSession: LogSession | null = null;

  listen<{current: number; total: number; status: string; added: number; track_name?: string | null}>('index-progress', (e) => {
    const p = e.payload;
    const suppressLocalLog = Date.now() < suppressLocalIndexLogUntil;
    const ensureLocalSession = () => {
      if (suppressLocalLog) return;
      if (!localSession) {
        localSession = { id: crypto.randomUUID(), kind: 'local', addedCount: 0, filesAdded: 0, files: [], startedAt: Date.now(), status: 'running' };
        indexLog.value.push(localSession);
      }
    };
    if (p.status === 'indexing' || p.status === 'scanning') {
      libraryScanBusy.value = true;
      ensureLocalSession();
      indexCurrent.value = p.current;
      indexTotal.value = p.total;
      indexRunning.value = true;
      indexDone.value = false;
      if (indexDismissTimer) { clearTimeout(indexDismissTimer); indexDismissTimer = null; }
    } else if (p.status === 'added') {
      ensureLocalSession();
      if (localSession) {
        localSession.addedCount++;
        localSession.filesAdded++;
        addSessionFile(localSession, p.track_name);
        indexLog.value = [...indexLog.value];
      }
    } else if (p.status === 'done') {
      libraryScanBusy.value = false;
      indexAdded.value = p.added;
      indexDone.value = true;
      indexRunning.value = true;
      ensureLocalSession();
      if (localSession) {
        localSession.addedCount = p.added;
        localSession.filesAdded = p.added;
        localSession.status = 'done';
        localSession.finishedAt = Date.now();
        indexLog.value = [...indexLog.value];
        localSession = null;
      }
      // Keep Home current even if a platform delivers progress but misses the
      // separate library-changed event during app startup.
      void loadAppData('sync');
    }
    nextTick(() => { if (indexLogRef.value) indexLogRef.value.scrollTop = indexLogRef.value.scrollHeight; });
  });
  listen<{current: number; total: number; track_id: number; track_name: string | null; status: string; message: string | null}>('identify-progress', (e) => {
    const p = e.payload;
    identifyCurrent.value = p.current;
    identifyTotal.value = p.total;
    if (p.status === 'done') {
      identifyDone.value = true;
      identifyResults.value.push({ track_id: 0, track_name: null, status: 'done', message: `Finished: ${p.total} tracks processed` });
      loadLibrary();
    } else {
      identifyResults.value.push({ track_id: p.track_id, track_name: p.track_name, status: p.status, message: p.message });
    }
    nextTick(() => {
      if (identifyLogRef.value) identifyLogRef.value.scrollTop = identifyLogRef.value.scrollHeight;
    });
  });
});
onUnmounted(() => {
  document.documentElement.classList.remove('player-reduced-motion', 'player-no-rarity-motion');
  document.removeEventListener('click', onDocClick);
  document.removeEventListener('keydown', onKeyDown);
  window.removeEventListener('focus', onWindowFocus);
  document.removeEventListener('visibilitychange', onDocumentVisibilityChange);
  delete (window as any)._playbackFinished;
  clearTrackLongPress();
  clearHomePinnedLongPress();
  if (libraryFilterTimer) clearTimeout(libraryFilterTimer);
  if (localSearchTimer) clearTimeout(localSearchTimer);
  if (unlistenSoulseekDownload) unlistenSoulseekDownload();
  if (unlistenSoulseekPreview) unlistenSoulseekPreview();
  if (unlistenDragDrop) unlistenDragDrop();
  if (beatRafId !== null) cancelAnimationFrame(beatRafId);
  for (const timeoutId of scheduledBeatTimeoutIds) {
    clearTimeout(timeoutId);
  }
  scheduledBeatTimeoutIds.clear();
  if (_mouseRafId) cancelAnimationFrame(_mouseRafId);
  cancelAnimationFrame(cardSpringRaf);
  stopAmbient();
  stopSpectrumPolling();
  stopTicker();
  if (pendingDesktopUpdate) {
    void pendingDesktopUpdate.close();
    pendingDesktopUpdate = null;
  }
});
</script>

<template>
  <div class="app">
    <Transition name="nav-overlay">
      <div v-if="showMobileNav" class="mobile-nav-overlay" @click="showMobileNav = false" />
    </Transition>
    <AppSidebar
      :active-nav="activeNav"
      :playlist-tab="playlistTab"
      :open="showMobileNav"
      :sync-active="syncActive"
      :peers-count="peers.length"
      :update-available="hasAvailableUpdate"
      @close="showMobileNav = false"
      @navigate-home="activeNav = 'home'; showMobileNav = false"
      @navigate-search="activeNav = 'search'; showMobileNav = false"
      @navigate-library="activeNav = 'library'; showMobileNav = false"
      @navigate-playlists="activeNav = 'playlists'; playlistView = null; playlistTab = 'regular'; showMobileNav = false"
      @navigate-smart-playlists="activeNav = 'playlists'; playlistTab = 'smart'; playlistView = null; editingSP = null; smartView = null; showMobileNav = false"
      @navigate-discovery="activeNav = 'discovery'; showMobileNav = false"
      @navigate-settings="openSettings()"
    />

    <!-- Main -->
    <main class="main">
      <AppTopbar
        :title="pageTitle"
        :subtitle="pageSubtitle"
        :can-nav-back="canNavBack"
        :can-nav-forward="canNavForward"
        @open-menu="showMobileNav = true"
        @nav-back="navBack"
        @nav-forward="navForward"
      >
        <template v-if="activeNav === 'playlists' && playlistTab === 'smart' && editingSP" #title>
          <input v-model="editingSP.name" class="page-title-input" aria-label="Playlist name" placeholder="Playlist name" @input="saveSP()" @click.stop />
        </template>
      </AppTopbar>

      <div ref="contentRef" class="content" @scroll.passive="maybeLoadMoreSoulseekResults">
        <!-- Home view -->
        <template v-if="activeNav === 'home'">
          <section>
            <div class="section-head">
              <h2>{{ !libraryLoading && libraryTotal === 0 ? 'Get started' : 'Recently played' }}</h2>
              <a class="show-all" href="#" @click.prevent="activeNav = 'history'; loadHistory()">Show all</a>
            </div>
            <div v-if="libraryLoading" class="library-empty">Loading…</div>
            <div
              v-else-if="visibleRecentTracks.length === 0 && libraryTotal === 0"
              class="card-list recent-onboarding-list"
              v-horizontal-scroll
            >
              <button type="button" class="card onboarding-music-card" @click="startOnboardingSync">
                <div class="cover onboarding-action-cover onboarding-sync-cover">
                  <AppIcon name="network" />
                  <span class="hover-play">
                    <span class="green-circle onboarding-card-go"><AppIcon name="go" :size="22" /></span>
                  </span>
                </div>
                <div class="card-title">Sync</div>
                <div class="card-artist">From another device</div>
              </button>

              <button type="button" class="card onboarding-music-card" @click="openDataDir">
                <div class="cover onboarding-action-cover onboarding-upload-cover">
                  <AppIcon name="folder" />
                  <span class="hover-play">
                    <span class="green-circle onboarding-card-go"><AppIcon name="go" :size="22" /></span>
                  </span>
                </div>
                <div class="card-title">Upload</div>
                <div class="card-artist">From a folder on disk</div>
              </button>

              <div class="recent-onboarding-guide" aria-hidden="true">
                <img class="recent-onboarding-zippy" src="/zippi-sprites/zippi-introduce.webp" alt="" />
              </div>
            </div>
            <div v-else class="card-list" v-horizontal-scroll>
              <div v-for="(track, idx) in (visibleRecentTracks.length ? visibleRecentTracks : filterDuplicateTracks(libraryTracks).slice(0, 12))" :key="track.id + '-' + idx"
                class="card" :class="rarityClass(track.rarity)" :style="rarityVars(track.rarity)"
                @click="playRecentCard(idx)"
                @contextmenu.prevent="openTrackContextMenu($event, track)"
                @touchstart.passive="startTrackRowLongPress($event, track)"
                @touchmove.passive="moveTrackRowLongPress"
                @touchend="endTrackRowLongPress"
                @touchcancel="endTrackRowLongPress">
                <CoverImage class="cover" :src="coverUrl(track.cover_hash, 'medium')" :placeholder="placeholderCoverColors(track.file_hash)">
                  <div class="hover-play">
                    <button class="green-circle" type="button" @click.stop="toggleRecentCardPlayback(idx, track.id)">
                      <AppIcon name="play" v-if="!isTrackPlaying(track.id)" :size="18" />
                      <AppIcon name="pause" v-else :size="18" />
                    </button>
                  </div>
                </CoverImage>
                <div class="card-title">{{ track.title || track.path }}</div>
                <div class="card-artist">{{ artistLabel(track.artist) }}</div>
              </div>
            </div>
          </section>

          <section v-if="hasHomePlaylistCandidates">
            <div class="section-head">
              <h2>Pinned Playlists</h2>
              <a
                class="show-all"
                href="#"
                @click.prevent="activeNav = 'playlists'; playlistTab = 'regular'; playlistView = null; smartView = null; editingSP = null"
              >Show all</a>
            </div>
            <div v-if="homePinnedItems.length === 0" class="library-empty">Pin playlists from Playlists or Flexible Playlists to keep them here.</div>
            <div v-else class="card-list" v-horizontal-scroll>
              <div
                v-for="item in homePinnedItems"
                :key="item.key"
                class="card"
                @click="openHomePinnedCard(item)"
                @contextmenu.prevent="openHomePinnedContextMenu($event, item)"
                @touchstart.passive="startHomePinnedCardLongPress($event, item)"
                @touchmove.passive="moveHomePinnedCardLongPress"
                @touchend="endHomePinnedCardLongPress"
                @touchcancel="endHomePinnedCardLongPress"
              >
                <div class="cover" :style="homePinnedCoverStyle(item)">
                  <div class="hover-play">
                    <button v-if="item.trackCount" class="green-circle" type="button" @click.stop="playHomePinnedCard(item)">
                      <AppIcon name="play" :size="18" />
                    </button>
                  </div>
                  <AppIcon name="playlist" v-if="item.kind === 'regular'" :size="30" class="pinned-cover-icon" />
                  <AppIcon name="smart-playlist" v-else :size="30" class="pinned-cover-icon" />
                </div>
                <div class="card-title">{{ item.name }}</div>
                <div class="card-artist">{{ item.subtitle }}</div>
              </div>
            </div>
          </section>

          <section v-for="item in homePinnedSections" :key="`${item.key}:preview`">
            <div class="section-head">
              <h2>{{ item.name }}</h2>
              <a class="show-all" href="#" @click.prevent="openHomePinnedItem(item)">Show all</a>
            </div>
            <div v-if="item.previewTracks.length === 0" class="library-empty">No tracks in this playlist yet.</div>
            <div v-else class="track-list">
              <TrackRow
                v-for="(track, idx) in item.previewTracks"
                :key="`${item.key}:${track.id}`"
                :track="track"
                show-genre
                @play="playPlaylistTrack(item.tracks, idx)"
              />
            </div>
            <div v-if="item.tracks.length > item.previewTracks.length" class="library-empty home-playlist-more">… and {{ item.tracks.length - item.previewTracks.length }} more</div>
          </section>

          <section v-if="libraryTotal > 0">
            <div class="section-head">
              <h2>Made For You</h2>
              <a class="show-all" href="#">Show all</a>
            </div>
            <div class="card-list" v-horizontal-scroll>
              <div v-for="item in madeForYou" :key="item.id" class="card">
                <div class="cover" :style="`background: linear-gradient(135deg, ${item.colors[0]}, ${item.colors[1]})`">
                  <div class="hover-play">
                    <button class="green-circle">
                      <AppIcon name="play" :size="18" />
                    </button>
                  </div>
                </div>
                <div class="card-title">{{ item.title }}</div>
                <div class="card-artist">{{ item.artist }}</div>
              </div>
            </div>
          </section>
        </template>

        <!-- Library view -->
        <template v-else-if="activeNav === 'library'">
          <section>
            <div class="library-header page-toolbar">
              <input
                v-model="libraryQuery"
                class="library-search"
                type="text"
                placeholder="Filter tracks..."
              />
            </div>

            <div v-if="libraryLoading" class="library-empty">Loading…</div>
            <div v-else-if="filteredTracks.length === 0" class="library-empty">
              No tracks found. Add music to the data directory and it will appear here.
            </div>

            <div v-else class="track-groups">
              <div v-for="[artist, tracks] in groupedByArtist" :key="artist" class="track-group">
                <div class="group-artist">
                  <a v-if="hasArtistPage(artist)" class="group-artist-link" href="#" @click.prevent="openArtist(artist)">{{ artist }}</a>
                  <template v-else>{{ artistLabel(artist) }}</template>
                </div>
                <TrackRow
                  v-for="track in tracks"
                  :key="track.id"
                  :track="track"
                  show-number
                  :show-artist="false"
                  @play="playLibraryTrack(libraryFlatList.indexOf(track))"
                />
              </div>
            </div>

            <div v-if="libraryLoadingMore" class="library-empty library-load-more"><div class="identify-mini-spinner" /></div>
          </section>
        </template>

        <!-- Search view -->
        <template v-else-if="activeNav === 'search'">
          <section>
            <div class="library-header page-toolbar">
              <input
                v-model="searchQuery"
                class="library-search"
                @input="handleSearchInput"
                type="text"
                placeholder="Search tracks, artists, albums, genres..."
              />
            </div>

            <div v-if="libraryLoading" class="library-empty">Loading...</div>

            <template v-else>
              <template v-if="!searchQuery.trim()">
                <p class="search-empty-copy">Search across your local library and, when enabled, Soulseek.</p>

                <div v-if="searchRecentTracks.length" class="section-head search-section-head">
                  <h2>Start From Recent</h2>
                </div>

                <div v-if="searchRecentTracks.length" class="track-list">
                  <TrackRow
                    v-for="(track, idx) in searchRecentTracks"
                    :key="track.id"
                    :track="track"
                    show-year
                    show-genre
                    @play="playPlaylistTrack(searchRecentTracks, idx)"
                  />
                </div>
              </template>

              <template v-else>
                <div class="search-summary">
                  <span><strong>{{ localSearchTotal }}</strong> local tracks</span>
                  <span><strong>{{ searchArtistCount }}</strong> artists on this page</span>
                  <span><strong>{{ searchAlbumCount }}</strong> albums on this page</span>
                  <span v-if="soulseekReady"><strong>{{ searchSoulseekCountLabel }}</strong> Soulseek tracks</span>
                </div>

                <div class="section-head search-section-head">
                  <h2>Local Library</h2>
                </div>

                <div v-if="localSearchLoading" class="library-empty search-inline-empty">Searching local library…</div>

                <div v-else-if="searchResults.length === 0" class="library-empty search-inline-empty">
                  Nothing in your library for "{{ searchQuery }}".
                </div>

                <div v-else class="track-list">
                  <TrackRow
                    v-for="(track, idx) in searchResults"
                    :key="track.id"
                    :track="track"
                    show-year
                    show-genre
                    @play="playPlaylistTrack(searchResults, idx)"
                  />
                </div>
                <div v-if="searchResults.length" class="library-page-controls library-page-controls-bottom">
                  <span>{{ localSearchPageLabel }}</span>
                  <div>
                    <button class="icon-btn text-action-btn" :disabled="localSearchOffset === 0" @click="previousLocalSearchPage"><AppIcon name="page-previous" /> Previous</button>
                    <button class="icon-btn text-action-btn" :disabled="!localSearchHasMore" @click="nextLocalSearchPage">Next <AppIcon name="page-next" /></button>
                  </div>
                </div>
              </template>

              <div class="section-head search-section-head soulseek-section-head">
                <h2>Soulseek</h2>
                <div class="soulseek-section-actions">
                  <button
                    v-if="soulseekReady"
                    class="icon-btn text-action-btn soulseek-search-btn"
                    :disabled="!canRunSoulseekSearch"
                    @click="runExplicitSoulseekSearch"
                  >
                    {{ soulseekLoading ? 'Searching…' : 'Search' }}
                  </button>
                  <button class="text-action-btn soulseek-settings-link" @click="openSoulseekSettings">
                    {{ soulseekStatus?.enabled ? 'Settings' : 'Enable' }}
                  </button>
                </div>
              </div>

              <div v-if="!soulseekStatus?.enabled" class="library-empty search-inline-empty">
                Soulseek search is off. Enable it in Soulseek settings.
              </div>
              <div v-else-if="!soulseekStatus?.configured" class="library-empty search-inline-empty">
                Add Soulseek username and password in Soulseek settings to search the network.
              </div>
              <div v-else-if="!searchQuery.trim()" class="library-empty search-inline-empty">
                Enter a query above, then press Search in this block.
              </div>
              <div v-else-if="soulseekQueryDirty" class="library-empty search-inline-empty">
                Press Search in this block to query Soulseek for "{{ searchQuery.trim() }}".
              </div>
              <div v-else-if="soulseekLoading" class="library-empty search-inline-empty">
                Searching Soulseek…
              </div>
              <div v-else-if="soulseekError" class="library-empty search-inline-empty soulseek-inline-error">
                {{ soulseekError }}
              </div>
              <div v-else-if="soulseekResults.length === 0" class="library-empty search-inline-empty">
                No Soulseek matches for "{{ soulseekSubmittedQuery }}".
              </div>
              <div v-else class="track-list soulseek-list">
                <div
                  v-for="result in visibleSoulseekResults"
                  :key="`${result.username}\u0000${result.filename}`"
                  class="track-row soulseek-row"
                  @click="activateSoulseekResult(result)"
                >
                  <CoverImage class="track-cover-sm soulseek-cover-sm" :src="soulseekCoverUrl(result)">
                    <span v-if="!soulseekCoverUrl(result)">{{ result.coverFilename ? 'ART' : 'SL' }}</span>
                  </CoverImage>
                  <div class="track-info">
                    <div class="track-title-row">
                      <span class="track-title">{{ result.basename }}</span>
                      <span v-if="soulseekPreviewState(result)" class="soulseek-status-pill" :class="`state-${soulseekPreviewState(result)?.state}`">
                        {{ soulseekPreviewActionLabel(result) }}
                      </span>
                      <span v-if="soulseekDownloadState(result)" class="soulseek-status-pill" :class="`state-${soulseekDownloadState(result)?.state}`">
                        {{ soulseekDownloadActionLabel(result) }}
                      </span>
                    </div>
                    <span class="track-album">
                      {{ result.username }} · {{ formatBytes(result.size) }}
                      <template v-if="result.duration"> · {{ formatDuration(result.duration) }}</template>
                      <template v-if="result.bitrate"> · {{ result.bitrate }} kbps</template>
                      <template v-if="result.sampleRate"> · {{ formatSampleRate(result.sampleRate) }}</template>
                      <template v-if="result.bitDepth"> · {{ result.bitDepth }}-bit</template>
                      <template v-if="result.coverFilename"> · cover art</template>
                      <template v-if="result.freeUploadSlots > 0"> · {{ result.freeUploadSlots }} slots</template>
                    </span>
                    <span v-if="soulseekPreviewState(result)?.state === 'progress'" class="soulseek-progress-copy">
                      Preview buffer {{ formatBytes(soulseekPreviewState(result)?.bytesDownloaded) }} / {{ formatBytes(soulseekPreviewThresholdBytes(result)) }}
                      · {{ formatTransferRate(soulseekPreviewState(result)?.speedBytesPerSec) }}
                    </span>
                    <span v-else-if="soulseekPreviewState(result)?.state === 'queued_remote' && soulseekPreviewState(result)?.queuePosition != null" class="soulseek-progress-copy">
                      Preview queue position {{ soulseekPreviewState(result)?.queuePosition }}
                    </span>
                    <span v-else-if="soulseekPreviewState(result)?.state === 'completed'" class="soulseek-progress-copy">
                      Preview ready
                    </span>
                    <span v-else-if="soulseekTransferError(soulseekPreviewState(result))" class="soulseek-progress-copy soulseek-inline-error">
                      {{ soulseekTransferError(soulseekPreviewState(result)) }}
                    </span>
                    <span v-else-if="soulseekDownloadState(result)?.state === 'progress'" class="soulseek-progress-copy">
                      {{ formatBytes(soulseekDownloadState(result)?.bytesDownloaded) }} / {{ formatBytes(soulseekDownloadState(result)?.totalBytes) }}
                      · {{ formatTransferRate(soulseekDownloadState(result)?.speedBytesPerSec) }}
                    </span>
                    <span v-else-if="soulseekDownloadState(result)?.state === 'queued_remote' && soulseekDownloadState(result)?.queuePosition != null" class="soulseek-progress-copy">
                      Queue position {{ soulseekDownloadState(result)?.queuePosition }}
                    </span>
                    <span v-else-if="soulseekDownloadState(result)?.state === 'completed'" class="soulseek-progress-copy">
                      Saved to local library
                    </span>
                    <span v-else-if="soulseekTransferError(soulseekDownloadState(result))" class="soulseek-progress-copy soulseek-inline-error">
                      {{ soulseekTransferError(soulseekDownloadState(result)) }}
                    </span>
                  </div>
                  <div class="soulseek-side">
                    <span class="soulseek-speed">{{ formatTransferRate(result.peerSpeed) }}</span>
                    <button class="btn-secondary soulseek-download-btn" :disabled="soulseekDownloadBusy(result)" @click.stop="downloadSoulseekResult(result)">
                      {{ soulseekDownloadActionLabel(result) }}
                    </button>
                  </div>
                </div>
                <div v-if="visibleSoulseekResults.length < soulseekResults.length" class="library-empty search-inline-empty soulseek-more-copy">
                  Scroll down to load more results · showing {{ visibleSoulseekResults.length }} of {{ soulseekResults.length }}
                </div>
              </div>
            </template>
          </section>
        </template>

        <template v-else-if="activeNav === 'track-replace'">
          <section v-if="trackReplaceDialog" class="soulseek-replace-page">
            <div class="library-header soulseek-replace-page-header">
              <div class="soulseek-replace-page-heading">
                <button class="icon-btn soulseek-replace-back-btn" title="Back" @click="closeTrackReplaceDialog">
                  <AppIcon name="back" :size="20" />
                </button>
                <div>
                  <p class="soulseek-replace-track-copy">
                    Filled metadata stays intact; empty fields are filled from the replacement file.
                  </p>
                </div>
              </div>
            </div>

            <label class="field">
              <span>Soulseek query</span>
              <div class="soulseek-replace-search-row">
                <input
                  v-model="trackReplaceDialog.query"
                  placeholder="Artist title"
                  @input="handleTrackReplaceInput"
                  @keydown.enter.prevent="runTrackReplaceSearch"
                />
                <button class="btn-primary soulseek-replace-compact-btn" :disabled="!canRunTrackReplaceSearch" @click="runTrackReplaceSearch">
                  {{ trackReplaceLoading ? 'Searching…' : 'Search' }}
                </button>
              </div>
            </label>

            <div v-if="trackReplaceActionError" class="settings-error">{{ trackReplaceActionError }}</div>

            <div v-if="!soulseekStatus?.enabled" class="library-empty search-inline-empty">
              Soulseek search is off. Enable it in Soulseek settings.
            </div>
            <div v-else-if="!soulseekStatus?.configured" class="library-empty search-inline-empty">
              Add Soulseek username and password in Soulseek settings to search the network.
            </div>
            <div v-else-if="!trackReplaceDialog.query.trim()" class="library-empty search-inline-empty">
              Enter a query, then search Soulseek for a replacement.
            </div>
            <div v-else-if="trackReplaceQueryDirty" class="library-empty search-inline-empty">
              Press Search to query Soulseek for "{{ trackReplaceDialog.query.trim() }}".
            </div>
            <div v-else-if="trackReplaceLoading" class="library-empty search-inline-empty">
              Searching Soulseek…
            </div>
            <div v-else-if="trackReplaceError" class="library-empty search-inline-empty soulseek-inline-error">
              {{ trackReplaceError }}
            </div>
            <div v-else-if="trackReplaceResults.length === 0" class="library-empty search-inline-empty">
              No Soulseek matches for "{{ trackReplaceSubmittedQuery }}".
            </div>
            <div v-else class="track-list soulseek-list soulseek-replace-results">
              <div
                v-for="result in visibleTrackReplaceResults"
                :key="`${result.username}\u0000${result.filename}`"
                class="track-row soulseek-row"
                @click="activateSoulseekResult(result)"
              >
                <CoverImage class="track-cover-sm soulseek-cover-sm" :src="soulseekCoverUrl(result)">
                  <span v-if="!soulseekCoverUrl(result)">{{ result.coverFilename ? 'ART' : 'SL' }}</span>
                </CoverImage>
                <div class="track-info">
                  <div class="track-title-row">
                    <span class="track-title">{{ result.basename }}</span>
                    <span v-if="soulseekPreviewState(result)" class="soulseek-status-pill" :class="`state-${soulseekPreviewState(result)?.state}`">
                      {{ soulseekPreviewActionLabel(result) }}
                    </span>
                    <span v-if="soulseekDownloadState(result)" class="soulseek-status-pill" :class="`state-${soulseekDownloadState(result)?.state}`">
                      {{ soulseekDownloadActionLabel(result) }}
                    </span>
                  </div>
                  <span class="track-album">
                    {{ result.username }} · {{ formatBytes(result.size) }}
                    <template v-if="result.duration"> · {{ formatDuration(result.duration) }}</template>
                    <template v-if="result.bitrate"> · {{ result.bitrate }} kbps</template>
                    <template v-if="result.sampleRate"> · {{ formatSampleRate(result.sampleRate) }}</template>
                    <template v-if="result.bitDepth"> · {{ result.bitDepth }}-bit</template>
                    <template v-if="result.coverFilename"> · cover art</template>
                    <template v-if="result.freeUploadSlots > 0"> · {{ result.freeUploadSlots }} slots</template>
                  </span>
                  <span v-if="soulseekPreviewState(result)?.state === 'progress'" class="soulseek-progress-copy">
                    Preview buffer {{ formatBytes(soulseekPreviewState(result)?.bytesDownloaded) }} / {{ formatBytes(soulseekPreviewThresholdBytes(result)) }}
                    · {{ formatTransferRate(soulseekPreviewState(result)?.speedBytesPerSec) }}
                  </span>
                  <span v-else-if="soulseekPreviewState(result)?.state === 'queued_remote' && soulseekPreviewState(result)?.queuePosition != null" class="soulseek-progress-copy">
                    Preview queue position {{ soulseekPreviewState(result)?.queuePosition }}
                  </span>
                  <span v-else-if="soulseekPreviewState(result)?.state === 'completed'" class="soulseek-progress-copy">
                    Preview ready
                  </span>
                  <span v-else-if="soulseekTransferError(soulseekPreviewState(result))" class="soulseek-progress-copy soulseek-inline-error">
                    {{ soulseekTransferError(soulseekPreviewState(result)) }}
                  </span>
                  <span v-else-if="soulseekDownloadState(result)?.state === 'progress'" class="soulseek-progress-copy">
                    {{ formatBytes(soulseekDownloadState(result)?.bytesDownloaded) }} / {{ formatBytes(soulseekDownloadState(result)?.totalBytes) }}
                    · {{ formatTransferRate(soulseekDownloadState(result)?.speedBytesPerSec) }}
                  </span>
                  <span v-else-if="soulseekDownloadState(result)?.state === 'queued_remote' && soulseekDownloadState(result)?.queuePosition != null" class="soulseek-progress-copy">
                    Queue position {{ soulseekDownloadState(result)?.queuePosition }}
                  </span>
                  <span v-else-if="soulseekDownloadState(result)?.state === 'completed'" class="soulseek-progress-copy">
                    Saved locally and ready to replace
                  </span>
                  <span v-else-if="soulseekTransferError(soulseekDownloadState(result))" class="soulseek-progress-copy soulseek-inline-error">
                    {{ soulseekTransferError(soulseekDownloadState(result)) }}
                  </span>
                </div>
                <div class="soulseek-side soulseek-replace-actions">
                  <span class="soulseek-speed">{{ formatTransferRate(result.peerSpeed) }}</span>
                  <button
                    class="icon-btn soulseek-download-icon-btn"
                    :class="{ green: soulseekDownloadState(result)?.state === 'completed' }"
                    :disabled="soulseekDownloadBusy(result)"
                    :title="soulseekDownloadActionLabel(result)"
                    :aria-label="soulseekDownloadActionLabel(result)"
                    @click.stop="downloadSoulseekResult(result)"
                  >
                    <AppIcon name="done" v-if="soulseekDownloadState(result)?.state === 'completed'" :size="16" />
                    <AppIcon name="download" v-else :size="16" />
                  </button>
                  <button class="btn-primary soulseek-replace-btn" :disabled="!!trackReplaceApplyingKey" @click.stop="replaceTrackWithSoulseekResult(trackReplaceDialog.track, result)">
                    {{ trackReplaceActionLabel(result) }}
                  </button>
                </div>
              </div>
              <div v-if="visibleTrackReplaceResults.length < trackReplaceResults.length" class="library-empty search-inline-empty soulseek-more-copy">
                <button class="btn-secondary soulseek-replace-compact-btn" @click="loadMoreTrackReplaceResults">Load more results</button>
              </div>
            </div>
          </section>
          <section v-else>
            <div class="library-header soulseek-replace-page-header">
              <div class="soulseek-replace-page-heading">
                <button class="icon-btn soulseek-replace-back-btn" title="Back" @click="closeTrackReplaceDialog">
                  <AppIcon name="back" :size="20" />
                </button>
              </div>
            </div>
            <div class="library-empty">No track selected.</div>
          </section>
        </template>

        <!-- History view -->
        <template v-else-if="activeNav === 'history'">
          <section>
            <div class="library-header">
              <div style="display:flex; align-items:center; gap:12px;">
                <button class="icon-btn text-action-btn" style="padding:4px;" @click="activeNav = 'home'">
                  <AppIcon name="back" :size="20" /> Home
                </button>
              </div>
            </div>
            <div v-if="historyLoading" class="library-empty">Loading...</div>
            <div v-else-if="historyEntries.length === 0" class="library-empty">No history yet.</div>
            <div v-else class="history-list">
              <TrackRow
                v-for="(entry, idx) in historyEntries"
                :key="idx"
                :track="entry.track"
                @play="playLibraryTrack(libraryFlatList.findIndex(t => t.id === entry.track.id))"
              >
                <template #extra>
                  <span class="history-time">{{ formatHistoryDate(entry.played_at) }}</span>
                </template>
              </TrackRow>
            </div>
          </section>
        </template>

        <!-- Playlists view -->
        <template v-else-if="activeNav === 'playlists'">
          <section>
            <!-- ── Regular playlists ─────────────────────────────────────── -->
            <template v-if="playlistTab === 'regular'">
              <!-- Playlist detail -->
              <template v-if="playlistView">
                <div v-if="playlistView.tracks.length" class="library-header" style="justify-content:flex-end;">
                  <button v-if="playlistView.tracks.length" class="btn-primary playlist-action" @click="playRegularPlaylistTrack(0)"><AppIcon name="play" /> Play</button>
                </div>
                <div v-if="playlistView.tracks.length === 0" class="library-empty">No tracks yet. Right-click any track to add.</div>
                <div v-else class="track-list">
                  <TrackRow
                    v-for="(track, idx) in playlistView.tracks"
                    :key="track.id"
                    :track="track"
                    :playlist-id="playlistView.id"
                    @play="playRegularPlaylistTrack(idx)"
                  >
                    <template #extra>
                      <button class="icon-btn track-inline-action" title="Remove from playlist" @click.stop="removeTrackFromPlaylist(playlistView!.id, track.id)">
                        <AppIcon name="remove" />
                      </button>
                    </template>
                  </TrackRow>
                </div>
                <div v-if="playlistView.loadingMore" class="library-empty library-load-more"><div class="identify-mini-spinner" /></div>
              </template>
              <!-- Playlist list -->
              <template v-else>
                <div class="library-header page-toolbar">
                  <button class="btn-secondary playlist-action" :aria-expanded="showNewPlaylistInput" @click="showNewPlaylistInput = !showNewPlaylistInput"><AppIcon name="add-to-playlist" /> New</button>
                </div>
                <div v-if="showNewPlaylistInput" style="display:flex; gap:8px; padding: 0 0 14px;">
                  <input
                    v-model="newPlaylistName"
                    class="library-search"
                    placeholder="Playlist name..."
                    style="flex:1;"
                    @keydown.enter="createPlaylist"
                    @keydown.esc="showNewPlaylistInput = false"
                  />
                  <button class="btn-primary playlist-action" @click="createPlaylist">Create</button>
                </div>
                <div v-if="playlists.length === 0" class="library-empty">No playlists yet.</div>
                <div v-else class="track-list">
                  <div v-for="pl in playlists" :key="pl.id" class="track-row" style="cursor:pointer;" @click="openPlaylist(pl)">
                    <PlaylistArtwork class="track-cover-sm" kind="regular" />
                    <div class="track-info">
                      <span class="track-title">{{ pl.name }}</span>
                      <span class="track-album">{{ pl.track_count }} track{{ pl.track_count !== 1 ? 's' : '' }}</span>
                    </div>
                    <button class="icon-btn" :class="{ green: pl.pinned }" style="margin-right:4px;" :title="pl.pinned ? 'Unpin from Home' : 'Pin to Home'" @click.stop="togglePlaylistPinned(pl)">
                      <AppIcon name="pin" :size="15" />
                    </button>
                    <button class="icon-btn" style="margin-right:8px;" title="Delete playlist" @click.stop="deletePlaylist(pl.id)">
                      <AppIcon name="delete" :size="16" />
                    </button>
                  </div>
                </div>
              </template>
            </template>

            <!-- ── Smart playlists ─────────────────────────────────────────── -->
            <template v-else>
              <!-- Editing a smart playlist -->
              <template v-if="editingSP">
                <div class="library-header" style="justify-content:flex-end;">
                  <span class="smart-track-count">{{ editingSPTotal }} tracks</span>
                </div>

                <!-- Match mode -->
                <div class="sp-match-row">
                  Match
                  <button :class="['sp-match-btn', editingSP.match === 'all' && 'active']" @click="editingSP.match = 'all'; saveSP()">ALL</button>
                  <button :class="['sp-match-btn', editingSP.match === 'any' && 'active']" @click="editingSP.match = 'any'; saveSP()">ANY</button>
                  of the following rules:
                </div>

                <!-- Rules -->
                <div class="sp-rules">
                  <div v-for="rule in editingSP.rules" :key="rule.id" class="sp-rule-row">
                    <AppSelect :model-value="rule.field" @update:model-value="rule.field = $event; onSPRuleFieldChange(rule)">
                      <option value="any">Any field</option>
                      <option value="title">Title</option>
                      <option value="artist">Artist</option>
                      <option value="album">Album</option>
                      <option value="album_artist_or_artist">Album artist</option>
                      <option value="genre">Genre</option>
                      <option value="tags">Tags</option>
                      <option value="rarity">Rarity</option>
                      <option value="path">Path</option>
                      <option value="extension">Extension</option>
                      <option value="disc_number">Disc #</option>
                      <option value="track_number">Track #</option>
                      <option value="duration_secs">Duration</option>
                      <option value="year">Year</option>
                      <option value="play_count">Play count</option>
                      <option value="is_liked">Is liked</option>
                      <option value="date_added">Date added</option>
                      <option value="sort">Sort by</option>
                    </AppSelect>

                    <template v-if="spFieldType(rule.field) === 'text'">
                      <span class="sp-op-label">contains</span>
                      <input class="library-search sp-text-input" v-model="rule.value" placeholder="search…" @input="saveSP()" />
                    </template>
                    <template v-else-if="spFieldType(rule.field) === 'number'">
                      <AppSelect :model-value="rule.op" @update:model-value="rule.op = $event; saveSP()">
                        <option value="gte">≥</option>
                        <option value="lte">≤</option>
                        <option value="eq">=</option>
                      </AppSelect>
                      <input class="library-search sp-num-input" type="number" min="0" v-model="rule.value" @input="saveSP()" />
                    </template>
                    <template v-else-if="spFieldType(rule.field) === 'bool'">
                      <button :class="['sp-match-btn', rule.op === 'is_true' && 'active']" @click="rule.op = 'is_true'; saveSP()">Yes</button>
                      <button :class="['sp-match-btn', rule.op === 'is_false' && 'active']" @click="rule.op = 'is_false'; saveSP()">No</button>
                    </template>
                    <template v-else-if="spFieldType(rule.field) === 'date'">
                      <AppSelect :model-value="rule.op" @update:model-value="rule.op = $event; saveSP()">
                        <option value="gte">after</option>
                        <option value="lte">before</option>
                        <option value="eq">on</option>
                      </AppSelect>
                      <input class="library-search sp-text-input" type="date" v-model="rule.value" @change="saveSP()" />
                    </template>
                    <template v-else-if="spFieldType(rule.field) === 'sort'">
                      <span class="sp-op-label">by</span>
                      <AppSelect :model-value="normalizeSPSortField(rule.value)" @update:model-value="rule.value = normalizeSPSortField($event); saveSP()">
                        <option v-for="option in SP_SORT_FIELD_OPTIONS" :key="option.value" :value="option.value">{{ option.label }}</option>
                      </AppSelect>
                      <AppSelect :model-value="normalizeSPSortOp(rule.op)" @update:model-value="rule.op = normalizeSPSortOp($event); saveSP()">
                        <option value="sort_asc">ASC</option>
                        <option value="sort_desc">DESC</option>
                      </AppSelect>
                    </template>
                    <template v-else>
                      <div class="sp-multiselect">
                        <label v-for="v in spUniqueValues(rule.field)" :key="v" class="sp-chip" :class="{ selected: spIsSelected(rule, v) }" @click="spToggleValue(rule, v)">{{ v }}</label>
                        <span v-if="spUniqueValues(rule.field).length === 0" class="smart-empty-values">No values in library</span>
                      </div>
                    </template>

                    <button class="icon-btn" style="margin-left:auto; flex-shrink:0;" @click="removeSPRule(rule.id)" title="Remove rule">
                      <AppIcon name="remove" :size="14" />
                    </button>
                  </div>
                  <button class="sp-add-rule-btn" @click="addSPRule()">+ Add rule</button>
                </div>

                <!-- Preview -->
                <div class="sp-preview-header">Preview ({{ editingSPTotal }} tracks)</div>
                <div v-if="editingSP.rules.length === 0" class="library-empty">Add rules above to filter your library.</div>
                <div v-else-if="editingSPLoading" class="library-empty">Updating preview…</div>
                <div v-else-if="editingSPTotal === 0" class="library-empty">No tracks match the current rules.</div>
                <div v-else class="track-list sp-preview">
                  <TrackRow
                    v-for="track in editingSPTracks"
                    :key="track.id"
                    :track="track"
                    show-year
                    show-genre
                    @play="playLibraryTrack(libraryFlatList.findIndex(t => t.id === track.id))"
                  />
                  <div v-if="editingSPTotal > editingSPTracks.length" class="library-empty" style="padding:12px 0;">… and {{ editingSPTotal - editingSPTracks.length }} more</div>
                </div>
              </template>

              <!-- Viewing a smart playlist (read-only) -->
              <template v-else-if="smartView">
                <div class="library-header" style="justify-content:flex-end;">
                  <div style="display:flex;gap:8px;">
                    <button class="btn-secondary playlist-action" @click="editingSP = { ...smartView! }; smartView = null"><AppIcon name="edit" /> Edit</button>
                    <button v-if="smartViewTracks.length" class="btn-primary playlist-action" @click="playSmartPlaylistTrack(0)"><AppIcon name="play" /> Play</button>
                  </div>
                </div>
                <div v-if="smartViewLoading" class="library-empty">Loading playlist…</div>
                <div v-else-if="smartViewTotal === 0" class="library-empty">No tracks match this smart playlist.</div>
                <div v-else class="track-list">
                  <TrackRow
                    v-for="(track, idx) in smartViewTracks"
                    :key="track.id"
                    :track="track"
                    show-year
                    show-genre
                    @play="playSmartPlaylistTrack(idx)"
                  />
                  <div v-if="smartViewLoadingMore" class="library-load-more"><span class="identify-mini-spinner" aria-label="Loading more tracks" /></div>
                </div>
              </template>

              <!-- Smart playlist list -->
              <template v-else>
                <div class="library-header page-toolbar">
                  <button class="btn-secondary playlist-action" :aria-expanded="showNewSPInput" @click="showNewSPInput = !showNewSPInput"><AppIcon name="add-to-playlist" /> New</button>
                </div>
                <div v-if="showNewSPInput" style="display:flex; gap:8px; padding: 0 0 14px;">
                  <input v-model="newSPName" class="library-search" placeholder="Smart playlist name…" style="flex:1;" @keydown.enter="createSmartPlaylist" @keydown.esc="showNewSPInput = false" />
                  <button class="btn-primary playlist-action" @click="createSmartPlaylist">Create</button>
                </div>
                <div v-if="smartPlaylists.length === 0" class="library-empty">No flexible playlists yet.</div>
                <div v-else class="track-list">
                  <div v-for="sp in smartPlaylists" :key="sp.id" class="track-row" style="cursor:pointer;" @click="smartView = sp">
                    <PlaylistArtwork class="track-cover-sm" kind="smart" />
                    <div class="track-info">
                      <span class="track-title">{{ sp.name }}</span>
                      <span class="track-album">{{ sp.rules.length }} rule{{ sp.rules.length !== 1 ? 's' : '' }} · {{ smartPlaylistPreviews[sp.id]?.total ?? 0 }} tracks</span>
                    </div>
                    <button class="icon-btn" :class="{ green: sp.pinned }" style="margin-right:4px;" :title="sp.pinned ? 'Unpin from Home' : 'Pin to Home'" @click.stop="toggleSmartPlaylistPinned(sp)">
                      <AppIcon name="pin" :size="15" />
                    </button>
                    <button class="icon-btn" style="margin-right:4px;" title="Edit" @click.stop="editingSP = { ...sp }; smartView = null">
                      <AppIcon name="edit" :size="14" />
                    </button>
                    <button class="icon-btn" style="margin-right:8px;" title="Delete" @click.stop="deleteSmartPlaylist(sp.id)">
                      <AppIcon name="delete" :size="14" />
                    </button>
                  </div>
                </div>
              </template>
            </template>
          </section>
        </template>

        <!-- Duplicates view -->
        <template v-else-if="activeNav === 'dedup'">
          <section>
            <div class="library-header page-toolbar">
              <div class="dedup-toolbar">
                <button
                  class="dedup-filter-btn"
                  :class="{ active: dedupFilter === 'all' }"
                  @click="dedupFilter = 'all'"
                >All groups</button>
                <button
                  class="dedup-filter-btn"
                  :class="{ active: dedupFilter === 'unresolved' }"
                  @click="dedupFilter = 'unresolved'"
                >Unresolved</button>
                <button class="btn-secondary dedup-rescan-btn" @click="openDedup()" :disabled="dedupLoading">
                  <AppIcon name="refresh" :size="14" />
                  Rescan
                </button>
              </div>
            </div>

            <div v-if="dedupLoading" class="library-empty">Scanning library…</div>

            <div v-else-if="dedupError" class="settings-error dedup-error">{{ dedupError }}</div>

            <template v-else-if="dedupGroups.length === 0">
              <p class="search-empty-copy dedup-clean-copy">
                <AppIcon name="success" :size="20" class="success-icon" />
                No duplicates found. Your library looks clean!
              </p>
            </template>

            <template v-else>
              <div class="dedup-summary">
                Found <strong>{{ dedupGroups.length }}</strong> potential duplicate group{{ dedupGroups.length === 1 ? '' : 's' }}.
                Click a track to toggle it between Keep and Duplicate, then click Apply.
              </div>

              <div v-for="{ g, i } in dedupFilteredGroups" :key="i" class="dedup-group">
                <div class="dedup-group-header">
                  <div class="dedup-group-title">
                    Group {{ i + 1 }}
                    <span class="dedup-track-count">{{ g.tracks.length }} tracks</span>
                  </div>
                  <div class="dedup-reasons">
                    <span v-for="r in g.reasons" :key="r" class="dedup-reason-tag">{{ r }}</span>
                  </div>
                </div>

                <div class="dedup-group-tracks">
                  <div
                    v-for="track in g.tracks"
                    :key="track.id"
                    class="track-row dedup-track-row"
                    :class="[
                      rarityClass(track.rarity),
                      { 'dedup-keep': !dedupIsMarkedDuplicate(i, track.id), 'dedup-remove': dedupIsMarkedDuplicate(i, track.id), 'dedup-already-marked': track.is_duplicate }
                    ]"
                    :style="rarityVars(track.rarity)"
                    @click="dedupToggleTrack(i, track.id)"
                  >
                    <CoverImage class="track-cover-sm" :src="coverUrl(track.cover_hash, 'small')" :placeholder="placeholderCoverColors(track.file_hash)" />
                    <div class="track-info">
                      <div class="track-title-row">
                        <span class="track-title">{{ track.title || track.path }}</span>
                        <span v-if="!dedupIsMarkedDuplicate(i, track.id)" class="dedup-keep-badge">Keep</span>
                        <span v-else class="dedup-remove-badge">Duplicate</span>
                      </div>
                      <span class="track-album">
                        {{ artistLabel(track.artist) }}{{ track.album ? ' · ' + track.album : '' }}
                      </span>
                      <span class="dedup-track-meta">
                        <span>{{ track.path }}</span>
                        <span v-if="track.play_count > 0"><AppIcon name="play-count" :size="12" /> {{ track.play_count }}</span>
                        <span v-if="track.manually_edited" class="dedup-edited-tag">edited</span>
                        <span v-if="track.is_duplicate" class="dedup-flagged-tag">flagged</span>
                      </span>
                    </div>
                    <div class="dedup-keep-radio">
                      <div class="dedup-radio-dot" :class="{ active: !dedupIsMarkedDuplicate(i, track.id) }"></div>
                    </div>
                  </div>
                </div>
              </div>

              <div class="dedup-actions">
                <button
                  v-if="dedupHasPersistedFlags"
                  class="btn-secondary"
                  style="margin-right: auto;"
                  @click="invoke('unmark_duplicates', { ids: dedupGroups.flatMap(g => g.tracks.filter(t => t.is_duplicate).map(t => t.id)) }).then(() => openDedup()).then(() => loadLibrary())"
                >
                  Unmark all
                </button>
                <button class="btn-primary" @click="dedupConfirmOpen = true" :disabled="dedupApplying">
                  Mark as duplicates ({{ dedupMarkedTotal }})
                </button>
              </div>
            </template>
          </section>
        </template>

        <!-- Discovery view -->
        <template v-else-if="activeNav === 'discovery'">
          <section>
            <div class="library-header discovery-header page-toolbar">
              <div class="discovery-toolbar">
                <button class="sync-toggle" @click="openDeviceSettings"><span class="device-emoji">{{ deviceEmoji }}</span> Settings</button>
                <button
                  class="sync-toggle"
                  :class="{ active: syncEnabled }"
                  @click="toggleSync"
                  :title="syncEnabled ? 'Sync on — click to disable' : 'Sync off — click to enable'"
                >
                  <AppIcon name="sync" :size="14" />
                  Sync
                </button>
              </div>
            </div>
            <div class="search-summary discovery-summary">
              <span><strong>{{ peers.length }}</strong> devices</span>
              <span><strong>{{ syncEnabled ? 'On' : 'Off' }}</strong> sync</span>
              <span><strong>mDNS</strong> auto discovery</span>
            </div>
            <div v-if="!peers.length" class="library-empty discovery-empty">
              <span>No other instances found.</span>
              <span class="discovery-help-text">Make sure devices are on the same Wi-Fi network.</span>
            </div>
            <div v-else class="track-list discovery-list">
              <div v-for="peer in peers" :key="peer.host" class="track-row discovery-row">
                <div class="track-cover-sm peer-icon">{{ peer.device_emoji || syncProgress[peer.name]?.device_emoji || '🎵' }}</div>
                <div class="track-info discovery-info">
                  <div class="track-title-row">
                    <span class="track-title">{{ peer.device_name || peerDeviceNames[peer.name] || peer.name }}</span>
                    <span v-if="isRemoteOutputPeer(peer)" class="track-playback-badge next discovery-output-pill">Output</span>
                    <span class="discovery-status-pill" :class="peerPlaybackClass(peer)">
                      <span class="peer-playback-icon" aria-hidden="true">
                        <AppIcon name="play" v-if="peer.playback?.state === 'playing'" :size="14" />
                        <AppIcon name="pause" v-else-if="peer.playback?.state === 'paused'" :size="14" />
                        <AppIcon name="stop" v-else :size="14" />
                      </span>
                      {{ peerPlaybackLabel(peer) }}
                    </span>
                  </div>
                  <span class="track-album discovery-meta">
                    {{ peerNowPlayingText(peer) || 'No active playback' }} · {{ peer.host }}:{{ peer.port }}
                  </span>
                  <template v-if="syncProgress[peer.name]">
                    <div class="discovery-sync-block">
                      <div class="sync-status-row">
                        <span class="sync-status" :class="'sync-' + syncProgress[peer.name].phase">
                          {{ syncStatusText(syncProgress[peer.name]) }}
                        </span>
                        <span v-if="syncCountText(syncProgress[peer.name])" class="sync-count">
                          {{ syncCountText(syncProgress[peer.name]) }}
                        </span>
                      </div>
                      <div class="sync-bar-wrap" v-if="['download','reindex','merging'].includes(syncProgress[peer.name].phase)">
                        <div class="sync-bar" :style="{ width: syncItemPercent(syncProgress[peer.name]) + '%' }" />
                        <div class="sync-bar-overall" :style="{ width: syncOverallPercent(syncProgress[peer.name]) + '%' }" />
                      </div>
                    </div>
                  </template>
                </div>
                <button
                  v-if="syncEnabled"
                  class="sync-now-btn discovery-sync-btn"
                  @click="syncPeer(peer)"
                  :disabled="['index','download','reindex','merging'].includes(syncProgress[peer.name]?.phase ?? '')"
                  title="Sync now"
                >
                  <AppIcon name="sync" :size="16" />
                </button>
              </div>
            </div>
          </section>
        </template>

        <template v-else-if="isSettingsNav || activeNav === 'about'">
          <SettingsView
            v-model:show-duplicate-tracks="showDuplicateTracks"
            :section="activeNav === 'about' ? 'about' : settingsSection"
            :data-dir="libraryDataDir"
            :index-busy="libraryScanBusy"
            :index-current="indexCurrent" :index-total="indexTotal" :index-done="indexDone" :index-added="indexAdded"
            :identify-busy="identifyPreparing || (identifyRunning && !identifyDone)" :action-error="settingsActionError" :action-message="settingsActionMessage"
            :output-devices="outputDevices" :current-device="currentDevice" :selected-device="selectedOutputDevice"
            :current-sample-rate="currentSampleRate" :native-sample-rate="nativeSampleRate"
            :audio-loading="audioLoading" :audio-busy="audioBusy" :audio-error="deviceMenuError"
            :remote-output-label="remoteOutputPeer ? peerLabel(remoteOutputPeer) : ''"
            :sync-enabled="syncEnabled" :sync-busy="syncBusy" :sync-error="syncError" :sync-active="syncActive" :peers-count="peers.length"
            :update-available="hasAvailableUpdate"
            :discord-enabled="discord.enabled.value" :discord-available="discord.available.value"
            :discord-configured="discord.configured.value" :discord-ready="discord.ready.value" :discord-error="discord.error.value"
            @toggle-discord="discord.setEnabled"
            :cover-url="currentCoverUrl" :cover-colors="currentTrack.colors"
            @navigate="openSettings" @open-folder="openSettingsFolder" @reindex="doReindex" @identify="identifyFromSettings"
            @duplicates="activeNav = 'dedup'; openDedup()" @devices="activeNav = 'discovery'"
            @refresh-audio="refreshOutputDevices" @select-output="selectSettingsOutput" @select-sample-rate="selectSettingsSampleRate"
            @toggle-sync="toggleSync" @device-saved="onDeviceSettingsSaved" @soulseek-saved="loadSoulseekStatus"
          >
            <template #appearance-track>
              <div v-for="(rarity, index) in ['Epic', 'Legendary', 'Mythic']" :key="rarity" class="track-row appearance-track-preview" :class="rarityClass(rarity)" :style="rarityVars(rarity)" role="listitem" :aria-label="`${rarity} track appearance preview`">
                <div class="track-cover-sm" :style="currentCoverUrl
                  ? { backgroundImage: `url(${currentCoverUrl})`, backgroundSize: 'cover', backgroundPosition: 'center' }
                  : { background: `linear-gradient(135deg, ${currentTrack.colors[0]}, ${currentTrack.colors[1]})` }" />
                <span class="track-num">{{ index + 1 }}</span>
                <div class="track-info">
                  <div class="track-title-row"><span class="track-title">Preview track {{ index + 1 }}</span></div>
                  <span class="track-album">{{ nowPlaying?.album || 'Your music library' }}</span>
                </div>
                <span class="track-dur">{{ formatDuration(213 + index * 24) }}</span>
              </div>
            </template>
            <template #about>
              <AboutPanel :info="aboutInfo" :loading="aboutLoading" :error="aboutError" :status="aboutUpdateStatus"
                :checking="aboutCheckingUpdates" :updating="aboutUpdating" :progress="aboutUpdateProgress"
                :desktop-ready="aboutDesktopUpdateReady" :update-error="aboutUpdateError" :update-done="aboutUpdateDone"
                @check="checkAboutUpdates" @update="doAboutUpdate" @open-release="openAboutDownloadLink"
                @open-repository="openUrl('https://github.com/' + (aboutInfo?.release_repo || 'verncat/player'))"
                @retry="loadAbout(true)" @open-external="openUrl" />
            </template>
          </SettingsView>
        </template>

        <template v-else-if="activeNav === 'artist'">
          <ArtistView
            :name="navTarget"
            :include-duplicates="showDuplicateTracks"
            :library-version="libraryVersion"
            @play="playPlaylistTrack"
          />
        </template>

        <template v-else-if="activeNav === 'album'">
          <AlbumView
            :name="navTarget"
            :album-artist-or-artist="navTargetArtist"
            :include-duplicates="showDuplicateTracks"
            :library-version="libraryVersion"
            @play="playPlaylistTrack"
          />
        </template>
      </div>
    </main>

    <!-- Context menus -->
    <ResponsivePopup
      :open="!!trackContextMenu"
      :anchor-x="trackContextMenu?.x ?? null"
      :anchor-y="trackContextMenu?.y ?? null"
      wrapper-class="track-context-menu-wrapper"
      panel-class="playlist-menu track-context-menu"
      @close="trackContextMenu = null"
    >
      <template v-if="trackContextMenu">
        <div class="playlist-menu-header">Track actions</div>
        <button class="playlist-menu-item" @click="toggleLikeFromTrackContext">{{ trackContextMenu.track.is_liked ? 'Unlike' : 'Like' }}</button>
        <button v-if="hasArtistPage(trackContextMenu.track.artist)" class="playlist-menu-item" @click="openArtist(trackContextMenu.track.artist); trackContextMenu = null">Go to artist</button>
        <button v-if="trackContextMenu.track.album" class="playlist-menu-item" @click="openAlbum(trackContextMenu.track.album, trackContextMenu.track.album_artist_or_artist ?? ''); trackContextMenu = null">Go to album</button>
        <button class="playlist-menu-item" @click="openTrackReplaceDialogFromTrackContext">Search and replace</button>
        <button class="playlist-menu-item" @click="shareTrackFromTrackContext">Share</button>
        <button class="playlist-menu-item" @click="addTrackToPlaylistFromTrackContext">Add to playlist</button>
        <button class="playlist-menu-item" @click="editTrackFromTrackContext">Edit metadata</button>
        <button class="playlist-menu-item" @click="identifyTrackFromTrackContext">Identify</button>
        <button
          v-if="trackContextMenu.playlistId !== null"
          class="playlist-menu-item danger"
          @click="removeTrackFromPlaylistFromTrackContext"
        >
          Remove from playlist
        </button>
        <button class="playlist-menu-item danger" @click="hideTrackAsDuplicateFromTrackContext">Hide (mark as duplicate)</button>
      </template>
    </ResponsivePopup>

    <ResponsivePopup
      :open="!!homePinnedContextMenu"
      :anchor-x="homePinnedContextMenu?.x ?? null"
      :anchor-y="homePinnedContextMenu?.y ?? null"
      wrapper-class="home-pinned-context-menu-wrapper"
      panel-class="playlist-menu"
      @close="homePinnedContextMenu = null"
    >
      <template v-if="homePinnedContextMenu">
        <div class="playlist-menu-header">{{ homePinnedContextMenu.item.kind === 'regular' ? 'Playlist actions' : 'Flexible playlist actions' }}</div>
        <button class="playlist-menu-item" @click="openHomePinnedFromContextMenu">Open</button>
        <button v-if="homePinnedContextMenu.item.trackCount > 0" class="playlist-menu-item" @click="playHomePinnedFromContextMenu">Play</button>
        <button v-if="homePinnedContextMenu.item.kind === 'smart'" class="playlist-menu-item" @click="editHomePinnedFromContextMenu">Edit rules</button>
        <button class="playlist-menu-item" @click="unpinHomePinnedFromContextMenu">Unpin from Home</button>
        <button class="playlist-menu-item danger" @click="deleteHomePinnedFromContextMenu">Delete</button>
      </template>
    </ResponsivePopup>

    <ResponsivePopup
      :open="!!addToPlaylistMenu"
      :anchor-x="addToPlaylistMenu?.x ?? null"
      :anchor-y="addToPlaylistMenu?.y ?? null"
      wrapper-class="add-to-playlist-menu-wrapper"
      panel-class="playlist-menu"
      @close="addToPlaylistMenu = null"
    >
      <template v-if="addToPlaylistMenu">
        <div class="playlist-menu-header">Add to playlist</div>
        <div v-if="playlists.length === 0" class="playlist-menu-empty">No playlists. Create one first.</div>
        <button
          v-for="pl in playlists"
          :key="pl.id"
          class="playlist-menu-item"
          @click="addTrackToPlaylist(pl.id, addToPlaylistMenu!.track.id); addToPlaylistMenu = null"
        >
          {{ pl.name }}
        </button>
      </template>
    </ResponsivePopup>

    <!-- Dedup confirm modal -->
    <ResponsivePopup
      :open="dedupConfirmOpen"
      wrapper-class="dedup-confirm-popup-wrapper"
      panel-class="settings-dropdown dedup-confirm-popup"
      centered
      show-actions
      cancel-label="Cancel"
      :save-label="dedupApplying ? 'Marking…' : 'Mark as duplicates'"
      :save-disabled="dedupApplying"
      @close="dedupConfirmOpen = false"
      @cancel="dedupConfirmOpen = false"
      @save="applyDedup()"
    >
      <div class="modal-header">
        <h3>Confirm marking</h3>
        <button class="icon-btn" @click="dedupConfirmOpen = false">
          <AppIcon name="close" :size="18" />
        </button>
      </div>
      <div class="modal-body">
        <p class="modal-copy">
          <strong>{{ dedupMarkedTotal }}</strong>
          track(s) will be flagged as duplicates in the database. Files stay on disk. You can unmark them at any time.
        </p>
        <div v-if="dedupError" class="settings-error">{{ dedupError }}</div>
      </div>
    </ResponsivePopup>

    <!-- Edit modal -->
    <Transition name="modal">
      <div v-if="editingTrack" class="modal-overlay" @click.self="editingTrack = null">
        <div class="modal edit-track-modal">
          <div class="modal-header">
            <h3>Edit Track</h3>
            <button class="icon-btn" @click="editingTrack = null">
              <AppIcon name="close" :size="18" />
            </button>
          </div>
          <div class="modal-body edit-track-body">
            <label class="field">
              <span>Title</span>
              <input v-model="editForm.title" />
            </label>
            <label class="field">
              <span>Artist</span>
              <input v-model="editForm.artist" />
            </label>
            <label class="field">
              <span>Album</span>
              <input v-model="editForm.album" />
            </label>
            <label class="field">
              <span>Album artist</span>
              <input v-model="editForm.album_artist" :placeholder="albumArtistPlaceholder" />
            </label>
            <label class="field">
              <span>Track #</span>
              <input v-model.number="editForm.track_number" type="number" min="0" />
            </label>
            <label class="field">
              <span>Disc #</span>
              <input v-model.number="editForm.disc_number" type="number" min="1" />
            </label>
            <label class="field">
              <span>Year</span>
              <input v-model.number="editForm.year" type="number" min="0" />
            </label>
            <label class="field">
              <span>Genre</span>
              <input v-model="editForm.genre" />
            </label>
            <label class="field">
              <span>User tags</span>
              <input v-model="editForm.tags" placeholder="tag one, tag two" />
            </label>
            <label class="field">
              <span>Play count</span>
              <input v-model.number="editForm.play_count" type="number" min="0" />
            </label>
            <label class="field">
              <span>Liked</span>
              <AppSelect v-model="editForm.is_liked" variant="filled">
                <option :value="false">No</option>
                <option :value="true">Yes</option>
              </AppSelect>
            </label>
            <label class="field">
              <span>Date added</span>
              <input v-model="editForm.date_added" type="date" />
            </label>
            <label class="field">
              <span>Rarity</span>
              <AppSelect v-model="editForm.rarity" variant="filled">
                <option value="">Unset</option>
                <option v-for="rarity in TRACK_RARITY_OPTIONS" :key="rarity" :value="rarity">{{ rarity }}</option>
              </AppSelect>
            </label>
          </div>
          <div class="modal-footer">
            <button class="btn-secondary" @click="editingTrack = null">Cancel</button>
            <button class="btn-primary" @click="saveTrack">Save</button>
          </div>
        </div>
      </div>
    </Transition>

    <!-- Index progress pill (clickable to open log) -->
    <div class="activity-dock" :class="{ 'activity-active': indexRunning || libraryPageLoading }">
      <button
        type="button"
        class="identify-mini activity-mini"
        :class="{ 'index-done': indexDone }"
        :aria-label="libraryPageLoading ? 'Loading library' : 'Open activity log'"
        @click="indexLogOpen = true"
      >
        <template v-if="libraryPageLoading">
          <div class="identify-mini-spinner" />
        </template>
        <template v-else-if="indexRunning">
          <div v-if="!indexDone" class="identify-mini-spinner" />
          <AppIcon name="success" v-else :size="16" class="success-icon" />
          <span v-if="!indexDone">{{ indexCurrent }}/{{ indexTotal }}</span>
          <span v-else>+{{ indexAdded }}</span>
        </template>
        <template v-else>
          <AppIcon name="activity" :size="15" />
        </template>
        <span v-if="!indexRunning" class="activity-mini-label">Activity</span>
      </button>
    </div>

    <!-- Identify progress mini indicator (top-right when minimized) -->
    <div class="status-pills">
      <!-- Identify progress pill -->
      <div v-if="identifyRunning && identifyMinimized" class="identify-mini" @click="identifyMinimized = false">
        <div v-if="!identifyDone" class="identify-mini-spinner" />
        <span>{{ identifyCurrent }}/{{ identifyTotal }}</span>
        <span v-if="identifyDone" class="identify-mini-done"><AppIcon name="done" /></span>
      </div>
    </div>

    <!-- Identify progress modal -->
    <Transition name="modal">
      <div v-if="identifyRunning && !identifyMinimized" class="modal-overlay" @click.self="identifyMinimized = true">
        <div class="modal identify-modal">
          <div class="modal-header">
            <h3>Identifying tracks <span class="powered-by">powered by <a href="https://acoustid.org" target="_blank">AcoustID</a></span></h3>
            <div style="display:flex;gap:8px">
              <button class="icon-btn" title="Minimize" @click="identifyMinimized = true">
                <AppIcon name="minimize" :size="18" />
              </button>
              <button v-if="identifyDone" class="icon-btn" title="Close" @click="identifyRunning = false">
                <AppIcon name="close" :size="18" />
              </button>
            </div>
          </div>
          <div class="modal-body">
            <div class="identify-bar-wrap">
              <div class="identify-bar-fill" :style="`width:${identifyTotal > 0 ? (identifyCurrent / identifyTotal * 100) : 0}%`" />
            </div>
            <div class="identify-status">{{ identifyCurrent }} / {{ identifyTotal }}{{ identifyDone ? ' — Done' : '' }}</div>
            <div class="identify-results" ref="identifyLogRef">
              <div v-for="(item, idx) in identifyResults" :key="idx"
                class="identify-result-item" :class="'ir-' + item.status">
                <span class="ir-icon">{{ identifyStatusIcon(item.status) }}</span>
                <span class="ir-name" v-if="item.track_name">{{ item.track_name }}</span>
                <span class="ir-text">{{ item.message || item.status.replace('_', ' ') }}</span>
              </div>
              <div v-if="!identifyResults.length && !identifyDone" class="identify-empty">Processing…</div>
            </div>
          </div>
        </div>
      </div>
    </Transition>

    <!-- Index & Sync log modal -->
    <Transition name="modal">
      <div v-if="indexLogOpen" class="modal-overlay" @click.self="indexLogOpen = false; indexRunning = false">
        <div class="modal identify-modal">
          <div class="modal-header">
            <h3>Activity <span class="powered-by">library &amp; sync history</span></h3>
            <button class="icon-btn" title="Close" @click="indexLogOpen = false; indexRunning = false">
              <AppIcon name="close" :size="18" />
            </button>
          </div>
          <div class="modal-body">
            <!-- Active local indexing progress bar -->
            <div v-if="indexRunning && !indexDone" class="log-progress-wrap">
              <div class="log-progress-bar" :style="`width:${indexTotal > 0 ? (indexCurrent / indexTotal * 100) : 0}%`" />
              <span class="log-progress-label">Scanning {{ indexCurrent }}/{{ indexTotal }}</span>
            </div>

            <div class="index-results" ref="indexLogRef">
              <div v-if="!indexLog.length" class="index-empty">No activity yet</div>
              <div v-for="session in [...indexLog].reverse()" :key="session.id" class="log-session" :class="`ls-${session.status} ls-${session.kind}`">
                <!-- Icon + source label -->
                <div class="ls-header">
                  <span class="ls-kind">
                    {{ session.kind === 'local' ? 'Local' : 'Sync' }}
                  </span>
                  <span class="ls-source">
                    <template v-if="session.kind === 'local'">Local library</template>
                    <template v-else>{{ session.device || 'Unknown device' }}</template>
                  </span>
                  <span class="ls-badge" v-if="session.status === 'running'">
                    <span class="ls-spinner" />
                    <template v-if="session.kind === 'sync' && session.currentMessage">{{ session.currentMessage }}</template>
                    <template v-else-if="session.kind === 'sync' && session.filesAdded > 0">{{ session.filesAdded }} files</template>
                    <template v-else>scanning…</template>
                  </span>
                  <span class="ls-badge ls-badge-done" v-else-if="session.status === 'done'">
                    <AppIcon name="done" :size="12" />
                    <template v-if="session.filesAdded > 0">+{{ session.filesAdded }} files</template>
                    <template v-else>up to date</template>
                  </span>
                  <span class="ls-badge ls-badge-err" v-else>
                    <AppIcon name="error" :size="12" />
                    error
                  </span>
                  <span class="ls-time">{{ new Date(session.finishedAt || session.startedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) }}</span>
                </div>
                <!-- Error detail -->
                <div v-if="session.status === 'error' && session.errorMsg" class="ls-error-msg">{{ session.errorMsg }}</div>
                <div v-else-if="session.kind === 'sync' && session.status !== 'running' && session.currentMessage" class="ls-current-msg">
                  {{ session.currentMessage }}
                </div>
                <!-- File list -->
                <div v-if="session.files.length" class="ls-files">
                  <div v-for="(f, i) in session.files" :key="i" class="ls-file-item">
                    <span class="ls-file-icon" />{{ f }}
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </Transition>
    <div class="mobile-seek-wrap" :class="{ active: mobileSeekActive }">
      <div
        class="mobile-seek-hit"
        @pointerdown="mobileSeekHoldStart"
        @pointermove="mobileSeekHoldMove"
        @pointerup="mobileSeekHoldEnd"
        @pointercancel="mobileSeekHoldEnd"
      >
        <div class="mobile-seek-track">
          <div class="mobile-seek-fill" :style="`width:${displayProgressPercent}%`">
            <div class="mobile-seek-thumb" />
          </div>
        </div>
      </div>
    </div>
    <PlayerBar
      :reduce-motion="reducedMotion"
      :is-liked="nowPlaying?.is_liked ?? false"
      :like-pending="!!nowPlaying && pendingLikes.has(nowPlaying.id)"
      @toggle-like="nowPlaying && toggleLike(nowPlaying)"
      v-model:is-shuffled="isShuffled"
      v-model:repeat-mode="repeatMode"
      v-model:show-queue-menu="showQueueMenu"
      v-model:show-device-menu="showDeviceMenu"
      :now-playing="nowPlaying"
      :current-track="currentTrack"
      :beat-scale="beatScale"
      :is-playing="isPlaying"
      :current-time="currentTime"
      :duration="duration"
      :show-soulseek-preview-buffer="showSoulseekPreviewBuffer"
      :soulseek-preview-buffer-percent="soulseekPreviewBufferPercent"
      :display-progress-percent="displayProgressPercent"
      :queue-source="queueSource"
      :queue="queue"
      :output-devices="outputDevices"
      :remote-output-peer="remoteOutputPeer"
      :current-device="currentDevice"
      :current-sample-rate="currentSampleRate"
      :remote-output-devices="remoteOutputDevices"
      :device-menu-error="deviceMenuError"
      :volume="volume"
      :native-sample-rate-label="nativeSampleRateLabel()"
      :peer-label="peerLabel"
      :is-remote-output-peer="isRemoteOutputPeer"
      @open-detail="showDetail = true"
      @play-prev="playPrev"
      @play-next="playNext"
      @toggle-play="togglePlay"
      @seek="seek"
      @toggle-queue-menu="toggleQueueMenu"
      @toggle-device-menu="toggleDeviceMenu"
      @open-playback-settings="openSettings('playback')"
      @jump-to-queue-item="jumpToQueueItem"
      @pick-local-device="pickLocalDevice"
      @pick-local-sample-rate="pickLocalSampleRate"
      @pick-remote-device="pickRemoteDevice"
      @set-volume="setVolume"
    />
  </div>

  <!-- Player detail overlay — teleported to body to escape footer stacking context -->
  <Teleport to="body">
    <Transition name="detail">
      <div v-if="showDetail && nowPlaying" class="player-detail" @click.self="showDetail = false">
          <div class="detail-backdrop" aria-hidden="true">
            <CoverImage class="detail-backdrop-image" :src="currentCoverUrl" :placeholder="currentTrack.colors" />
            <div class="detail-backdrop-wash" :style="detailBackdropWashStyle" />
          </div>
          <div class="detail-sheet">
            <!-- drag handle -->
            <div class="detail-handle" @click="showDetail = false" />
            <!-- cover 3D card -->
            <div class="detail-cover-wrap"
              :style="{
                transform: `scale(${beatScale})`,
                transformOrigin: 'center center',
                willChange: 'transform',
              }"
              @mousemove="onCardMouseMove"
              @mouseenter="onCardMouseEnter"
              @mouseleave="onCardMouseLeave"
              @mousedown="onCardMouseDown"
              @mouseup="onCardMouseUp"
              @touchstart.passive="onCardTouchStart"
              @touchmove.prevent="onCardTouchMove"
              @touchend="onCardTouchEnd"
            >
              <div class="detail-aura-field" :style="detailAuraStyle" aria-hidden="true" />
              <div class="detail-art-stage">
                <WebGLAlbumRenderer
                  :cover-url="coverUrl(nowPlaying.cover_hash, 'medium')"
                  v-if="appearance.album3d && !reducedMotion"
                  :colors="currentTrack.colors"
                  :rarity-color="currentRarityColor"
                  :beat-scale="beatScale"
                  :is-playing="isPlaying"
                  :tilt-x="cardRotX"
                  :tilt-y="cardRotY"
                  :offset-x="cardTX"
                  :offset-y="cardTY"
                  :dragging="cardDragging"
                />
                <CoverImage v-else class="detail-static-cover" :src="currentCoverUrl" :placeholder="currentTrack.colors" />
                <div v-if="!reducedMotion" class="detail-spectrum-ring" :class="{ playing: isPlaying }" :style="detailSpectrumStyle" aria-hidden="true">
                  <div
                    v-for="segment in spectrumSegments"
                    :key="segment.index"
                    class="detail-spectrum-spoke"
                    :style="spectrumSpokeStyle(segment)"
                  >
                    <span class="detail-spectrum-bar" :style="spectrumBarStyle(segment)" />
                  </div>
                </div>
              </div>
            </div>
            <!-- track info -->
            <div class="detail-info">
              <div class="detail-track-name">
                <span class="marquee-text">{{ currentTrack.title }}&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;{{ currentTrack.title }}</span>
              </div>
              <div class="detail-track-artist">
                <span class="marquee-text">{{ currentTrack.artist }}&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;{{ currentTrack.artist }}</span>
              </div>
              <button class="icon-btn" :class="{ green: nowPlaying.is_liked }" @click.stop="toggleLike(nowPlaying!)" style="margin-left:auto;">
                <AppIcon name="liked" v-if="nowPlaying.is_liked" :size="22" />
                <AppIcon name="like" v-else :size="22" />
              </button>
            </div>
            <!-- seek -->
            <div class="detail-seek-wrap">
              <div class="detail-bar"
                @pointerdown="mobileSeekHoldStart"
                @pointermove="mobileSeekHoldMove"
                @pointerup="mobileSeekHoldEnd"
                @pointercancel="mobileSeekHoldEnd"
              >
                <div class="detail-bar-track">
                  <div
                    v-if="showSoulseekPreviewBuffer"
                    class="detail-bar-buffer"
                    :style="`width:${soulseekPreviewBufferPercent}%`"
                  />
                  <div class="detail-bar-fill" :style="`width:${displayProgressPercent}%`">
                    <div class="detail-bar-thumb" />
                  </div>
                </div>
              </div>
              <div class="detail-time-row">
                <span>{{ formatTime(currentTime) }}</span>
                <span>{{ formatTime(duration) }}</span>
              </div>
            </div>
            <!-- controls -->
            <div class="detail-controls">
              <button class="icon-btn" :class="{ green: isShuffled }" @click="isShuffled = !isShuffled">
                <AppIcon name="shuffle" :size="20" />
              </button>
              <button class="icon-btn" @click="playPrev">
                <AppIcon name="previous" :size="32" />
              </button>
              <button class="detail-play-btn" @click="togglePlay">
                <AppIcon name="play" v-if="!isPlaying" :size="28" />
                <AppIcon name="pause" v-else :size="28" />
              </button>
              <button class="icon-btn" @click="playNext">
                <AppIcon name="next" :size="32" />
              </button>
              <button class="icon-btn" :class="{ green: repeatMode > 0 }" @click="repeatMode = (repeatMode + 1) % 3">
                <AppIcon name="repeat" v-if="repeatMode < 2" :size="20" />
                <AppIcon name="repeat-one" v-else :size="20" />
              </button>
            </div>
          </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.detail-static-cover { position: absolute; inset: 15%; border-radius: 14px; box-shadow: 0 20px 50px color-mix(in srgb, var(--color-shadow) 40%, transparent); }
.page-title-input { width: 100%; min-width: 0; padding: 0; border: 0; border-radius: 4px; background: transparent; color: inherit; font: inherit; letter-spacing: inherit; }
.page-title-input:focus-visible { outline: 2px solid var(--color-accent); outline-offset: -2px; }

/* ── Layout ── */
.app {
  display: grid;
  grid-template-columns: 220px 1fr;
  grid-template-rows: 1fr 90px;
  height: 100vh;
  color: var(--color-text);
  background: var(--color-bg);
}

.brand {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 12px;
  margin-bottom: 14px;
  cursor: pointer;
}
.brand-name { font-size: 20px; font-weight: 800; letter-spacing: -0.5px; }

.discovery-header {
  align-items: flex-end;
}
.discovery-toolbar {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
  flex-wrap: wrap;
}
.discovery-summary {
  margin: 0 0 18px;
}
.discovery-empty {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 6px;
  padding: 12px 0 22px;
}
.discovery-list { display: flex; flex-direction: column; gap: 0; }
.discovery-row {
  align-items: flex-start;
  cursor: default;
  padding-top: 10px;
  padding-bottom: 10px;
}
.peer-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(135deg, #24304a, #11161f);
  color: var(--color-text);
  font-size: 18px;
}
.discovery-info {
  gap: 4px;
}
.peer-playback-icon {
  width: 14px;
  height: 14px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}
.discovery-status-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 2px 8px;
  border-radius: 999px;
  font-size: var(--fs-badge);
  font-weight: 700;
  background: color-mix(in srgb, var(--color-text) 8%, transparent);
  color: var(--color-text);
  flex-shrink: 0;
}
.discovery-status-pill.peer-status-playing { background: color-mix(in srgb, var(--color-accent) 16%, transparent); color: var(--color-accent-text); }
.discovery-status-pill.peer-status-paused { background: color-mix(in srgb, var(--color-warning) 16%, transparent); color: var(--color-warning); }
.discovery-status-pill.peer-status-stopped,
.discovery-status-pill.peer-status-ended,
.discovery-status-pill.peer-status-idle { background: color-mix(in srgb, var(--color-text) 6%, transparent); color: var(--color-text-muted); }
.discovery-meta {
  white-space: normal;
}
.discovery-sync-block {
  display: flex;
  flex-direction: column;
  gap: 5px;
  margin-top: 2px;
  max-width: 420px;
}
.sync-status-row {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}
.discovery-output-pill {
  flex-shrink: 0;
}

.sync-toggle {
  display: flex; align-items: center; gap: 6px;
  padding: 5px 12px; border-radius: 20px; border: 1px solid var(--color-border-strong);
  background: transparent; color: var(--color-text-muted); font-size: var(--fs-control); font-weight: 600;
  cursor: pointer; transition: all 0.15s;
}
.sync-toggle:hover { border-color: var(--color-text); color: var(--color-text); }
.sync-toggle.active { background: var(--color-accent); border-color: var(--color-accent); color: var(--color-on-accent); }

.settings-error { color: var(--color-danger-text); font-size: var(--fs-body-sm); }

/* ── Duplicates view ── */
.dedup-toolbar {
  display: flex; gap: 8px; align-items: center; flex-wrap: wrap;
}
.dedup-filter-btn {
  padding: 5px 12px; border-radius: 20px; border: 1px solid var(--color-surface-hover);
  background: transparent; color: var(--color-text-muted); cursor: pointer;
  font-size: var(--fs-body-sm); transition: background 0.12s, color 0.12s, border-color 0.12s;
}
.dedup-filter-btn:hover { border-color: var(--color-border-strong); color: var(--color-text); }
.dedup-filter-btn.active { background: var(--color-accent); color: var(--color-on-accent); border-color: var(--color-accent); }
.dedup-rescan-btn {
  display: flex; align-items: center; gap: 6px;
  padding: 5px 14px; font-size: var(--fs-body-sm);
}
.dedup-summary {
  margin: 10px 0 18px; color: var(--color-text-muted); font-size: var(--fs-body-sm);
}
.dedup-clean-copy {
  display: flex; align-items: center; gap: 8px; color: var(--color-text-muted);
}
.dedup-error { margin-top: 16px; }
.dedup-group {
  margin-bottom: 20px; border: 1px solid var(--color-border); border-radius: 8px; overflow: hidden;
}
.dedup-group-header {
  padding: 10px 16px 8px; background: var(--color-card);
  display: flex; align-items: flex-start; gap: 12px; flex-wrap: wrap;
}
.dedup-group-title {
  font-size: var(--fs-body-sm); color: var(--color-text); font-weight: 600;
  display: flex; align-items: center; gap: 8px;
}
.dedup-track-count {
  font-size: 11px; color: var(--color-text-muted); font-weight: 400;
}
.dedup-reasons {
  display: flex; gap: 6px; flex-wrap: wrap; margin-left: auto;
}
.dedup-reason-tag {
  padding: 2px 8px; border-radius: 10px; background: var(--color-surface);
  font-size: 10px; color: var(--color-text-faint); font-family: monospace;
}
.dedup-group-tracks { padding: 4px 0; }
.dedup-track-row {
  cursor: pointer;
}
.dedup-track-row.dedup-keep {
  background: color-mix(in srgb, var(--color-accent) 8%, transparent);
}
.dedup-track-row.dedup-remove {
  opacity: 0.55;
}
.dedup-track-meta {
  display: flex; gap: 10px; font-size: 11px; color: var(--color-text-faint); margin-top: 2px;
  flex-wrap: wrap;
}
.dedup-edited-tag {
  background: color-mix(in srgb, var(--color-accent) 15%, transparent); color: var(--color-accent); padding: 1px 6px; border-radius: 4px;
  font-size: 10px;
}
.dedup-already-marked {
  outline: 1px solid color-mix(in srgb, var(--color-warning) 20%, transparent);
  background: color-mix(in srgb, var(--color-warning) 5%, transparent);
}
.dedup-flagged-tag {
  background: color-mix(in srgb, var(--color-warning) 15%, transparent); color: var(--color-warning); padding: 1px 6px; border-radius: 4px;
  font-size: 10px;
}
.dedup-keep-badge {
  background: var(--color-accent); color: var(--color-on-accent); font-size: 10px; font-weight: 700;
  padding: 1px 6px; border-radius: 4px; margin-left: 8px;
}
.dedup-remove-badge {
  background: color-mix(in srgb, var(--color-danger) 13%, transparent); color: var(--color-danger-text); font-size: 10px;
  padding: 1px 6px; border-radius: 4px; margin-left: 8px;
}
.dedup-keep-radio {
  flex-shrink: 0; display: flex; align-items: center; justify-content: center;
  width: 24px; margin-left: 4px;
}
.dedup-radio-dot {
  width: 14px; height: 14px; border-radius: 50%; border: 2px solid var(--color-border-strong);
  transition: border-color 0.12s, background 0.12s;
}
.dedup-radio-dot.active {
  border-color: var(--color-accent); background: var(--color-accent);
}
.dedup-actions {
  margin-top: 24px; margin-bottom: 12px; display: flex; justify-content: flex-end;
}

.sync-now-btn {
  flex-shrink: 0; width: 32px; height: 32px; border-radius: 50%;
  background: var(--color-surface); border: none; color: var(--color-text-muted);
  display: flex; align-items: center; justify-content: center;
  cursor: pointer; transition: background 0.15s, color 0.15s;
}
.sync-now-btn:hover:not(:disabled) { background: var(--color-accent); color: var(--color-on-accent); }
.sync-now-btn:disabled { opacity: 0.4; cursor: default; }
.discovery-sync-btn { margin-left: 10px; }

.sync-bar-wrap {
  position: relative;
  height: 8px;
  background: var(--color-surface);
  border-radius: 4px;
  overflow: hidden;
  margin-top: 1px;
}
.sync-bar {
  height: 5px;
  background: color-mix(in srgb, var(--color-accent) 92%, transparent);
  border-radius: 4px 4px 2px 2px;
  transition: width 0.22s linear;
}
.sync-bar-overall {
  position: absolute;
  left: 0;
  bottom: 0;
  height: 2px;
  background: color-mix(in srgb, var(--color-text) 32%, transparent);
  border-radius: 0 0 4px 4px;
  transition: width 0.3s ease;
}
.sync-status {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-powered-by);
  color: var(--color-text-muted);
}
.sync-count {
  flex-shrink: 0;
  font-size: var(--fs-powered-by);
  color: var(--color-text-faint);
  font-variant-numeric: tabular-nums;
}
.sync-done { color: var(--color-accent); }
.sync-error { color: var(--color-danger-text); }

/* ── Main area ── */
.main {
  grid-row: 1 / 2;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: linear-gradient(180deg, var(--color-hero) 0%, color-mix(in oklab, var(--color-hero) 33%, var(--color-bg-elevated)) 30%, var(--color-bg-elevated) 58%);
}


.user-menu-wrapper { position: relative; }
.user-btn {
  display: flex; align-items: center; gap: 8px;
  background: color-mix(in srgb, var(--color-shadow) 50%, transparent);
  border: none; color: var(--color-text);
  padding: 4px 10px 4px 4px;
  border-radius: 20px;
  cursor: pointer; font-size: 13px; font-weight: 700;
}
.user-btn:hover { background: color-mix(in srgb, var(--color-shadow) 80%, transparent); }
.avatar {
  width: 28px; height: 28px; border-radius: 50%;
  background: var(--color-border-strong);
  display: flex; align-items: center; justify-content: center;
  font-size: 13px; font-weight: 700; text-transform: uppercase;
}

.dropdown a {
  display: flex; align-items: center; justify-content: space-between;
  padding: 10px 16px;
  color: var(--color-text); text-decoration: none; font-size: 14px; cursor: pointer;
}
.dropdown a:hover { background: var(--color-surface-hover); }
.dropdown-sep { height: 1px; background: var(--color-surface-hover); margin: 4px 0; }

/* ── Content ── */
.content {
  flex: 1; overflow-y: scroll;
  scrollbar-gutter: stable;
  padding: 0 24px 32px;
}
.content::-webkit-scrollbar { width: 8px; }
.content::-webkit-scrollbar-thumb { background: var(--color-border-strong); border-radius: 4px; }
.content::-webkit-scrollbar-track { background: transparent; }

section { margin-bottom: 36px; }
section h2 { font-size: var(--fs-h2); font-weight: 800; margin-bottom: 16px; }

.section-head {
  display: flex; align-items: baseline; justify-content: space-between;
  margin-bottom: 16px;
}
.section-head h2 { margin-bottom: 0; }
.history-list {
  display: flex; flex-direction: column; gap: 2px;
}
.show-all {
  font-size: var(--fs-section-link); font-weight: 700;
  text-transform: uppercase; letter-spacing: .08em;
  color: var(--color-text-muted); text-decoration: none;
}
.show-all:hover { color: var(--color-text); text-decoration: underline; }

.grid {
  display: grid;
  grid-template-columns: repeat(6, 1fr);
  gap: 16px;
}
.card-list {
  width: 100%;
  display: flex;
  overflow-x: auto;
  scrollbar-width: none;
  gap: 16px;
}

.card {
  background: var(--color-card); border-radius: 6px; padding: 14px;
  cursor: pointer; transition: background .18s; position: relative;
  width: 150px;
  flex-shrink: 0;
}
.card:hover { background: var(--color-card-hover) !important; }
.card.rarity-tint { background: var(--rarity-tint); }
.card:hover .hover-play { opacity: 1; transform: translateY(0); }

.recent-onboarding-list {
  align-items: flex-start;
  min-height: 236px;
}
.onboarding-music-card {
  border: 0;
  color: inherit;
  font: inherit;
  text-align: left;
}
.onboarding-music-card:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: -2px;
}
.onboarding-action-cover {
  display: grid;
  place-items: center;
  box-shadow: inset 0 1px 0 color-mix(in srgb, var(--color-text) 14%, transparent);
}
.onboarding-sync-cover {
  background: linear-gradient(145deg, #36c9b4, #176c7c 58%, #222449);
}
.onboarding-upload-cover {
  background: linear-gradient(145deg, #9b6be7, #5a3b9c 56%, #26233f);
}
.onboarding-action-cover > svg {
  width: 56px;
  height: 56px;
  color: rgba(255, 255, 255, 0.94);
  filter: drop-shadow(0 8px 12px color-mix(in srgb, var(--color-shadow) 24%, transparent));
}
.onboarding-card-go {
  color: var(--color-on-accent);
  font-size: 22px;
  font-weight: 800;
}
.recent-onboarding-guide {
  position: relative;
  width: 190px;
  height: 236px;
  flex: 0 0 190px;
  pointer-events: none;
}
.recent-onboarding-zippy {
  position: absolute;
  left: -54px;
  bottom: -42px;
  width: 230px;
  max-width: none;
  filter: drop-shadow(0 18px 24px color-mix(in srgb, var(--color-shadow) 30%, transparent));
  transform-origin: center bottom;
  animation:
    recent-zippy-enter 650ms cubic-bezier(.18, .86, .28, 1.16) both,
    recent-zippy-float 3.6s ease-in-out 650ms infinite;
}
@keyframes recent-zippy-enter {
  from { opacity: 0; transform: translate3d(34px, 24px, 0) rotate(3deg) scale(0.92); }
  to { opacity: 1; transform: translate3d(0, 0, 0) rotate(0) scale(1); }
}
@keyframes recent-zippy-float {
  0%, 100% { transform: translateY(0) rotate(0); }
  50% { transform: translateY(-5px) rotate(-0.7deg); }
}
@media (prefers-reduced-motion: reduce) {
  .recent-onboarding-zippy {
    animation: none;
  }
}
@media (max-width: 620px) {
  .recent-onboarding-list {
    flex-wrap: wrap;
    overflow-x: visible;
  }
  .onboarding-music-card {
    width: calc((100% - 16px) / 2);
    min-width: 0;
  }
  .recent-onboarding-guide {
    width: 100%;
    height: 250px;
    flex: 0 0 100%;
  }
  .recent-onboarding-zippy {
    right: 4%;
    bottom: -38px;
    left: auto;
  }
}

.cover {
  width: 100%; aspect-ratio: 1;
  border-radius: 4px; margin-bottom: 14px;
  position: relative; overflow: hidden;
}

.hover-play {
  position: absolute; bottom: 8px; right: 8px;
  opacity: 0; transform: translateY(6px);
  transition: opacity .2s, transform .2s;
}
.green-circle {
  width: 42px; height: 42px; border-radius: 50%;
  background: var(--color-accent); border: none;
  color: var(--color-on-accent);
  display: flex; align-items: center; justify-content: center;
  cursor: pointer;
  box-shadow: 0 8px 24px color-mix(in srgb, var(--color-shadow) 50%, transparent);
  transition: transform .1s, background .1s;
}
.green-circle:hover { transform: scale(1.06); background: var(--color-accent-hover); }

.card-title {
  font-size: var(--fs-card-title); font-weight: 700;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  margin-bottom: 4px;
}
.card-artist {
  font-size: var(--fs-card-meta); color: var(--color-text-muted);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}

.icon-btn {
  background: none; border: none;
  color: var(--color-text-muted); cursor: pointer; padding: 4px;
  display: flex; align-items: center; justify-content: center;
  position: relative; border-radius: 4px;
  transition: color .1s;
}
.icon-btn:hover { color: var(--color-text); }
.icon-btn.green { color: var(--color-accent); }
.success-icon { color: var(--color-accent); }
.pinned-cover-icon { color: color-mix(in srgb, var(--color-text) 88%, transparent); }
.icon-btn.green:hover { color: var(--color-accent-hover); }
.icon-btn.dot::after {
  content: '';
  position: absolute; bottom: -3px; left: 50%;
  transform: translateX(-50%);
  width: 4px; height: 4px; border-radius: 50%;
  background: var(--color-accent);
}

/* ── Library view ── */
.library-header {
  display: flex; align-items: center; justify-content: space-between;
  margin-bottom: 16px; gap: 16px;
}
.library-header.page-toolbar { justify-content: flex-end; }
.library-page-controls {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin: -4px 0 12px;
  color: var(--color-text-muted);
  font-size: 13px;
}
.library-page-controls > div { display: flex; gap: 6px; }
.library-page-controls button { gap: 6px; }
.library-page-controls-bottom { margin: 18px 0 0; justify-content: center; }
.library-load-more { display: flex; justify-content: center; padding: 18px 0; }
.library-search {
  background: var(--color-surface); border: none; border-radius: 4px;
  color: var(--color-text); font-size: var(--fs-input); padding: 8px 12px;
  width: 220px; outline: none;
}
.library-search::placeholder { color: var(--color-text-faint); }
.library-search:focus { outline: 1px solid var(--color-border-strong); }
.library-empty {
  color: var(--color-text-muted); font-size: var(--fs-empty); padding: 32px 0;
}
.search-empty-copy {
  color: var(--color-text-muted);
  font-size: var(--fs-body-md);
  margin: 0 0 16px;
}
.search-section-head {
  margin-top: 6px;
}
.search-inline-empty {
  padding: 14px 0 18px;
}
.search-summary {
  display: flex;
  flex-wrap: wrap;
  gap: 14px;
  margin: 0 0 14px;
  color: var(--color-text-muted);
  font-size: var(--fs-body-md);
}
.search-summary strong {
  color: var(--color-text);
  font-weight: 700;
}
.soulseek-section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-top: 22px;
}
.soulseek-section-actions {
  display: flex;
  align-items: center;
  gap: 12px;
}
.soulseek-search-btn {
  padding: 6px 12px;
}
.soulseek-search-btn:disabled {
  opacity: 0.45;
  cursor: default;
}
.soulseek-settings-link {
  background: transparent;
  border: none;
  color: var(--color-accent-text);
  cursor: pointer;
  padding: 0;
}
.soulseek-list {
  gap: 0;
}
.soulseek-row {
  align-items: center;
}
.soulseek-cover-sm {
  display: flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(135deg, color-mix(in srgb, var(--color-accent) 20%, var(--color-card)), color-mix(in srgb, var(--color-accent) 8%, var(--color-bg)));
  color: var(--color-accent-text);
  font-size: 11px;
  font-weight: 800;
  letter-spacing: .08em;
  text-transform: uppercase;
}
.soulseek-status-pill {
  display: inline-flex;
  align-items: center;
  padding: 2px 8px;
  border-radius: 999px;
  font-size: var(--fs-badge);
  font-weight: 700;
  background: color-mix(in srgb, var(--color-text) 8%, transparent);
  color: var(--color-text);
}
.soulseek-status-pill.state-completed {
  background: color-mix(in srgb, var(--color-accent) 16%, transparent);
  color: var(--color-accent-text);
}
.soulseek-status-pill.state-failed,
.soulseek-status-pill.state-timed_out,
.soulseek-status-pill.state-cancelled {
  background: color-mix(in srgb, var(--color-danger) 16%, transparent);
  color: var(--color-danger-text);
}
.soulseek-progress-copy {
  font-size: var(--fs-body-sm);
  color: var(--color-text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.soulseek-more-copy {
  padding-top: 10px;
}
.soulseek-inline-error {
  color: var(--color-danger-text);
}
.soulseek-side {
  display: flex;
  flex-direction: row;
  align-items: flex-end;
  gap: 12px;
  margin-left: auto;
  flex-shrink: 0;
  min-width: 0;
}
.soulseek-speed {
  font-size: var(--fs-body-sm);
  color: var(--color-text-muted);
  white-space: nowrap;
}
.soulseek-download-btn {
  min-width: 96px;
  padding: 6px 14px;
}
.soulseek-replace-page {
  display: flex;
  flex-direction: column;
  gap: 18px;
}
.soulseek-replace-page-header {
  align-items: flex-start;
}
.soulseek-replace-page-heading {
  display: flex;
  align-items: flex-start;
  gap: 12px;
}
.soulseek-replace-back-btn {
  flex-shrink: 0;
  margin-top: 2px;
}
.soulseek-replace-track-copy {
  color: var(--color-text-muted);
  line-height: 1.5;
  margin: 0;
}
.soulseek-replace-search-row {
  display: flex;
  align-items: center;
  gap: 10px;
}
.soulseek-replace-search-row input {
  flex: 1;
}
.soulseek-replace-results {
  max-height: none;
  overflow: visible;
  padding-right: 0;
}
.soulseek-replace-actions {
  align-items: center;
  gap: 8px;
}
.soulseek-download-icon-btn {
  width: 32px;
  height: 32px;
  border: 1px solid var(--color-border-strong);
  border-radius: 999px;
  flex-shrink: 0;
}
.soulseek-download-icon-btn:hover:not(:disabled) {
  border-color: var(--color-text);
}
.soulseek-replace-compact-btn {
  padding: 5px 12px;
  min-width: 0;
  font-size: var(--fs-body-sm);
  border-radius: 16px;
}
.soulseek-replace-btn {
  min-width: 124px;
  padding: 6px 14px;
}
.soulseek-download-btn:disabled,
.btn-primary:disabled,
.soulseek-download-icon-btn:disabled {
  opacity: 0.72;
  cursor: default;
  transform: none;
}
.text-action-btn { font-size: var(--fs-button); font-weight: 600; }
.btn-primary.playlist-action,
.btn-secondary.playlist-action {
  display: inline-flex; align-items: center; justify-content: center; gap: 8px;
  min-height: 36px; padding: 8px 16px; white-space: nowrap;
}
.playlist-action:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 3px; }

.history-time {
  font-size: var(--fs-body-sm);
  color: var(--color-text-muted);
  white-space: nowrap;
}
.smart-track-count {
  font-size: var(--fs-body-sm);
  color: var(--color-text-muted);
}
.smart-empty-values {
  color: var(--color-text-faint);
  font-size: var(--fs-body-sm);
}
.device-emoji {
  font-size: calc(var(--fs-peer-title) + 2px);
  line-height: 1;
}
.discovery-caption {
  color: var(--color-text-muted);
  font-size: var(--fs-body-sm);
}
.discovery-help-text {
  font-size: var(--fs-body-sm);
  color: var(--color-text-faint);
}
/* Playlist menu */
.playlist-menu {
  position: fixed; z-index: 1001;
  background: var(--color-surface); border: 1px solid var(--color-surface-hover); border-radius: 8px;
  min-width: 180px; max-width: 240px;
  box-shadow: 0 8px 24px color-mix(in srgb, var(--color-shadow) 50%, transparent);
  overflow: hidden;
  -webkit-user-select: none;
  user-select: none;
  -webkit-touch-callout: none;
}
.responsive-popup-desktop.playlist-menu,
.responsive-popup-mobile-panel.playlist-menu {
  right: auto;
  padding: 0;
  min-width: 180px;
  max-width: 240px;
  z-index: 1001;
}
.responsive-popup-mobile-panel.playlist-menu {
  max-width: calc(100vw - 24px);
}
.dedup-confirm-popup .modal-header {
  padding: 14px 16px 8px;
}
.dedup-confirm-popup .modal-header h3 {
  color: var(--color-text);
}
.dedup-confirm-popup .modal-body {
  padding: 8px 16px 4px;
}
.playlist-menu-header {
  font-size: var(--fs-eyebrow); font-weight: 700; text-transform: uppercase;
  letter-spacing: .06em; color: var(--color-text-muted);
  padding: 10px 14px 6px;
  border-bottom: 1px solid var(--color-surface-hover);
  -webkit-user-select: none;
  user-select: none;
  -webkit-touch-callout: none;
}
.playlist-menu-empty {
  font-size: var(--fs-body-sm); color: var(--color-text-faint); padding: 10px 14px;
  -webkit-user-select: none;
  user-select: none;
  -webkit-touch-callout: none;
}
.playlist-menu-item {
  display: block; width: 100%; text-align: left;
  background: none; border: none; color: var(--color-text);
  font-size: var(--fs-button); padding: 9px 14px; cursor: pointer;
  -webkit-user-select: none;
  user-select: none;
  -webkit-touch-callout: none;
}
.playlist-menu-item:hover { background: var(--color-surface-hover); }
.playlist-menu-item.danger { color: var(--color-danger-text); }
.playlist-menu-item.danger:hover { background: color-mix(in srgb, var(--color-danger) 12%, transparent); }

/* Smart Playlists */
.sp-match-row {
  display: flex; align-items: center; gap: 8px;
  font-size: var(--fs-body-md); color: var(--color-text-muted); padding: 0 0 14px;
}
.sp-match-btn {
  background: transparent; border: 1px solid var(--color-border-strong); color: var(--color-text-muted);
  border-radius: 20px; padding: 4px 12px; font-size: var(--fs-body-sm);
  font-weight: 600; cursor: pointer; transition: all .15s;
}
.sp-match-btn.active, .sp-match-btn:hover { background: var(--color-accent); border-color: var(--color-accent); color: var(--color-on-accent); }
.sp-rules { display: flex; flex-direction: column; gap: 8px; margin-bottom: 20px; }
.sp-rule-row {
  display: flex; align-items: center; gap: 8px;
  background: var(--color-card); border-radius: 12px; padding: 8px 12px;
  flex-wrap: wrap;
}
.sp-op-label { font-size: var(--fs-body-sm); color: var(--color-text-muted); white-space: nowrap; }
.sp-text-input { flex: 1; min-width: 100px; }
.sp-num-input { width: 80px; }
.sp-add-rule-btn {
  background: transparent; border: 1px solid var(--color-border-strong); color: var(--color-text-muted);
  border-radius: 20px; padding: 5px 14px; font-size: var(--fs-body-sm); font-weight: 600;
  cursor: pointer; transition: all .15s;
  align-self: flex-start;
}
.sp-add-rule-btn:hover { border-color: var(--color-text); color: var(--color-text); }
.sp-multiselect {
  display: flex; flex-wrap: wrap; gap: 6px; align-items: center; flex: 1;
}
.sp-chip {
  background: var(--color-surface); border: 1px solid var(--color-border-strong); color: var(--color-text-muted);
  border-radius: 20px; padding: 3px 10px; font-size: var(--fs-body-sm);
  cursor: pointer; transition: background .12s, color .12s, border-color .12s;
  user-select: none;
}
.sp-chip.selected { background: var(--color-accent); border-color: var(--color-accent); color: var(--color-on-accent); font-weight: 600; }
.sp-chip:hover:not(.selected) { border-color: var(--color-border-strong); color: var(--color-text); }
.sp-preview-header {
  font-size: var(--fs-eyebrow); font-weight: 700; text-transform: uppercase;
  letter-spacing: .06em; color: var(--color-text-muted);
  padding: 0 0 10px; border-bottom: 1px solid var(--color-border); margin-bottom: 8px;
}
.sp-preview { opacity: 0.9; }
.track-groups { display: flex; flex-direction: column; gap: 24px; }
.group-artist {
  font-size: var(--fs-group); font-weight: 700; color: var(--color-text-muted);
  text-transform: uppercase; letter-spacing: .04em;
  padding-bottom: 8px; border-bottom: 1px solid var(--color-border);
  margin-bottom: 4px;
}
.group-artist-link { color: inherit; text-decoration: none; }
.group-artist-link:hover { color: var(--color-text); text-decoration: underline; }
.track-row {
  display: flex; align-items: center; gap: 12px;
  padding: 8px 8px; border-radius: 4px;
  cursor: pointer; user-select: none;
}
.track-row:hover { background: var(--color-card-hover) !important; }
.appearance-track-preview { width: 100%; pointer-events: none; }
.track-row.rarity-tint { background: var(--rarity-tint); }
.track-cover-sm {
  width: 36px; height: 36px; border-radius: 3px; flex-shrink: 0;
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
.track-row.track-row-current {
  background: color-mix(in srgb, var(--color-accent) 14%, transparent) !important;
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--color-accent) 30%, transparent);
}
.track-row.track-row-next {
  background: color-mix(in srgb, var(--color-accent) 8%, transparent) !important;
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

/* ── Edit modal ── */
.modal-overlay {
  position: fixed; inset: 0;
  background: color-mix(in srgb, var(--color-shadow) 65%, transparent);
  display: flex; align-items: center; justify-content: center;
  z-index: 500;
}
.modal {
  background: var(--color-surface); border-radius: 8px;
  width: 400px; max-width: 90vw;
  box-shadow: 0 16px 48px color-mix(in srgb, var(--color-shadow) 60%, transparent);
}
.modal-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 20px 24px 12px;
}
.modal-header h3 { font-size: var(--fs-modal-title); font-weight: 700; }
.modal-body { padding: 8px 24px 16px; display: flex; flex-direction: column; gap: 14px; }
.modal-copy { color: var(--color-text-muted); line-height: 1.5; }
.modal-copy strong { color: var(--color-text); }
.field {
  display: flex; flex-direction: column; gap: 4px;
}
.field > span {
  font-size: var(--fs-field-label); font-weight: 600; color: var(--color-text-muted); text-transform: uppercase; letter-spacing: .03em;
}
.field input {
  background: var(--color-surface-hover); border: none; border-radius: 4px;
  color: var(--color-text); font-size: var(--fs-field-input); padding: 10px 12px; outline: none;
  min-height: 42px;
  line-height: 1.2;
}
.field input:focus { outline: 1px solid var(--color-accent); }
.edit-track-modal { width: 460px; }
.edit-track-body {
  max-height: min(70vh, 560px);
  overflow-y: auto;
}
.modal-footer {
  display: flex; justify-content: flex-end; gap: 10px;
  padding: 12px 24px 20px;
}
.btn-secondary {
  background: transparent; border: 1px solid var(--color-border-strong); border-radius: 20px;
  color: var(--color-text); font-size: var(--fs-button); font-weight: 700;
  padding: 8px 24px; cursor: pointer;
}
.btn-secondary:hover { border-color: var(--color-text); }
.btn-primary {
  background: var(--color-accent); border: none; border-radius: 20px;
  color: var(--color-on-accent); font-size: var(--fs-button); font-weight: 700;
  padding: 8px 28px; cursor: pointer;
}
.btn-primary:hover { background: var(--color-accent-hover); transform: scale(1.02); }

.modal-enter-active, .modal-leave-active { transition: opacity .15s; }
.modal-enter-from, .modal-leave-to { opacity: 0; }
.nav-overlay-enter-active, .nav-overlay-leave-active { transition: opacity .25s; }
.nav-overlay-enter-from, .nav-overlay-leave-to { opacity: 0; }

.dropdown-header {
  padding: 10px 16px 6px;
  font-size: var(--fs-dropdown-label);
  font-weight: 700;
  color: var(--color-text-muted);
  text-transform: uppercase;
  letter-spacing: .04em;
}
/* ── Identify progress ── */
/* Activity tab tucked behind the bottom player bar on desktop. */
.activity-dock {
  position: fixed;
  right: 24px;
  bottom: 90px;
  z-index: 90;
  transform: translateY(calc(100% - 8px));
  transition: transform 0.18s ease;
}
.activity-dock:hover,
.activity-dock:focus-within,
.activity-dock.activity-active {
  transform: translateY(-10px);
}
.activity-mini {
  border-radius: 18px 18px 0 0;
  border-bottom: none;
  min-height: 34px;
}
.activity-mini-label {
  color: var(--color-text);
}

/* ── Status pills container ── */
.status-pills {
  position: fixed;
  top: 16px;
  right: 24px;
  z-index: 400;
  display: flex;
  gap: 8px;
  align-items: center;
}
.identify-mini {
  display: flex;
  align-items: center;
  gap: 8px;
  background: var(--color-surface);
  border: 1px solid var(--color-surface-hover);
  border-radius: 20px;
  padding: 6px 14px;
  cursor: pointer;
  font-size: var(--fs-status-pill);
  font-weight: 600;
  color: var(--color-text);
  box-shadow: 0 4px 16px color-mix(in srgb, var(--color-shadow) 50%, transparent);
  transition: background .12s;
}
.identify-mini:hover { background: var(--color-surface-hover); cursor: pointer; }
.identify-mini.index-done { border-color: var(--color-accent); }
.identify-mini-spinner {
  width: 14px; height: 14px;
  border: 2px solid var(--color-border-strong);
  border-top-color: var(--color-accent);
  border-radius: 50%;
  animation: ident-spin .8s linear infinite;
}
.identify-mini-done { color: var(--color-accent); font-size: 14px; }
@keyframes ident-spin { to { transform: rotate(360deg); } }
.identify-modal { width: 440px; }
.powered-by { font-size: var(--fs-powered-by); font-weight: 400; color: var(--color-text-faint); margin-left: 6px; }
.powered-by a { color: var(--color-accent); text-decoration: none; }
.powered-by a:hover { text-decoration: underline; }
.identify-bar-wrap {
  height: 4px;
  background: var(--color-border-strong);
  border-radius: 2px;
  overflow: hidden;
  margin-bottom: 8px;
}
.identify-bar-fill {
  height: 100%;
  background: var(--color-accent);
  border-radius: 2px;
  transition: width .3s;
}
.identify-status {
  font-size: var(--fs-body-sm);
  color: var(--color-text-muted);
  margin-bottom: 12px;
}
.identify-results {
  max-height: 260px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.identify-results::-webkit-scrollbar { width: 6px; }
.identify-results::-webkit-scrollbar-thumb { background: var(--color-border-strong); border-radius: 3px; }
.identify-results::-webkit-scrollbar-track { background: transparent; }
.identify-result-item {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 6px 8px;
  border-radius: 4px;
  font-size: var(--fs-body-md);
  min-width: 0;
}
.ir-icon { width: 16px; text-align: center; flex-shrink: 0; }
.ir-name {
  color: var(--color-text-muted);
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 140px;
  flex-shrink: 0;
}
.ir-text {
  white-space: pre-wrap;
  word-break: break-all;
  min-width: 0;
  flex: 1;
}
.ir-found { color: var(--color-accent); }
.ir-not_found { color: var(--color-text-muted); }
.ir-error { color: var(--color-danger); }
.ir-fingerprinting { color: var(--color-info); }
.ir-looking_up { color: var(--color-warning); }
.ir-done { color: var(--color-accent); font-weight: 600; }
.identify-empty {
  color: var(--color-text-muted);
  font-size: var(--fs-body-md);
  padding: 12px 0;
  text-align: center;
}

/* ── Index & Sync log ── */
.index-results {
  max-height: 320px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 2px 0;
}
.index-results::-webkit-scrollbar { width: 6px; }
.index-results::-webkit-scrollbar-thumb { background: var(--color-border-strong); border-radius: 3px; }
.index-results::-webkit-scrollbar-track { background: transparent; }
.index-empty { padding: 40px 20px; text-align: center; color: var(--color-text-muted); font-size: var(--fs-empty); }

/* Progress bar shown above the list while scanning */
.log-progress-wrap {
  position: relative;
  height: 4px;
  background: var(--color-surface-hover);
  border-radius: 2px;
  margin-bottom: 12px;
  overflow: hidden;
}
.log-progress-bar {
  height: 100%;
  background: var(--color-accent);
  border-radius: 2px;
  transition: width 0.3s;
}
.log-progress-label {
  position: absolute;
  right: 0;
  top: 7px;
  font-size: var(--fs-powered-by);
  color: var(--color-text-muted);
}

/* Session card */
.log-session {
  background: var(--color-card);
  border-radius: 8px;
  padding: 10px 12px;
  border-left: 3px solid var(--color-surface-hover);
}
.ls-local  { border-left-color: var(--color-info); }
.ls-sync   { border-left-color: var(--color-accent); }
.ls-error  { border-left-color: var(--color-danger); }

.ls-header {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-body-md);
}
.ls-kind {
  flex-shrink: 0;
  min-width: 42px;
  padding: 2px 6px;
  border-radius: 4px;
  background: var(--color-surface);
  color: var(--color-text-muted);
  font-size: 10px;
  font-weight: 800;
  line-height: 1.2;
  text-align: center;
  text-transform: uppercase;
  letter-spacing: 0;
}
.ls-source { font-weight: 600; color: var(--color-text); flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ls-time { font-size: var(--fs-powered-by); color: var(--color-text-faint); flex-shrink: 0; }

.ls-badge {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: var(--fs-body-sm);
  color: var(--color-text-muted);
  white-space: nowrap;
  flex-shrink: 0;
}
.ls-badge-done { color: var(--color-accent); }
.ls-badge-err  { color: var(--color-danger); }

.ls-spinner {
  display: inline-block;
  width: 10px; height: 10px;
  border: 2px solid var(--color-border-strong);
  border-top-color: var(--color-text);
  border-radius: 50%;
  animation: spin 0.7s linear infinite;
}
.ls-error-msg {
  margin-top: 6px;
  font-size: var(--fs-body-sm);
  color: var(--color-danger-text);
  word-break: break-all;
}
.ls-current-msg {
  margin-top: 6px;
  font-size: var(--fs-body-sm);
  color: var(--color-text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.ls-files {
  margin-top: 6px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.ls-file-item {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-body-sm);
  color: var(--color-text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.ls-file-icon {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--color-text-faint);
  flex-shrink: 0;
}

/* Hidden on desktop; appears above footer on mobile/tablet */
.mobile-seek-wrap { display: none; }
.mobile-nav-overlay { display: none; }

/* ── Responsive: tablets and small screens ── */
@media (max-width: 768px) {
  .app {
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: 1fr calc(130px + env(safe-area-inset-bottom));
  }

  /* Dark overlay */
  .mobile-nav-overlay {
    display: block;
    position: fixed;
    inset: 0;
    background: color-mix(in srgb, var(--color-shadow) 60%, transparent);
    z-index: 399;
  }

  /* Main spans the full grid now that sidebar is out of flow */
  .main { grid-row: 1 / 2; grid-column: 1; }

  .content { padding: 0 14px 24px; }

  .card-list {
    grid-template-columns: repeat(auto-fill, minmax(130px, 1fr));
    gap: 14px;
  }
  .cover { aspect-ratio: 1; height: auto; }

  .library-header { flex-direction: column; gap: 10px; align-items: stretch; }
  .library-search { width: 100%; }

  .track-row { gap: 8px; padding: 6px 4px; }
  .track-cover-sm { width: 32px; height: 32px; }
  .discovery-toolbar { justify-content: flex-start; }
  .discovery-summary { margin-bottom: 14px; }
  .discovery-row { align-items: center; }
  .discovery-sync-btn { margin-left: 0; }
  .soulseek-row { align-items: center; }
  .soulseek-side { min-width: 0; gap: 8px; }
  .soulseek-speed { display: none; }
  .soulseek-download-btn { padding: 6px 12px; min-width: 92px; }
  .soulseek-progress-copy { white-space: normal; }
  .track-inline-action { display: none !important; }

  .icon-btn { padding: 6px; }

  .mobile-seek-wrap {
    display: block;
    position: fixed;
    left: 0;
    right: 0;
    bottom: calc(125px + env(safe-area-inset-bottom));
    z-index: 220;
    padding: 0 10px;
    pointer-events: none;
  }
  .mobile-seek-hit {
    pointer-events: auto;
    padding: 4px 0;
  }
  .mobile-seek-track {
    height: 4px;
    border-radius: 999px;
    background: var(--color-surface-hover);
    overflow: hidden;
    transition: height .15s ease;
  }
  .mobile-seek-wrap.active .mobile-seek-track {
    height: 10px;
  }
  .mobile-seek-fill {
    height: 100%;
    background: var(--color-text);
    position: relative;
  }
  .mobile-seek-thumb {
    position: absolute;
    right: 0;
    top: 50%;
    transform: translate(50%, -50%);
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--color-text);
    opacity: 0;
    transition: opacity .15s ease;
  }
  .mobile-seek-wrap.active .mobile-seek-thumb {
    opacity: 1;
  }

  .activity-dock {
    right: 14px;
    bottom: calc(130px + env(safe-area-inset-bottom));
    display: none;
    transform: none;
    z-index: 400;
  }
  .activity-dock.activity-active { display: block; transform: none; }
  .activity-mini {
    border-radius: 20px;
    border-bottom: 1px solid var(--color-surface-hover);
  }
  .status-pills { top: auto; bottom: calc(150px + env(safe-area-inset-bottom)); right: 14px; flex-direction: column; align-items: end; }

  .modal { width: 95vw; }
  .identify-modal { width: 95vw; }
}

/* ── Responsive: phones ── */
@media (max-width: 480px) {
  .app {
    --fs-nav: 14px;
    --fs-h2: 18px;
    --fs-section-link: 10px;
    --fs-card-title: 11px;
    --fs-card-meta: 10px;
    --fs-player-title: 11px;
    --fs-player-meta: 9px;
    --fs-input: 11px;
    --fs-empty: 12px;
    --fs-eyebrow: 10px;
    --fs-group: 11px;
    --fs-track-side: 11px;
    --fs-track-title: 12px;
    --fs-track-meta: 10px;
    --fs-badge: 9px;
    --fs-modal-title: 15px;
    --fs-field-label: 10px;
    --fs-field-input: 12px;
    --fs-button: 11px;
    --fs-dropdown-label: 10px;
    --fs-dropdown-item: 12px;
    --fs-queue-title: 11px;
    --fs-queue-meta: 9px;
    --fs-queue-dur: 10px;
    --fs-status-pill: 11px;
    --fs-powered-by: 9px;
    --fs-body-sm: 10px;
    --fs-body-md: 11px;
    --fs-peer-title: 12px;
    --fs-peer-meta: 10px;
    --fs-control: 10px;
    --fs-detail-title: 16px;
    --fs-detail-artist: 12px;
    --fs-detail-time: 10px;
  }

  .card-list { grid-template-columns: repeat(auto-fill, minmax(110px, 1fr)); gap: 10px; }

  .track-title { font-size: var(--fs-track-title); }
  .track-album { display: none; }
  .discovery-row .track-title-row {
    flex-wrap: wrap;
  }
  .discovery-row .track-album {
    display: block;
    white-space: normal;
  }
  .soulseek-row .track-album {
    display: block;
    white-space: normal;
  }
  .track-dur { display: none; }

  .mobile-seek-wrap {
    display: block;
    position: fixed;
    left: 0;
    right: 0;
    bottom: calc(120px + env(safe-area-inset-bottom));
    z-index: 220;
    padding: 0 10px;
    pointer-events: none;
  }
}

/* ── Player detail panel ───────────────────────────────────────────────── */
.player-detail {
  position: fixed;
  inset: 0;
  z-index: 300;
  display: flex;
  align-items: flex-end;
  justify-content: center;
  background: color-mix(in srgb, var(--color-shadow) 45%, transparent);
  backdrop-filter: blur(4px);
  overflow: hidden;
}

.detail-backdrop {
  position: absolute;
  inset: 0;
  pointer-events: none;
}

.detail-backdrop-image,
.detail-backdrop-wash {
  position: absolute;
  inset: -8%;
}

.detail-backdrop-image {
  filter: blur(20px) saturate(1.15);
  opacity: 0.3;
  transform: scale(1.1);
}

.detail-backdrop-wash {
  opacity: 0.78;
}

.detail-sheet {
  position: relative;
  width: 100%;
  max-width: 480px;
  max-height: 95vh;
  overflow: visible;
  background: linear-gradient(180deg, color-mix(in srgb, var(--color-bg-elevated) 80%, transparent), color-mix(in srgb, var(--color-bg-elevated) 94%, transparent));
  backdrop-filter: blur(10px) saturate(1.05);
  border-radius: 20px 20px 0 0;
  border: 1px solid color-mix(in srgb, var(--color-text) 8%, transparent);
  box-shadow: 0 -10px 60px color-mix(in srgb, var(--color-shadow) 38%, transparent);
  padding: 0 24px 24px;
  padding-bottom: calc(24px + env(safe-area-inset-bottom));
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 14px;
}

.detail-handle {
  width: 36px; height: 4px;
  border-radius: 999px;
  background: var(--color-surface-hover);
  margin: 12px auto 0;
  cursor: pointer;
  flex-shrink: 0;
}

.detail-cover-wrap {
  position: relative;
  z-index: 200;
  aspect-ratio: 1 / 1;
  flex-shrink: 0;
  width: min(28vh, 260px, 80vw);
  height: auto;
  align-self: center;
  perspective: 600px;
  cursor: grab;
  user-select: none;
  overflow: visible;
  isolation: isolate;
}

.detail-aura-field {
  position: absolute;
  inset: -18%;
  border-radius: 50%;
  pointer-events: none;
  z-index: 0;
}

.detail-art-stage {
  position: relative;
  width: 100%;
  height: 100%;
  transform-style: preserve-3d;
  will-change: transform;
}

.detail-spectrum-ring {
  position: absolute;
  inset: -18%;
  pointer-events: none;
  z-index: 1;
  --ring-radius: clamp(86px, 43%, 124px);
}

.detail-spectrum-spoke {
  position: absolute;
  inset: 0;
  transform-origin: center center;
}

.detail-spectrum-bar {
  display: block;
  position: absolute;
  left: 50%;
  top: 50%;
  width: var(--bar-width);
  height: var(--bar-height);
  border-radius: 999px;
  transform: translate(-50%, calc(-100% - var(--ring-radius) - var(--bar-lift))) scaleY(var(--bar-scale));
  transform-origin: center bottom;
  opacity: 0.12;
  will-change: transform, opacity, filter;
  transition: transform 72ms linear, opacity 72ms linear, box-shadow 120ms ease, filter 120ms ease;
}

.detail-vinyl {
  position: absolute;
  left: 62%;
  top: 50%;
  width: 88%;
  aspect-ratio: 1;
  border-radius: 50%;
  background:
    radial-gradient(circle at center, rgba(18,18,22,1) 0 10%, rgba(210,210,210,0.22) 10.5% 11.5%, rgba(20,20,24,1) 12% 18%, transparent 18.5%),
    repeating-radial-gradient(circle at center, rgba(255,255,255,0.055) 0 1px, rgba(16,16,20,0.9) 1px 5px, rgba(8,8,10,0.98) 5px 8px),
    radial-gradient(circle at 35% 30%, rgba(255,255,255,0.12), transparent 38%),
    radial-gradient(circle at center, #181a20 0%, #07080b 72%);
  border: 1px solid rgba(255,255,255,0.08);
  box-shadow: 24px 18px 42px color-mix(in srgb, var(--color-shadow) 40%, transparent);
  overflow: hidden;
  z-index: 0;
  will-change: transform;
}

.detail-vinyl::before {
  content: '';
  position: absolute;
  inset: 7%;
  border-radius: 50%;
  border: 1px solid rgba(255,255,255,0.04);
}

.detail-vinyl::after {
  content: '';
  position: absolute;
  left: 50%;
  top: 50%;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  transform: translate(-50%, -50%);
  background: radial-gradient(circle at 35% 35%, #ddd 0%, #9fa4af 30%, #111 32%, #050608 100%);
  box-shadow: 0 0 0 5px rgba(245,245,245,0.08);
}

.detail-vinyl-sheen {
  position: absolute;
  inset: 0;
  border-radius: 50%;
  background: linear-gradient(130deg, rgba(255,255,255,0.14), transparent 26%, transparent 58%, rgba(255,255,255,0.06) 72%, transparent 84%);
  mix-blend-mode: screen;
  pointer-events: none;
}

.detail-cover {
  width: 100%;
  height: 100%;
  border-radius: 12px;
  object-fit: cover;
  position: relative;
  overflow: hidden;
  transform-style: preserve-3d;
  will-change: transform;
  z-index: 2;
}

.detail-cover-3d {
  transform-origin: center center;
  transform: translateZ(46px);
  border: 1px solid color-mix(in srgb, var(--color-text) 8%, transparent);
}

.card-gloss {
  position: absolute;
  inset: 0;
  border-radius: 12px;
  pointer-events: none;
  mix-blend-mode: screen;
  z-index: 1;
}

.card-rainbow {
  position: absolute;
  inset: 0;
  border-radius: 12px;
  pointer-events: none;
  mix-blend-mode: screen;
  z-index: 2;
}

.detail-info {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  min-width: 0;
}

.detail-track-name {
  font-size: var(--fs-detail-title); font-weight: 700; color: var(--color-text);
  overflow: hidden;
  flex: 1;
  min-width: 0;
  mask-image: linear-gradient(to right, transparent 0%, black 5%, black 85%, transparent 100%);
  -webkit-mask-image: linear-gradient(to right, transparent 0%, black 5%, black 85%, transparent 100%);
}

.marquee-text {
  display: inline-block;
  white-space: nowrap;
  animation: marquee 14s linear 2s infinite;
}

@keyframes marquee {
  0%   { transform: translateX(0); }
  15%  { transform: translateX(0); }
  85%  { transform: translateX(-50%); }
  100% { transform: translateX(-50%); }
}


.detail-track-artist {
  font-size: var(--fs-detail-artist); color: var(--color-text-muted);
  flex: 1;
  overflow: hidden;
  min-width: 0;
  mask-image: linear-gradient(to right, transparent 0%, black 5%, black 85%, transparent 100%);
  -webkit-mask-image: linear-gradient(to right, transparent 0%, black 5%, black 85%, transparent 100%);
}

.detail-seek-wrap { display: flex; flex-direction: column; gap: 6px; width: 100%; }

.detail-bar {
  height: 28px; display: flex; align-items: center; cursor: pointer;
}

.detail-seek-wrap { position: relative; }

.detail-bar {
  position: relative;
  height: 28px;
}

.detail-bar-track {
  position: relative;
  width: 100%;
  height: 4px;
  background: var(--color-surface-hover);
  border-radius: 999px;
  pointer-events: none;
}

.detail-bar-fill {
  position: absolute;
  inset: 0 auto 0 0;
  height: 4px;
  background: var(--color-text);
  border-radius: 999px;
  pointer-events: none;
}

.detail-bar-buffer {
  position: absolute;
  left: 0;
  top: 50%;
  height: 2px;
  transform: translateY(-50%);
  background: color-mix(in srgb, var(--color-text) 20%, transparent);
  border-radius: 999px;
  pointer-events: none;
}

.detail-bar-thumb {
  position: absolute;
  right: -6px; top: 50%;
  transform: translateY(-50%);
  width: 12px; height: 12px;
  border-radius: 50%;
  background: var(--color-text);
}

.detail-bar-track {
  flex-shrink: 0;
}

.detail-time-row {
  display: flex;
  justify-content: space-between;
  font-size: var(--fs-detail-time);
  color: var(--color-text-muted);
}

.detail-controls {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
}

.detail-play-btn {
  width: 60px; height: 60px;
  border-radius: 50%;
  background: var(--color-text);
  border: none;
  display: flex; align-items: center; justify-content: center;
  cursor: pointer;
  color: var(--color-bg);
}

.detail-play-btn:hover { background: color-mix(in srgb, var(--color-text) 88%, var(--color-bg)); }

/* slide-up transition */
.detail-enter-active, .detail-leave-active {
  transition: opacity .25s ease;
}
.detail-enter-from, .detail-leave-to {
  opacity: 0;
}
.detail-enter-active .detail-sheet,
.detail-leave-active .detail-sheet {
  transition: transform .3s cubic-bezier(.4,0,.2,1);
}
.detail-enter-from .detail-sheet,
.detail-leave-to .detail-sheet {
  transform: translateY(100%);
}

/* ── Desktop override: centered dialog ──────────────────────────────────── */
@media (max-width: 768px) {
  .player-detail {
    background: color-mix(in srgb, var(--color-shadow) 72%, transparent);
    backdrop-filter: none;
  }

  .detail-backdrop {
    display: none;
  }

  .detail-sheet {
    backdrop-filter: none;
  }

  .detail-spectrum-ring {
    --ring-radius: clamp(70px, 38%, 100px);
  }

  .detail-backdrop-image {
    filter: blur(14px) saturate(1.08);
    opacity: 0.24;
  }
}

@media (min-width: 769px) {
  .player-detail {
    align-items: center;
  }

  .detail-sheet {
    max-width: 680px;
    max-height: 80vh;
    border-radius: 16px;
    padding: 32px;
    display: grid;
    grid-template-columns: 240px 1fr;
    grid-template-rows: auto auto auto auto;
    column-gap: 32px;
    row-gap: 20px;
    align-items: start;
  }

  /* cover spans all rows on the left */
  .detail-cover-wrap {
    grid-column: 1;
    grid-row: 1 / 5;
    max-height: none;
    width: 100%;
    aspect-ratio: 1;
    align-self: center;
  }

  .detail-backdrop-image {
    filter: blur(32px) saturate(1.22);
    opacity: 0.38;
  }

  .detail-cover {
    max-height: none;
  }

  /* handle hidden on desktop — click backdrop to close */
  .detail-handle { display: none; }

  .detail-info    { grid-column: 2; grid-row: 1; flex-direction: column; align-items: flex-start; gap: 4px; overflow: hidden;}
  .detail-seek-wrap { grid-column: 2; grid-row: 2; }
  .detail-controls  { grid-column: 2; grid-row: 3; }

  .detail-track-name  { font-size: 22px; }
  .detail-track-artist { font-size: 15px; }

  .detail-play-btn { width: 52px; height: 52px; }

  /* scale-in transition for desktop instead of slide-up */
  .detail-enter-from .detail-sheet,
  .detail-leave-to .detail-sheet {
    transform: scale(0.92);
  }
}
</style>
