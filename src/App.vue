<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import WebGLAlbumRenderer from "./components/WebGLAlbumRenderer.vue";

interface AudioDevice { name: string }
interface DeviceList { devices: AudioDevice[]; current: string | null }

interface SoulseekStatus {
  enabled: boolean;
  configured: boolean;
  username: string | null;
  activeSession: boolean;
}

interface SoulseekSearchResult {
  username: string;
  filename: string;
  basename: string;
  coverFilename: string | null;
  coverSize: number | null;
  size: number;
  bitrate: number | null;
  duration: number | null;
  sampleRate: number | null;
  bitDepth: number | null;
  vbr: boolean | null;
  peerSpeed: number;
  freeUploadSlots: number;
  extension: string | null;
}

interface SoulseekDownloadEvent {
  transferId: string;
  username: string;
  filename: string;
  basename: string;
  state: string;
  bytesDownloaded: number | null;
  totalBytes: number | null;
  speedBytesPerSec: number | null;
  queuePosition: number | null;
  localPath: string | null;
  error: string | null;
}

interface Track {
  id: number;
  path: string;
  title: string | null;
  artist: string | null;
  album: string | null;
  track_number: number | null;
  duration_secs: number | null;
  file_hash: string | null;
  rarity: string | null;
  manually_edited: boolean;
  is_liked: boolean;
  play_count: number;
  year: number | null;
  genre: string | null;
  tags: string | null;
  date_added: number | null;
  is_duplicate: boolean;
  local_preview_path?: string | null;
  preview_growing?: boolean;
}

const rarityColors: Record<string, string> = {
  Common: '#b0b0b0',
  Uncommon: '#1db954',
  Rare: '#4fc3f7',
  Epic: '#ba68c8',
  Legendary: '#ffa726',
  Mythic: '#ff5252',
};
const animatedRarities = new Set(['Epic', 'Legendary', 'Mythic']);
const TRACK_RARITY_OPTIONS = ['Common', 'Uncommon', 'Rare', 'Epic', 'Legendary', 'Mythic'] as const;

function rarityClass(r: string | null) {
  if (!r || r === 'Common') return '';
  if (animatedRarities.has(r)) return `rarity-animated rarity-${r.toLowerCase()}`;
  return 'rarity-tint';
}
function rarityVars(r: string | null): Record<string, string> {
  if (!r || r === 'Common') return {};
  const c = rarityColors[r] || '#b0b0b0';
  return { '--rc': c };
}

/** Derive two complementary HSL colors from a hex hash string (or fallback). */
function hashToColors(hash: string | null): [string, string] {
  if (!hash || hash.length < 8) return ['#1a1a2e', '#16213e'];
  // Take two 3-byte windows from the hash as seeds
  const a = parseInt(hash.slice(0, 6), 16);
  const b = parseInt(hash.slice(6, 12), 16);
  const hueA = a % 360;
  const hueB = (hueA + 137) % 360;  // golden angle offset for contrast
  const satA = 45 + (a >> 16 & 0x1f); // 45-76%
  const satB = 45 + (b >> 16 & 0x1f);
  const litA = 20 + (a >> 8 & 0x0f);  // 20-35%
  const litB = 15 + (b >> 8 & 0x0f);  // 15-30%
  return [
    `hsl(${hueA}, ${satA}%, ${litA}%)`,
    `hsl(${hueB}, ${satB}%, ${litB}%)`,
  ];
}

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
const isLiked = ref(true);
const showUserMenu = ref(false);
const showDeviceMenu = ref(false);
const showMobileNav = ref(false);
const outputDevices = ref<AudioDevice[]>([]);
const currentDevice = ref<string | null>(null);
const deviceMenuError = ref('');
const activeNav = ref("home");
const libraryTracks = ref<Track[]>([]);
const showDuplicateTracks = ref(false);
const libraryLoading = ref(false);
const libraryQuery = ref("");
const searchQuery = ref("");
const editingTrack = ref<Track | null>(null);
const editForm = ref({
  title: '',
  artist: '',
  album: '',
  track_number: null as number | null,
  year: null as number | null,
  genre: '',
  tags: '',
  play_count: 0,
  is_liked: false,
  date_added: '',
  rarity: '',
});
const covers = ref<Record<number, string | null>>({});

function filterDuplicateTracks(tracks: Track[]) {
  return showDuplicateTracks.value ? tracks : tracks.filter((track) => !track.is_duplicate);
}

function trackTagsList(track: Pick<Track, 'tags'>): string[] {
  if (!track.tags) return [];
  const seen = new Set<string>();
  const tags: string[] = [];
  for (const raw of track.tags.split(/[\n,;]+/)) {
    const tag = raw.trim();
    const key = tag.toLowerCase();
    if (!tag || seen.has(key)) continue;
    seen.add(key);
    tags.push(tag);
  }
  return tags;
}

function trackTagsText(track: Pick<Track, 'tags'>) {
  return trackTagsList(track).join(', ');
}

function normalizeTrackTagsInput(value: string): string | null {
  const tags = trackTagsList({ tags: value });
  return tags.length ? tags.join(', ') : null;
}

function trackDateInputValue(value: number | null) {
  if (value == null) return '';
  const date = new Date(value * 1000);
  return Number.isNaN(date.getTime()) ? '' : date.toISOString().slice(0, 10);
}

function parseTrackDateInput(value: string): number | null {
  if (!value) return null;
  const parsed = Date.parse(`${value}T00:00:00Z`);
  return Number.isNaN(parsed) ? null : Math.floor(parsed / 1000);
}

function normalizeOptionalInteger(value: unknown): number | null {
  if (value == null || value === '') return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? Math.trunc(parsed) : null;
}

function normalizeNonNegativeInteger(value: unknown): number {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) return 0;
  return Math.max(0, Math.trunc(parsed));
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
  // shared
  startedAt: number;
  finishedAt?: number;
  status: 'running' | 'done' | 'error';
  errorMsg?: string;
}
const indexLog = ref<LogSession[]>([]);
const indexLogRef = ref<HTMLElement | null>(null);
const indexLogOpen = ref(false);

/* ── Queue state ── */
type QueueSource = 'recent' | 'library' | 'playlist' | 'soulseek';
const queueSource = ref<QueueSource>('library');
const queueSourceIndex = ref(0);       // index into source list of the LAST item pushed
const queuePlaylistTracks = ref<Track[]>([]); // tracks for 'playlist' source
const queueSoulseekTracks = ref<Track[]>([]);
const queue = ref<Track[]>([]);         // upcoming tracks (max 5 visible)
const nowPlaying = ref<Track | null>(null);
const recentTracks = ref<Track[]>([]);
const showQueueMenu = ref(false);

interface PlayHistoryEntry {
  played_at: number; // unix timestamp (seconds)
  track: Track;
}
interface AboutChangelogEntry {
  subject: string;
  short_hash: string;
  committed_at: string;
}

interface AboutInfo {
  current_version: string;
  build_commit: string | null;
  release_repo: string | null;
  changelog: AboutChangelogEntry[];
}

interface AboutUpdateStatus {
  current_version: string;
  latest_version: string | null;
  has_update: boolean;
  release_url: string | null;
  checked_repo: string | null;
  message: string;
}

const historyEntries = ref<PlayHistoryEntry[]>([]);
const historyLoading = ref(false);
const aboutInfo = ref<AboutInfo | null>(null);
const aboutLoading = ref(false);
const aboutError = ref('');
const aboutUpdateStatus = ref<AboutUpdateStatus | null>(null);
const aboutCheckingUpdates = ref(false);
const aboutUpdateError = ref('');

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
const playlistView = ref<{ id: number; name: string; tracks: Track[] } | null>(null);
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
  const tracks = filterDuplicateTracks(await invoke<Track[]>('get_playlist_tracks', { playlistId: pl.id }));
  playlistView.value = { id: pl.id, name: pl.name, tracks };
}

async function addTrackToPlaylist(playlistId: number, trackId: number) {
  await invoke('add_track_to_playlist', { playlistId, trackId });
  await loadPlaylists();
  if (playlistView.value?.id === playlistId) {
    const tracks = filterDuplicateTracks(await invoke<Track[]>('get_playlist_tracks', { playlistId }));
    playlistView.value = { ...playlistView.value, tracks };
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

function openAddToPlaylistMenu(e: MouseEvent, track: Track) {
  e.stopPropagation();
  openAddToPlaylistMenuAt(track, e.clientX, e.clientY);
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
  playTrackFrom('library', index);
}

function playPlaylistTrack(tracks: Track[], index: number) {
  if (index < 0 || shouldSuppressTrackRowClick()) return;
  playFromPlaylist(tracks, index);
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
type SPField = 'any' | 'title' | 'artist' | 'album' | 'genre' | 'tags' | 'rarity' | 'path' | 'extension' | 'track_number' | 'duration_secs' | 'year' | 'play_count' | 'is_liked' | 'date_added' | 'sort';
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
  { value: 'genre', label: 'Genre' },
  { value: 'tags', label: 'Tags' },
  { value: 'rarity', label: 'Rarity' },
  { value: 'path', label: 'Path' },
  { value: 'extension', label: 'Extension' },
  { value: 'track_number', label: 'Track #' },
  { value: 'duration_secs', label: 'Duration (sec)' },
  { value: 'year', label: 'Year' },
  { value: 'play_count', label: 'Play count' },
  { value: 'is_liked', label: 'Is liked' },
  { value: 'date_added', label: 'Date added' },
];

const SP_SORT_FIELD_SET = new Set<SPSortField>(SP_SORT_FIELD_OPTIONS.map((option) => option.value));
const smartPlaylistSortCollator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

const smartPlaylists = ref<SmartPlaylist[]>([]);
const smartView = ref<SmartPlaylist | null>(null);
const editingSP = ref<SmartPlaylist | null>(null);
const showNewSPInput = ref(false);
const newSPName = ref('');
const playlistTab = ref<'regular' | 'smart'>('regular');

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
  });
}
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
  const f = rule.field;
  if (f === 'sort') { rule.op = 'sort_asc'; rule.value = 'title'; }
  else if (f === 'is_liked') { rule.op = 'is_true'; rule.value = ''; }
  else if (f === 'year' || f === 'play_count' || f === 'track_number' || f === 'duration_secs') { rule.op = 'gte'; rule.value = '0'; }
  else if (f === 'date_added') { rule.op = 'gte'; rule.value = new Date().toISOString().slice(0, 10); }
  else if (f === 'genre' || f === 'tags' || f === 'rarity' || f === 'extension' || f === 'artist' || f === 'album') { rule.op = 'in'; rule.value = '[]'; }
  else { rule.op = 'contains'; rule.value = ''; }
  saveSP();
}
function spFieldType(field: SPField): 'text' | 'multiselect' | 'number' | 'bool' | 'date' | 'sort' {
  if (field === 'sort') return 'sort';
  if (field === 'is_liked') return 'bool';
  if (field === 'year' || field === 'play_count' || field === 'track_number' || field === 'duration_secs') return 'number';
  if (field === 'date_added') return 'date';
  if (field === 'genre' || field === 'tags' || field === 'rarity' || field === 'extension' || field === 'artist' || field === 'album') return 'multiselect';
  return 'text';
}
function spUniqueValues(field: SPField): string[] {
  const set = new Set<string>();
  for (const t of libraryTracks.value) {
    if (field === 'genre' && t.genre) set.add(t.genre);
    else if (field === 'tags') for (const tag of trackTagsList(t)) set.add(tag);
    else if (field === 'rarity' && t.rarity) set.add(t.rarity);
    else if (field === 'extension') { const e = t.path.split('.').pop()?.toLowerCase(); if (e) set.add(e); }
    else if (field === 'artist' && t.artist) set.add(t.artist);
    else if (field === 'album' && t.album) set.add(t.album);
  }
  return [...set].sort((left, right) => smartPlaylistSortCollator.compare(left, right));
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

function smartPlaylistSortRules(sp: SmartPlaylist): Array<{ field: SPSortField; op: SPSortOp }> {
  return sp.rules.flatMap((rule) => {
    if (rule.field !== 'sort') return [];
    return [{
      field: normalizeSPSortField(rule.value),
      op: normalizeSPSortOp(rule.op),
    }];
  });
}

function compareOptionalText(left: string | null | undefined, right: string | null | undefined, descending = false) {
  const leftText = left?.trim() ?? '';
  const rightText = right?.trim() ?? '';
  if (!leftText && !rightText) return 0;
  if (!leftText) return 1;
  if (!rightText) return -1;
  const comparison = smartPlaylistSortCollator.compare(leftText, rightText);
  return descending ? -comparison : comparison;
}

function compareOptionalNumber(left: number | null | undefined, right: number | null | undefined, descending = false) {
  if (left == null && right == null) return 0;
  if (left == null) return 1;
  if (right == null) return -1;
  const comparison = left - right;
  return descending ? -comparison : comparison;
}

function trackSortExtension(track: Track) {
  return track.path.split('.').pop()?.toLowerCase() || '';
}

function compareTracksForSortField(left: Track, right: Track, field: SPSortField, descending = false) {
  switch (field) {
    case 'title':
      return compareOptionalText(left.title || left.path, right.title || right.path, descending);
    case 'artist':
      return compareOptionalText(left.artist, right.artist, descending);
    case 'album':
      return compareOptionalText(left.album, right.album, descending);
    case 'genre':
      return compareOptionalText(left.genre, right.genre, descending);
    case 'tags':
      return compareOptionalText(trackTagsText(left), trackTagsText(right), descending);
    case 'rarity':
      return compareOptionalText(left.rarity, right.rarity, descending);
    case 'path':
      return compareOptionalText(left.path, right.path, descending);
    case 'extension':
      return compareOptionalText(trackSortExtension(left), trackSortExtension(right), descending);
    case 'track_number':
      return compareOptionalNumber(left.track_number, right.track_number, descending);
    case 'duration_secs':
      return compareOptionalNumber(left.duration_secs, right.duration_secs, descending);
    case 'year':
      return compareOptionalNumber(left.year, right.year, descending);
    case 'play_count':
      return compareOptionalNumber(left.play_count, right.play_count, descending);
    case 'is_liked':
      return compareOptionalNumber(left.is_liked ? 1 : 0, right.is_liked ? 1 : 0, descending);
    case 'date_added':
      return compareOptionalNumber(left.date_added, right.date_added, descending);
  }
}

function matchesRule(track: Track, rule: SPRule): boolean {
  switch (rule.field) {
    case 'any': {
      const q = rule.value.toLowerCase();
      const values = [
        track.title,
        track.artist,
        track.album,
        track.genre,
        trackTagsText(track),
        track.rarity,
        track.path,
        track.track_number != null ? String(track.track_number) : null,
        track.duration_secs != null ? String(track.duration_secs) : null,
        track.year != null ? String(track.year) : null,
        String(track.play_count),
      ];
      return !q || values.some((value) => value?.toLowerCase().includes(q));
    }
    case 'title': return !rule.value || !!track.title?.toLowerCase().includes(rule.value.toLowerCase());
    case 'artist': {
      if (rule.op === 'in') { const s: string[] = JSON.parse(rule.value || '[]'); return s.length === 0 || s.includes(track.artist ?? ''); }
      return !rule.value || !!track.artist?.toLowerCase().includes(rule.value.toLowerCase());
    }
    case 'album': {
      if (rule.op === 'in') { const s: string[] = JSON.parse(rule.value || '[]'); return s.length === 0 || s.includes(track.album ?? ''); }
      return !rule.value || !!track.album?.toLowerCase().includes(rule.value.toLowerCase());
    }
    case 'genre': {
      if (rule.op === 'in') { const s: string[] = JSON.parse(rule.value || '[]'); return s.length === 0 || s.includes(track.genre ?? ''); }
      return !rule.value || !!track.genre?.toLowerCase().includes(rule.value.toLowerCase());
    }
    case 'tags': {
      const tags = trackTagsList(track);
      if (rule.op === 'in') {
        const s: string[] = JSON.parse(rule.value || '[]');
        return s.length === 0 || s.some((tag) => tags.includes(tag));
      }
      return !rule.value || trackTagsText(track).toLowerCase().includes(rule.value.toLowerCase());
    }
    case 'rarity': {
      if (rule.op === 'in') { const s: string[] = JSON.parse(rule.value || '[]'); return s.length === 0 || s.includes(track.rarity ?? ''); }
      return !rule.value || !!track.rarity?.toLowerCase().includes(rule.value.toLowerCase());
    }
    case 'path':
      return !rule.value || track.path.toLowerCase().includes(rule.value.toLowerCase());
    case 'extension': {
      const ext = track.path.split('.').pop()?.toLowerCase() || '';
      if (rule.op === 'in') { const s: string[] = JSON.parse(rule.value || '[]'); return s.length === 0 || s.includes(ext); }
      return ext.includes(rule.value.toLowerCase());
    }
    case 'track_number': {
      if (track.track_number == null) return false;
      const n = Number(rule.value);
      if (rule.op === 'eq') return track.track_number === n;
      if (rule.op === 'gte') return track.track_number >= n;
      if (rule.op === 'lte') return track.track_number <= n;
      return false;
    }
    case 'duration_secs': {
      if (track.duration_secs == null) return false;
      const n = Number(rule.value);
      if (rule.op === 'eq') return track.duration_secs === n;
      if (rule.op === 'gte') return track.duration_secs >= n;
      if (rule.op === 'lte') return track.duration_secs <= n;
      return false;
    }
    case 'year': {
      if (track.year == null) return false;
      const n = Number(rule.value);
      if (rule.op === 'eq') return track.year === n;
      if (rule.op === 'gte') return track.year >= n;
      if (rule.op === 'lte') return track.year <= n;
      return false;
    }
    case 'play_count': {
      const n = Number(rule.value);
      if (rule.op === 'eq') return track.play_count === n;
      if (rule.op === 'gte') return track.play_count >= n;
      if (rule.op === 'lte') return track.play_count <= n;
      return false;
    }
    case 'is_liked': return rule.op === 'is_true' ? track.is_liked : !track.is_liked;
    case 'date_added': {
      if (track.date_added == null) return false;
      if (!rule.value) return true;
      const dayStart = new Date(rule.value).getTime() / 1000;
      if (rule.op === 'gte') return track.date_added >= dayStart;
      if (rule.op === 'lte') return track.date_added < dayStart + 86400;
      if (rule.op === 'eq') return track.date_added >= dayStart && track.date_added < dayStart + 86400;
      return false;
    }
    case 'sort': return true;
    default: return true;
  }
}
function smartPlaylistTracks(sp: SmartPlaylist): Track[] {
  if (sp.rules.length === 0) return [];
  const filterRules = sp.rules.filter((rule) => rule.field !== 'sort');
  const sortRules = smartPlaylistSortRules(sp);
  const baseTracks = filterDuplicateTracks(libraryTracks.value);
  const filteredTracks = filterRules.length === 0
    ? baseTracks.slice()
    : baseTracks.filter((track) =>
      sp.match === 'all'
        ? filterRules.every((rule) => matchesRule(track, rule))
        : filterRules.some((rule) => matchesRule(track, rule))
    );

  if (sortRules.length === 0) {
    return filteredTracks;
  }

  return filteredTracks.sort((left, right) => {
    for (const rule of sortRules) {
      const comparison = compareTracksForSortField(left, right, rule.field, rule.op === 'sort_desc');
      if (comparison !== 0) {
        return comparison;
      }
    }

    return smartPlaylistSortCollator.compare(left.path, right.path);
  });
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
      const trackCount = smartPlaylistTracks(sp).length;
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
      : smartPlaylistTracks(item.playlist as SmartPlaylist);

    return {
      ...item,
      tracks,
      previewTracks: tracks.slice(0, HOME_PLAYLIST_PREVIEW_LIMIT),
    };
  });
});

function homePinnedCoverStyle(item: HomePinnedPlaylistItem) {
  const [left, right] = hashToColors(item.key);
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

  const tracks = smartPlaylistTracks(item.playlist as SmartPlaylist);
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

  if (playlistView.value) {
    const tracks = filterDuplicateTracks(await invoke<Track[]>('get_playlist_tracks', { playlistId: playlistView.value.id }));
    playlistView.value = { ...playlistView.value, tracks };
  }
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

interface SyncProgress {
  peer: string;
  device_name?: string | null;
  device_emoji?: string | null;
  phase: string;
  total: number;
  done: number;
  message?: string;
}
const syncEnabled = ref(false);
const syncProgress = ref<Record<string, SyncProgress>>({});
const peerDeviceNames = ref<Record<string, string>>({});
const deviceEmoji = ref('🎵');
const EMOJI_OPTIONS = ['🎵', '🎶', '🎤', '🎧', '🎼', '🎹', '🎸', '🥁', '📱', '💻', '🖥️', '⌚', '📻', '📡', '🔊', '🎺', '🎻', '🪕', '🎷', '🍕'];
const settingsOpen = ref(false);
const soulseekSettingsOpen = ref(false);
const settingsEmoji = ref('🎵');
const settingsDeviceName = ref('');
const settingsSoulseekEnabled = ref(false);
const settingsSoulseekUsername = ref('');
const settingsSoulseekPassword = ref('');
const settingsSaving = ref(false);
const settingsError = ref('');
const soulseekSettingsSaving = ref(false);
const soulseekSettingsError = ref('');
const soulseekStatus = ref<SoulseekStatus | null>(null);
const soulseekResults = ref<SoulseekSearchResult[]>([]);
const soulseekPreviews = ref<Record<string, SoulseekDownloadEvent>>({});
const soulseekVisibleCount = ref(0);
const soulseekSubmittedQuery = ref('');
const soulseekLoading = ref(false);
const soulseekSearching = ref(false);
const soulseekError = ref('');
const soulseekDownloads = ref<Record<string, SoulseekDownloadEvent>>({});
const soulseekCoverUrls = ref<Record<string, string | null>>({});
const libraryDataDir = ref<string | null>(null);
let soulseekSearchSeq = 0;
let unlistenSoulseekDownload: (() => void) | null = null;
let unlistenSoulseekPreview: (() => void) | null = null;
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

interface DeviceSettings {
  emoji: string;
  device_name: string;
  sync_enabled: boolean;
  soulseek_enabled: boolean;
  soulseek_username: string;
  soulseek_password: string;
}

async function fetchDeviceSettings() {
  return invoke<DeviceSettings>('get_device_settings');
}

async function openDeviceSettings() {
  settingsError.value = '';
  const cfg = await fetchDeviceSettings();
  settingsEmoji.value = cfg.emoji || '🎵';
  settingsDeviceName.value = cfg.device_name || '';
  settingsOpen.value = true;
}

async function openSoulseekSettings() {
  soulseekSettingsError.value = '';
  const cfg = await fetchDeviceSettings();
  settingsSoulseekEnabled.value = !!cfg.soulseek_enabled;
  settingsSoulseekUsername.value = cfg.soulseek_username || '';
  settingsSoulseekPassword.value = cfg.soulseek_password || '';
  soulseekSettingsOpen.value = true;
}

async function saveDeviceSettings() {
  settingsSaving.value = true;
  settingsError.value = '';
  try {
    const cfg = await fetchDeviceSettings();
    await invoke('set_device_settings', {
      emoji: settingsEmoji.value,
      deviceName: settingsDeviceName.value,
      soulseekEnabled: cfg.soulseek_enabled,
      soulseekUsername: cfg.soulseek_username,
      soulseekPassword: cfg.soulseek_password,
    });
    deviceEmoji.value = settingsEmoji.value;
    await loadSoulseekStatus();
    settingsOpen.value = false;
  } catch (e: any) {
    settingsError.value = String(e ?? 'Failed to save settings');
  } finally {
    settingsSaving.value = false;
  }
}

async function saveSoulseekSettings() {
  soulseekSettingsSaving.value = true;
  soulseekSettingsError.value = '';
  try {
    const cfg = await fetchDeviceSettings();
    await invoke('set_device_settings', {
      emoji: cfg.emoji,
      deviceName: cfg.device_name,
      soulseekEnabled: settingsSoulseekEnabled.value,
      soulseekUsername: settingsSoulseekUsername.value,
      soulseekPassword: settingsSoulseekPassword.value,
    });
    await loadSoulseekStatus();
    soulseekSettingsOpen.value = false;
  } catch (e: any) {
    soulseekSettingsError.value = String(e ?? 'Failed to save Soulseek settings');
  } finally {
    soulseekSettingsSaving.value = false;
  }
}

const soulseekReady = computed(() => !!soulseekStatus.value?.enabled && !!soulseekStatus.value?.configured);

function soulseekResultKey(username: string, filename: string) {
  return `${username}\u0000${filename}`;
}

function soulseekCoverKey(username: string, filename: string) {
  return soulseekResultKey(username, filename);
}

function soulseekCoverUrl(result: SoulseekSearchResult) {
  if (!result.coverFilename) return null;
  return soulseekCoverUrls.value[soulseekCoverKey(result.username, result.coverFilename)] || null;
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
  if (isCurrentTrack(soulseekTrackId(result)) && !!nowPlaying.value?.local_preview_path && isPlaying.value) {
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
  if (!currentTrack || currentTrack.id !== previewTrackId || !currentTrack.local_preview_path) {
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
  isLiked.value = false;
  stopTicker();
  syncAndroid();
  return true;
}

function updateSoulseekPreviewTrackState(username: string, filename: string, previewGrowing: boolean) {
  const previewTrackId = soulseekTrackIdFromParts(username, filename);
  const updateTrack = (track: Track) => (
    track.id === previewTrackId && track.local_preview_path
      ? { ...track, preview_growing: previewGrowing }
      : track
  );

  if (nowPlaying.value?.id === previewTrackId && nowPlaying.value.local_preview_path) {
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

function formatBytes(bytes: number | null | undefined) {
  if (bytes == null) return '—';
  const units = ['B', 'KB', 'MB', 'GB'];
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  const precision = unitIndex === 0 || value >= 100 ? 0 : 1;
  return `${value.toFixed(precision)} ${units[unitIndex]}`;
}

function formatTransferRate(bytesPerSecond: number | null | undefined) {
  if (bytesPerSecond == null) return '—';
  return `${formatBytes(bytesPerSecond)}/s`;
}

function formatSampleRate(sampleRate: number | null | undefined) {
  if (!sampleRate) return '';
  const khz = sampleRate / 1000;
  return `${Number.isInteger(khz) ? khz.toFixed(0) : khz.toFixed(1)} kHz`;
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

function normalizePath(path: string) {
  return path.replace(/\\/g, '/');
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
  };
}

function makeSoulseekPreviewTrack(result: SoulseekSearchResult, localPath: string, previewGrowing: boolean): Track {
  return {
    ...makeSoulseekTrack(result, `Soulseek Preview/${result.basename}`),
    local_preview_path: localPath,
    preview_growing: previewGrowing,
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
  if (!currentTrack || currentTrack.id !== previewTrackId || !currentTrack.local_preview_path) {
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
  isLiked.value = indexedTrack.is_liked;
  duration.value = indexedTrack.duration_secs || duration.value;
  if (covers.value[indexedTrack.id] === undefined) {
    invoke<string | null>('get_track_cover', { id: indexedTrack.id })
      .then((url) => {
        covers.value[indexedTrack.id] = url;
      })
      .catch(() => {});
  }
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
    if (soulseekCoverUrls.value[key] !== undefined || soulseekCoverLoading.has(key)) continue;
    uniqueResults.set(key, result);
  }

  for (const [key, result] of uniqueResults) {
    if (!isCurrentSoulseekSearchRequest(target, requestId)) return;

    soulseekCoverLoading.add(key);
    try {
      const url = await invoke<string | null>('soulseek_fetch_cover', {
        request: {
          username: result.username,
          coverFilename: result.coverFilename,
          coverSize: result.coverSize,
        },
      });
      soulseekCoverUrls.value = {
        ...soulseekCoverUrls.value,
        [key]: url,
      };
    } catch (error) {
      console.error('Failed to fetch Soulseek cover preview:', error);
      soulseekCoverUrls.value = {
        ...soulseekCoverUrls.value,
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
  if (activeNav.value !== 'search' && activeNav.value !== 'track-replace') {
    return;
  }

  const content = contentRef.value;
  if (!content) return;

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

  const distanceToBottom = content.scrollHeight - (content.scrollTop + content.clientHeight);
  if (distanceToBottom > SOULSEEK_SCROLL_LOAD_THRESHOLD) return;

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

async function refreshTrackCover(trackId: number) {
  try {
    const coverUrl = await invoke<string | null>('get_track_cover', { id: trackId });
    covers.value = {
      ...covers.value,
      [trackId]: coverUrl,
    };
  } catch (_) {}
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

    await refreshTrackCover(replacedTrack.id);

    const previewTrackId = soulseekTrackId(result);
    if (nowPlaying.value?.id === previewTrackId && !!nowPlaying.value.local_preview_path) {
      replaceSoulseekTrackInPlaybackState(previewTrackId, replacedTrack);
      isLiked.value = replacedTrack.is_liked;
      duration.value = replacedTrack.duration_secs || duration.value;
      syncAndroid();
    } else if (nowPlaying.value?.id === replacedTrack.id) {
      nowPlaying.value = { ...replacedTrack };
      isLiked.value = replacedTrack.is_liked;
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
    console.error('Failed to update sync setting:', error);
  }
}

function syncPeer(peer: Peer) {
  invoke('sync_with_peer', { peerHost: peer.host, peerName: peer.name, peerAddresses: peer.addresses, peerPort: peer.port }).catch(() => {});
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
      artist: nowPlaying.value.artist || 'Unknown',
      colors: hashToColors(nowPlaying.value.file_hash),
    };
  }
  return { title: 'No track', artist: '', colors: ['#282828', '#181818'] };
});

const beatIntensity = computed(() => {
  if (BEAT_AMP <= 0) return 0;
  return Math.max(0, Math.min(1.4, (beatScale.value - 1) / BEAT_AMP));
});

const currentCoverUrl = computed(() => {
  if (!nowPlaying.value) return null;
  return covers.value[nowPlaying.value.id] ?? null;
});

const currentRarityColor = computed(() => {
  const rarity = nowPlaying.value?.rarity;
  if (!rarity || rarity === 'Common') return '#d9dee8';
  return rarityColors[rarity] ?? '#d9dee8';
});

const detailBackdropImageStyle = computed(() => {
  if (currentCoverUrl.value) {
    return {
      backgroundImage: `url(${currentCoverUrl.value})`,
      backgroundSize: 'cover',
      backgroundPosition: 'center',
    };
  }
  return {
    background: `linear-gradient(135deg, ${currentTrack.value.colors[0]}, ${currentTrack.value.colors[1]})`,
  };
});

const detailBackdropWashStyle = computed(() => {
  const [colorA, colorB] = currentTrack.value.colors;
  const pulse = beatIntensity.value;
  return {
    background: `radial-gradient(circle at 18% 20%, ${colorA} 0%, transparent ${Math.max(28, 46 - pulse * 5)}%), radial-gradient(circle at 82% 18%, ${colorB} 0%, transparent ${Math.max(26, 44 - pulse * 4)}%), linear-gradient(180deg, rgba(4,6,12,0.22), rgba(4,6,12,0.74) 38%, rgba(4,6,12,0.95))`,
    opacity: (0.62 + pulse * 0.18).toFixed(2),
  };
});

const detailAuraStyle = computed(() => {
  const [colorA, colorB] = currentTrack.value.colors;
  const pulse = beatIntensity.value;
  return {
    background: `radial-gradient(circle at 34% 34%, ${colorA} 0%, transparent 48%), radial-gradient(circle at 66% 62%, ${colorB} 0%, transparent 46%), radial-gradient(circle at 50% 50%, rgba(255,255,255,0.2) 0%, transparent 34%)`,
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
    background: `linear-gradient(180deg, ${colorA} 0%, rgba(255,255,255,0.95) 58%, ${colorB} 100%)`,
    boxShadow: `0 0 ${segment.glow} rgba(255,255,255,${(0.08 + level * 0.36).toFixed(3)})`,
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
  if (version !== spectrumPollVersion || spectrumPollPending || !showDetail.value || !nowPlaying.value) {
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
  if (spectrumPollTimer || !showDetail.value || !nowPlaying.value) return;
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
  cancelAnimationFrame(cardSpringRaf);
  cardInteracting.value = true;
}

let cardTouchStartX = 0;
let cardTouchStartY = 0;

function onCardTouchStart(e: TouchEvent) {
  cardInteracting.value = true;
  cardDragging.value = true;
  cancelAnimationFrame(cardSpringRaf);
  cardTouchStartX = e.touches[0].clientX;
  cardTouchStartY = e.touches[0].clientY;
}

function onCardTouchMove(e: TouchEvent) {
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

watch(showDetail, (open) => {
  if (open) {
    cardRotX.value = 0;
    cardRotY.value = 0;
    cardTX.value = 0;
    cardTY.value = 0;
    startAmbient();
    startSpectrumPolling();
  } else {
    stopAmbient();
    cancelAnimationFrame(cardSpringRaf);
    stopSpectrumPolling();
  }
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

/** Return the full ordered list for a given source */
function sourceList(src: QueueSource): Track[] {
  if (src === 'recent') {
    const recent = filterDuplicateTracks(recentTracks.value);
    return recent.length ? recent : filterDuplicateTracks(libraryTracks.value).slice(0, 12);
  }
  if (src === 'playlist') return queuePlaylistTracks.value;
  if (src === 'soulseek') return queueSoulseekTracks.value;
  // 'library' – flattened in grouped order (same as libraryFlatList)
  return filterDuplicateTracks(libraryFlatList.value);
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
function playFromPlaylist(tracks: Track[], index: number) {
  queuePlaylistTracks.value = [...tracks];
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

interface PlaybackStatus { playing: boolean; finished: boolean; position: number; duration: number; }

function trackForHash(hash?: string | null) {
  if (!hash) return null;
  return tracksByHash.value[hash] ?? null;
}

async function playTrackLocally(track: Track, autoplay: boolean, position = 0) {
  if (track.local_preview_path) {
    await invoke('playback_play_absolute', { path: track.local_preview_path, growing: !!track.preview_growing });
  } else {
    await invoke('playback_play', { path: track.path });
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
  if (track.local_preview_path) {
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
  if (currentTrack?.local_preview_path && currentTrack.preview_growing) {
    duration.value = Math.max(duration.value, metadataDuration, reportedDuration, currentTime.value);
  } else if (reportedDuration > 0) {
    duration.value = reportedDuration;
  }
  return st;
}

function formatTime(s: number) {
  const m = Math.floor(s / 60);
  return `${m}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
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
  if (e) e.stopPropagation();
  try {
    const isLiked = await invoke<boolean>('toggle_like', { id: track.id });
    track.is_liked = isLiked;
    // Update the nowPlaying reference as well if it's the same track
    if (nowPlaying.value && nowPlaying.value.id === track.id) {
      nowPlaying.value.is_liked = isLiked;
    }
  } catch (error) {
    console.error('Failed to toggle like', error);
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
  invoke('playback_set_volume', { value: volume.value / 100 });
}

async function toggleDeviceMenu() {
  if (!showDeviceMenu.value) {
    deviceMenuError.value = '';
    const res = await invoke<DeviceList>('get_output_devices');
    outputDevices.value = res.devices;
    currentDevice.value = res.current;
  }
  showDeviceMenu.value = !showDeviceMenu.value;
}

async function pickLocalDevice(name: string) {
  deviceMenuError.value = '';
  try {
    if (remoteOutputPeer.value) {
      const previousRemotePeer = remoteOutputPeer.value;
      const snapshot = await capturePlaybackSnapshot();
      await invoke('set_output_device', { name });
      currentDevice.value = name;
      if (snapshot.track) {
        await playTrackLocally(snapshot.track, false, snapshot.position);
      }
      if (snapshot.playing) {
        await invoke('remote_playback_pause', remotePeerArgs(previousRemotePeer));
      }
      remoteOutputPeer.value = null;
      if (snapshot.playing && snapshot.track) {
        await invoke('playback_resume');
      }
      const st = await refreshPlaybackState();
      if (st.playing) startTicker(); else stopTicker();
      syncAndroid();
    } else {
      const useDefault = name === currentDevice.value;
      await invoke('set_output_device', { name: useDefault ? null : name });
      currentDevice.value = useDefault ? null : name;
    }
    showDeviceMenu.value = false;
  } catch (e: any) {
    deviceMenuError.value = String(e ?? 'Failed to switch output device');
  }
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
}

function onKeyDown(e: KeyboardEvent) {
  const target = e.target as HTMLElement | null;
  if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable)) {
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

function trackMatchesQuery(track: Track, query: string) {
  const q = query.trim().toLowerCase();
  if (!q) return true;

  const values = [
    track.title,
    track.artist,
    track.album,
    track.genre,
    trackTagsText(track),
    track.rarity,
    track.path,
    track.track_number != null ? String(track.track_number) : null,
    track.duration_secs != null ? String(track.duration_secs) : null,
    String(track.play_count),
    track.year != null ? String(track.year) : null,
  ];

  return values.some((value) => value?.toLowerCase().includes(q));
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
  const visibleTracks = filterDuplicateTracks(libraryTracks.value);
  const q = libraryQuery.value.trim();
  if (!q) return visibleTracks;
  return visibleTracks.filter((track) => trackMatchesQuery(track, q));
});

const searchResults = computed(() => {
  const visibleTracks = filterDuplicateTracks(libraryTracks.value);
  const q = searchQuery.value.trim();
  if (!q) return [];
  return visibleTracks.filter((track) => trackMatchesQuery(track, q));
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

const groupedByArtist = computed(() => {
  const map = new Map<string, Track[]>();
  for (const t of filteredTracks.value) {
    const key = t.artist || 'Unknown Artist';
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

async function loadLibrary() {
  libraryLoading.value = true;
  try {
    libraryTracks.value = await invoke<Track[]>('get_all_tracks');
    fetchCovers(libraryTracks.value);
  } catch (e) {
    console.error('Failed to load library:', e);
  } finally {
    libraryLoading.value = false;
  }
}

let startupLibraryRetryCount = 0;
let startupLibraryRetryTimer: ReturnType<typeof setTimeout> | null = null;
let initialAppDataTimer: ReturnType<typeof setTimeout> | null = null;

function clearStartupLibraryRetry() {
  if (startupLibraryRetryTimer) {
    clearTimeout(startupLibraryRetryTimer);
    startupLibraryRetryTimer = null;
  }
}

function clearInitialAppDataTimer() {
  if (initialAppDataTimer) {
    clearTimeout(initialAppDataTimer);
    initialAppDataTimer = null;
  }
}

function scheduleInitialAppDataLoad() {
  clearInitialAppDataTimer();
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      void loadAppData('mount');
    });
  });
  initialAppDataTimer = setTimeout(() => {
    initialAppDataTimer = null;
    void loadAppData('retry');
  }, 1200);
}

function onWindowFocus() {
  void loadAppData('resume');
}

function onDocumentVisibilityChange() {
  if (document.visibilityState === 'visible') {
    void loadAppData('resume');
  }
}

async function loadAppData(reason: 'mount' | 'resume' | 'sync' | 'retry' = 'mount') {
  const shouldLoadLibrary = reason !== 'resume' || libraryTracks.value.length === 0;
  await Promise.allSettled([
    shouldLoadLibrary ? loadLibrary() : Promise.resolve(),
    loadRecent(),
    loadPlaylists(),
    loadSmartPlaylists(),
  ]);

  if (reason !== 'sync' && startupLibraryRetryCount < 2 && libraryTracks.value.length === 0 && playlists.value.length === 0 && smartPlaylists.value.length === 0) {
    clearStartupLibraryRetry();
    startupLibraryRetryCount += 1;
    const delayMs = startupLibraryRetryCount === 1 ? 900 : 2200;
    startupLibraryRetryTimer = setTimeout(() => {
      startupLibraryRetryTimer = null;
      void loadAppData('retry');
    }, delayMs);
    return;
  }

  startupLibraryRetryCount = 0;
  clearStartupLibraryRetry();
}

async function loadRecent() {
  try {
    recentTracks.value = await invoke<Track[]>('get_recent_tracks', { limit: 12 });
    fetchCovers(recentTracks.value);
  } catch (_) {}
}

async function loadHistory() {
  historyLoading.value = true;
  try {
    historyEntries.value = await invoke<PlayHistoryEntry[]>('get_play_history');
    fetchCovers(historyEntries.value.map(e => e.track));
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

  try {
    aboutUpdateStatus.value = await invoke<AboutUpdateStatus>('about_check_updates');
  } catch (error) {
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

async function fetchCovers(tracks: Track[]) {
  for (const t of tracks) {
    if (covers.value[t.id] !== undefined) continue;
    invoke<string | null>('get_track_cover', { id: t.id }).then(url => {
      covers.value[t.id] = url;
    });
  }
}

function openEditor(track: Track) {
  editForm.value = {
    title: track.title || '',
    artist: track.artist || '',
    album: track.album || '',
    track_number: track.track_number,
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
  await invoke('update_track', {
    id: activeTrack.id,
    title: editForm.value.title || null,
    artist: editForm.value.artist || null,
    album: editForm.value.album || null,
    trackNumber: normalizeOptionalInteger(editForm.value.track_number),
    year: normalizeOptionalInteger(editForm.value.year),
    genre: editForm.value.genre.trim() || null,
    tags: normalizeTrackTagsInput(editForm.value.tags),
    playCount: normalizeNonNegativeInteger(editForm.value.play_count),
    isLiked: editForm.value.is_liked,
    dateAdded: parseTrackDateInput(editForm.value.date_added),
    rarity: editForm.value.rarity.trim() || null,
  });
  editingTrack.value = null;
  await loadLibrary();
  await loadRecent();
  if (activePlaylistId !== null && playlistView.value?.id === activePlaylistId) {
    const tracks = filterDuplicateTracks(await invoke<Track[]>('get_playlist_tracks', { playlistId: activePlaylistId }));
    playlistView.value = { ...playlistView.value, tracks };
  }
  if (nowPlaying.value?.id === activeTrack.id) {
    const refreshedTrack = libraryTracks.value.find((track) => track.id === activeTrack.id);
    if (refreshedTrack) {
      nowPlaying.value = refreshedTrack;
      isLiked.value = refreshedTrack.is_liked;
    }
  }
}

function formatDuration(secs: number | null) {
  if (!secs) return '--:--';
  const m = Math.floor(secs / 60);
  return `${m}:${String(Math.floor(secs % 60)).padStart(2, '0')}`;
}

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
    await openPath(path);
  } catch (_) {
    try { await navigator.clipboard.writeText(path); } catch (_2) {}
    alert(`Data folder:\n${path}`);
  }
}

async function doReindex() {
  invoke('reindex');
}

async function startIdentify() {
  const ids = libraryTracks.value.filter(t => !t.manually_edited).map(t => t.id);
  if (!ids.length) return;
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
  if (nav === 'about') {
    void loadAbout();
  }
});

onMounted(() => {
  document.addEventListener('click', onDocClick);
  document.addEventListener('keydown', onKeyDown);
  window.addEventListener('focus', onWindowFocus);
  document.addEventListener('visibilitychange', onDocumentVisibilityChange);
  scheduleInitialAppDataLoad();
  void loadSoulseekStatus();
  invoke<DeviceSettings>('get_device_settings')
    .then((cfg) => {
      if (cfg?.emoji) deviceEmoji.value = cfg.emoji;
      syncEnabled.value = !!cfg?.sync_enabled;
    })
    .catch(() => {});
  ensureLibraryDataDir().catch(() => {});
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
      if (nowPlaying.value?.id === soulseekTrackIdFromParts(payload.username, payload.filename) && nowPlaying.value.local_preview_path) {
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
  
  listen('library-changed', () => { void loadAppData('sync'); });
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

    if (p.phase === 'download') {
      session.filesAdded = p.done;
      if (p.message) session.files.push(p.message);
      indexLog.value = [...indexLog.value]; // trigger reactivity
    } else if (p.phase === 'done') {
      session.filesAdded = p.done;
      session.addedCount = p.done;
      session.status = 'done';
      session.finishedAt = Date.now();
      indexLog.value = [...indexLog.value];
      delete syncSessions[p.peer];
      void loadAppData('sync');
      loadHistory();
    } else if (p.phase === 'error') {
      session.status = 'error';
      session.errorMsg = p.message || 'Connection failed';
      session.finishedAt = Date.now();
      indexLog.value = [...indexLog.value];
      delete syncSessions[p.peer];
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
    const ensureLocalSession = () => {
      if (!localSession) {
        localSession = { id: crypto.randomUUID(), kind: 'local', addedCount: 0, filesAdded: 0, files: [], startedAt: Date.now(), status: 'running' };
        indexLog.value.push(localSession);
      }
    };
    if (p.status === 'indexing' || p.status === 'scanning') {
      ensureLocalSession();
      indexCurrent.value = p.current;
      indexTotal.value = p.total;
      indexRunning.value = true;
      indexDone.value = false;
      if (indexDismissTimer) { clearTimeout(indexDismissTimer); indexDismissTimer = null; }
    } else if (p.status === 'added') {
      ensureLocalSession();
      localSession!.addedCount++;
      localSession!.filesAdded++;
      if (p.track_name) localSession!.files.push(p.track_name);
      // trigger reactivity
      indexLog.value = [...indexLog.value];
    } else if (p.status === 'done') {
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
  document.removeEventListener('click', onDocClick);
  document.removeEventListener('keydown', onKeyDown);
  window.removeEventListener('focus', onWindowFocus);
  document.removeEventListener('visibilitychange', onDocumentVisibilityChange);
  delete (window as any)._playbackFinished;
  clearTrackLongPress();
  clearHomePinnedLongPress();
  clearStartupLibraryRetry();
  clearInitialAppDataTimer();
  if (unlistenSoulseekDownload) unlistenSoulseekDownload();
  if (unlistenSoulseekPreview) unlistenSoulseekPreview();
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
});
</script>

<template>
  <div class="app">
    <Transition name="nav-overlay">
      <div v-if="showMobileNav" class="mobile-nav-overlay" @click="showMobileNav = false" />
    </Transition>
    <!-- Sidebar -->
    <aside class="sidebar" :class="{ 'sidebar-open': showMobileNav }">
      <div class="sidebar-mobile-header">
        <button class="icon-btn sidebar-close-btn" @click="showMobileNav = false" aria-label="Close menu">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M19 6.41 17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/></svg>
        </button>
      </div>
      <!-- <div class="brand">
        <svg viewBox="0 0 24 24" fill="#1db954" width="32" height="32">
          <path d="M12 2C6.477 2 2 6.477 2 12s4.477 10 10 10 10-4.477 10-10S17.523 2 12 2zm4.586 14.424a.622.622 0 0 1-.857.207c-2.348-1.435-5.304-1.76-8.785-.964a.622.622 0 1 1-.277-1.215c3.809-.87 7.076-.496 9.712 1.115a.622.622 0 0 1 .207.857zm1.223-2.722a.78.78 0 0 1-1.072.257c-2.687-1.652-6.785-2.131-9.965-1.166a.78.78 0 1 1-.453-1.492c3.632-1.102 8.147-.568 11.233 1.329a.78.78 0 0 1 .257 1.072zm.105-2.835C14.692 8.95 9.375 8.775 6.297 9.71a.937.937 0 1 1-.543-1.793c3.541-1.073 9.43-.865 13.152 1.337a.937.937 0 0 1-.992 1.613z"/>
        </svg>
        <span class="brand-name">Player</span>
      </div> -->

      <nav>
        <a class="nav-item" :class="{ active: activeNav === 'home' }" @click.prevent="activeNav = 'home'; showMobileNav = false" href="#">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z"/></svg>
          Home
        </a>
        <a class="nav-item" :class="{ active: activeNav === 'search' }" @click.prevent="activeNav = 'search'; showMobileNav = false" href="#">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M15.5 14h-.79l-.28-.27A6.471 6.471 0 0 0 16 9.5 6.5 6.5 0 1 0 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z"/></svg>
          Search
        </a>
        <a class="nav-item" :class="{ active: activeNav === 'library' }" @click.prevent="activeNav = 'library'; showMobileNav = false" href="#">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M4 6H2v14c0 1.1.9 2 2 2h14v-2H4V6zm16-4H8c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm-1 9H9V9h10v2zm-4 4H9v-2h6v2zm4-8H9V5h10v2z"/></svg>
          Your Library
        </a>
        <a class="nav-item" :class="{ active: activeNav === 'playlists' }" @click.prevent="activeNav = 'playlists'; playlistView = null; playlistTab = 'regular'; showMobileNav = false" href="#">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M15 6H3v2h12V6zm0 4H3v2h12v-2zM3 16h8v-2H3v2zM17 6v8.18c-.31-.11-.65-.18-1-.18-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3V8h3V6h-5z"/></svg>
          Playlists
        </a>
        <a class="nav-item" :class="{ active: activeNav === 'playlists' && playlistTab === 'smart' }" @click.prevent="activeNav = 'playlists'; playlistTab = 'smart'; playlistView = null; editingSP = null; smartView = null; showMobileNav = false" href="#">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M19 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V5a2 2 0 0 0-2-2zm-7 3c1.93 0 3.5 1.57 3.5 3.5S13.93 13 12 13s-3.5-1.57-3.5-3.5S10.07 6 12 6zm7 13H5v-.23c0-.62.28-1.2.76-1.58C7.47 15.82 9.64 15 12 15s4.53.82 6.24 2.19c.48.38.76.97.76 1.58V19z"/></svg>
          Flexible Playlists
        </a>
      </nav>

      <div class="sidebar-divider" />

      <nav>
        <!-- <a class="nav-item" href="#">
          <span class="icon-box create">+</span>
          Create Playlist
        </a>
        <a class="nav-item" href="#">
          <span class="icon-box liked">
            <svg viewBox="0 0 24 24" fill="white" width="14" height="14"><path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"/></svg>
          </span>
          Liked Songs
        </a> -->
        <a class="nav-item" :class="{ active: activeNav === 'discovery' }" @click.prevent="activeNav = 'discovery'; showMobileNav = false" href="#">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8 3 3 3-3a4.237 4.237 0 0 0-6 0zm-4-4 2 2a7.074 7.074 0 0 1 10 0l2-2C15.14 9.14 8.87 9.14 5 13z"/></svg>
          Devices
          <span v-if="peers.length" class="peer-badge">{{ peers.length }}</span>
        </a>
        <a class="nav-item" href="#" @click.prevent="openDataDir(); showMobileNav = false">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M10 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z"/></svg>
          Open Data Folder
        </a>
        <a class="nav-item" href="#" @click.prevent="doReindex; showMobileNav = false">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M17.65 6.35A7.958 7.958 0 0 0 12 4c-4.42 0-7.99 3.58-7.99 8s3.57 8 7.99 8c3.73 0 6.84-2.55 7.73-6h-2.08A5.99 5.99 0 0 1 12 18c-3.31 0-6-2.69-6-6s2.69-6 6-6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z"/></svg>
          Reindex
        </a>
        <a class="nav-item" href="#" @click.prevent="startIdentify(); showMobileNav = false">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="m22 2-2.5 1.4L17.1 2l1.4 2.5L17.1 7l2.4-1.4L22 7l-1.4-2.5zm-7.63 5.29a.996.996 0 0 0-1.41 0L1.29 18.96a.996.996 0 0 0 0 1.41l2.34 2.34c.39.39 1.02.39 1.41 0L16.7 11.05a.996.996 0 0 0 0-1.41l-2.33-2.35zM5.21 19.38l-1.59-1.59 8.93-8.93 1.59 1.59-8.93 8.93z"/></svg>
          Identify
        </a>
        <a class="nav-item" :class="{ active: activeNav === 'dedup' }" href="#" @click.prevent="activeNav = 'dedup'; showMobileNav = false; openDedup()">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M15 4H5v16h14V8zm-1 13H7v-2h7zm0-4H7v-2h7zm-3-4H7V7h4zM3 2v18H1V2zm18 0h2v18h-2z"/></svg>
          Duplicates
        </a>
        <a class="nav-item" :class="{ active: activeNav === 'about' }" href="#" @click.prevent="activeNav = 'about'; showMobileNav = false">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M11 7h2V5h-2v2zm0 12h2v-8h-2v8zm1-17C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2z"/></svg>
          About
        </a>
      </nav>
    </aside>

    <!-- Main -->
    <main class="main">
      <header class="topbar">
        <button class="burger-btn" @click="showMobileNav = true" aria-label="Menu">
          <svg viewBox="0 0 24 24" fill="currentColor" width="24" height="24"><path d="M3 18h18v-2H3v2zm0-5h18v-2H3v2zm0-7v2h18V6H3z"/></svg>
        </button>
        <div class="nav-arrows">
          <button class="arrow-btn">&lsaquo;</button>
          <button class="arrow-btn">&rsaquo;</button>
        </div>
        <button
          class="duplicates-toggle"
          :class="{ active: showDuplicateTracks }"
          @click="showDuplicateTracks = !showDuplicateTracks"
        >
          {{ showDuplicateTracks ? 'Hide duplicates' : 'Show duplicates' }}
        </button>
        <!-- <div class="user-menu-wrapper">
          <button class="user-btn" @click.stop="showUserMenu = !showUserMenu">
            <span class="avatar">e</span>
            <span>user</span>
            <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"
              :style="{ transform: showUserMenu ? 'rotate(180deg)' : 'none', transition: 'transform .15s' }">
              <path d="M7 10l5 5 5-5z"/>
            </svg>
          </button>
          <Transition name="dropdown">
            <div v-if="showUserMenu" class="dropdown">
              <a href="#">Account
                <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M19 19H5V5h7V3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z"/></svg>
              </a>
              <a href="#">Profile</a>
              <a href="#">Support
                <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M19 19H5V5h7V3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z"/></svg>
              </a>
              <a href="#">Download
                <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M19 19H5V5h7V3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z"/></svg>
              </a>
              <a href="#">Settings</a>
              <div class="dropdown-sep" />
              <a href="#">Log out</a>
            </div>
          </Transition>
        </div> -->
      </header>

      <div ref="contentRef" class="content" @scroll.passive="maybeLoadMoreSoulseekResults">
        <!-- Home view -->
        <template v-if="activeNav === 'home'">
          <section>
            <div class="section-head">
              <h2>Recently played</h2>
              <a class="show-all" href="#" @click.prevent="activeNav = 'history'; loadHistory()">Show all</a>
            </div>
            <div v-if="visibleRecentTracks.length === 0 && filterDuplicateTracks(libraryTracks).length === 0" class="library-empty">No tracks yet.</div>
            <div v-else class="card-list">
              <div v-for="(track, idx) in (visibleRecentTracks.length ? visibleRecentTracks : filterDuplicateTracks(libraryTracks).slice(0, 12))" :key="track.id + '-' + idx"
                class="card" :class="rarityClass(track.rarity)" :style="rarityVars(track.rarity)"
                @click="playRecentCard(idx)"
                @contextmenu.prevent="openTrackContextMenu($event, track)"
                @touchstart.passive="startTrackRowLongPress($event, track)"
                @touchmove.passive="moveTrackRowLongPress"
                @touchend="endTrackRowLongPress"
                @touchcancel="endTrackRowLongPress">
                <div class="cover" :style="covers[track.id]
                  ? `background-image: url(${covers[track.id]}); background-size: cover; background-position: center`
                  : `background: linear-gradient(135deg, ${hashToColors(track.file_hash)[0]}, ${hashToColors(track.file_hash)[1]})`">
                  <div class="hover-play">
                    <button class="green-circle" type="button" @click.stop="toggleRecentCardPlayback(idx, track.id)">
                      <svg v-if="!isTrackPlaying(track.id)" viewBox="0 0 24 24" fill="black" width="18" height="18"><path d="M8 5v14l11-7z"/></svg>
                      <svg v-else viewBox="0 0 24 24" fill="black" width="18" height="18"><path d="M6 19h4V5H6v14zm8-14v14h4V5h-4z"/></svg>
                    </button>
                  </div>
                </div>
                <div class="card-title">{{ track.title || track.path }}</div>
                <div class="card-artist">{{ track.artist || 'Unknown' }}</div>
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
            <div v-else class="card-list">
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
                      <svg viewBox="0 0 24 24" fill="black" width="18" height="18"><path d="M8 5v14l11-7z"/></svg>
                    </button>
                  </div>
                  <svg v-if="item.kind === 'regular'" viewBox="0 0 24 24" fill="rgba(255,255,255,0.88)" width="30" height="30"><path d="M15 6H3v2h12V6zm0 4H3v2h12v-2zM3 16h8v-2H3v2zM17 6v8.18c-.31-.11-.65-.18-1-.18-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3V8h3V6h-5z"/></svg>
                  <svg v-else viewBox="0 0 24 24" fill="rgba(255,255,255,0.88)" width="30" height="30"><path d="M19 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V5a2 2 0 0 0-2-2zm-7 3c1.93 0 3.5 1.57 3.5 3.5S13.93 13 12 13s-3.5-1.57-3.5-3.5S10.07 6 12 6zm7 13H5v-.23c0-.62.28-1.2.76-1.58C7.47 15.82 9.64 15 12 15s4.53.82 6.24 2.19c.48.38.76.97.76 1.58V19z"/></svg>
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
              <div
                v-for="(track, idx) in item.previewTracks"
                :key="`${item.key}:${track.id}`"
                class="track-row"
                :class="[rarityClass(track.rarity), { 'track-row-current': isCurrentTrack(track.id), 'track-row-next': isNextTrack(track.id) }]"
                :style="rarityVars(track.rarity)"
                @click="playPlaylistTrack(item.tracks, idx)"
                @contextmenu.prevent="openTrackContextMenu($event, track)"
                @touchstart.passive="startTrackRowLongPress($event, track)"
                @touchmove.passive="moveTrackRowLongPress"
                @touchend="endTrackRowLongPress"
                @touchcancel="endTrackRowLongPress"
              >
                <div class="track-cover-sm" :style="covers[track.id]
                  ? `background-image: url(${covers[track.id]}); background-size: cover; background-position: center`
                  : `background: linear-gradient(135deg, ${hashToColors(track.file_hash)[0]}, ${hashToColors(track.file_hash)[1]})`"
                />
                <div class="track-info">
                  <div class="track-title-row">
                    <span class="track-title">{{ track.title || track.path }}</span>
                    <span v-if="isCurrentTrack(track.id)" class="track-playback-badge current" title="Now playing">
                      <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M8 5v14l11-7z"/></svg>
                    </span>
                    <span v-else-if="isNextTrack(track.id)" class="track-playback-badge next" title="Up next">
                      <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M8 5v14l11-7z"/></svg>
                      <span class="track-playback-step">1</span>
                    </span>
                  </div>
                  <span class="track-album">{{ track.artist || 'Unknown' }}{{ track.album ? ' · ' + track.album : '' }}{{ track.genre ? ' · ' + track.genre : '' }}</span>
                </div>
                <span class="track-dur">{{ formatDuration(track.duration_secs) }}</span>
              </div>
            </div>
            <div v-if="item.tracks.length > item.previewTracks.length" class="library-empty home-playlist-more">… and {{ item.tracks.length - item.previewTracks.length }} more</div>
          </section>

          <section>
            <div class="section-head">
              <h2>Made For You</h2>
              <a class="show-all" href="#">Show all</a>
            </div>
            <div class="card-list">
              <div v-for="item in madeForYou" :key="item.id" class="card">
                <div class="cover" :style="`background: linear-gradient(135deg, ${item.colors[0]}, ${item.colors[1]})`">
                  <div class="hover-play">
                    <button class="green-circle">
                      <svg viewBox="0 0 24 24" fill="black" width="18" height="18"><path d="M8 5v14l11-7z"/></svg>
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
            <div class="library-header">
              <h2>Your Library</h2>
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
                <div class="group-artist">{{ artist }}</div>
                <div
                  v-for="track in tracks"
                  :key="track.id"
                  class="track-row" :class="[rarityClass(track.rarity), { 'track-row-current': isCurrentTrack(track.id), 'track-row-next': isNextTrack(track.id) }]"
                  :style="rarityVars(track.rarity)"
                  @click="playLibraryTrack(libraryFlatList.indexOf(track))"
                  @contextmenu.prevent="openTrackContextMenu($event, track)"
                  @touchstart.passive="startTrackRowLongPress($event, track)"
                  @touchmove.passive="moveTrackRowLongPress"
                  @touchend="endTrackRowLongPress"
                  @touchcancel="endTrackRowLongPress"
                >
                  <div class="track-cover-sm" :style="covers[track.id]
                    ? `background-image: url(${covers[track.id]}); background-size: cover; background-position: center`
                    : `background: linear-gradient(135deg, ${hashToColors(track.file_hash)[0]}, ${hashToColors(track.file_hash)[1]})`"
                  />
                  <span class="track-num">{{ track.track_number ?? '–' }}</span>
                  <div class="track-info">
                    <div class="track-title-row">
                      <span class="track-title">{{ track.title || track.path }}</span>
                      <span v-if="isCurrentTrack(track.id)" class="track-playback-badge current" title="Now playing">
                        <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M8 5v14l11-7z"/></svg>
                      </span>
                      <span v-else-if="isNextTrack(track.id)" class="track-playback-badge next" title="Up next">
                        <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M8 5v14l11-7z"/></svg>
                        <span class="track-playback-step">1</span>
                      </span>
                    </div>
                    <span class="track-album">{{ track.album || '' }}</span>
                  </div>
                  <button class="icon-btn edit-btn track-inline-action" title="Identify" @click.stop="identifySingle(track)">
                    <svg viewBox="0 0 24 24" fill="currentColor" width="14" height="14"><path d="m22 2-2.5 1.4L17.1 2l1.4 2.5L17.1 7l2.4-1.4L22 7l-1.4-2.5zm-7.63 5.29a.996.996 0 0 0-1.41 0L1.29 18.96a.996.996 0 0 0 0 1.41l2.34 2.34c.39.39 1.02.39 1.41 0L16.7 11.05a.996.996 0 0 0 0-1.41l-2.33-2.35zM5.21 19.38l-1.59-1.59 8.93-8.93 1.59 1.59-8.93 8.93z"/></svg>
                  </button>
                  <button class="icon-btn edit-btn track-inline-action" title="Edit" @click.stop="openEditor(track)">
                    <svg viewBox="0 0 24 24" fill="currentColor" width="14" height="14"><path d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04a1.003 1.003 0 0 0 0-1.42l-2.34-2.33a1.003 1.003 0 0 0-1.42 0l-1.83 1.83 3.75 3.75 1.84-1.83z"/></svg>
                  </button>
                  <span title="Play Count" v-if="track.play_count > 0" class="track-play-count" style="margin-right: 8px;">
                    ▶ {{ track.play_count }}
                  </span>
                  <button class="icon-btn like-btn track-inline-action" @click.stop="toggleLike(track)" style="margin-right: 8px;">
                    <svg v-if="track.is_liked" viewBox="0 0 24 24" fill="#1db954" width="16" height="16"><path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"/></svg>
                    <svg v-else viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M16.5 3c-1.74 0-3.41.81-4.5 2.09C10.91 3.81 9.24 3 7.5 3 4.42 3 2 5.42 2 8.5c0 3.78 3.4 6.86 8.55 11.54L12 21.35l1.45-1.32C18.6 15.36 22 12.28 22 8.5 22 5.42 19.58 3 16.5 3zm-4.4 15.55l-.1.1-.1-.1C7.14 14.24 4 11.39 4 8.5 4 6.5 5.5 5 7.5 5c1.54 0 3.04.99 3.57 2.36h1.87C13.46 5.99 14.96 5 16.5 5c2 0 3.5 1.5 3.5 3.5 0 2.89-3.14 5.74-7.9 10.05z"/></svg>
                  </button>
                  <button class="icon-btn edit-btn track-inline-action" title="Add to playlist" style="margin-right: 8px;" @click.stop="openAddToPlaylistMenu($event, track)">
                    <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M14 10H3v2h11v-2zm0-4H3v2h11V6zM3 16h7v-2H3v2zm11.41-2.83L13 14.59 11.59 13 10 14.59l3 3.01 4-4.01-1.59-1.42z"/></svg>
                  </button>
                  <span class="track-dur">{{ formatDuration(track.duration_secs) }}</span>
                </div>
              </div>
            </div>
          </section>
        </template>

        <!-- Search view -->
        <template v-else-if="activeNav === 'search'">
          <section>
            <div class="library-header">
              <h2>Search</h2>
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
                  <div
                    v-for="(track, idx) in searchRecentTracks"
                    :key="track.id"
                    class="track-row"
                    :class="[rarityClass(track.rarity), { 'track-row-current': isCurrentTrack(track.id), 'track-row-next': isNextTrack(track.id) }]"
                    :style="rarityVars(track.rarity)"
                    @click="playPlaylistTrack(searchRecentTracks, idx)"
                    @contextmenu.prevent="openTrackContextMenu($event, track)"
                    @touchstart.passive="startTrackRowLongPress($event, track)"
                    @touchmove.passive="moveTrackRowLongPress"
                    @touchend="endTrackRowLongPress"
                    @touchcancel="endTrackRowLongPress"
                  >
                    <div class="track-cover-sm" :style="covers[track.id]
                      ? `background-image: url(${covers[track.id]}); background-size: cover; background-position: center`
                      : `background: linear-gradient(135deg, ${hashToColors(track.file_hash)[0]}, ${hashToColors(track.file_hash)[1]})`"
                    />
                    <div class="track-info">
                      <div class="track-title-row">
                        <span class="track-title">{{ track.title || track.path }}</span>
                        <span v-if="isCurrentTrack(track.id)" class="track-playback-badge current" title="Now playing">
                          <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M8 5v14l11-7z"/></svg>
                        </span>
                        <span v-else-if="isNextTrack(track.id)" class="track-playback-badge next" title="Up next">
                          <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M8 5v14l11-7z"/></svg>
                          <span class="track-playback-step">1</span>
                        </span>
                      </div>
                      <span class="track-album">{{ track.artist || 'Unknown' }}{{ track.album ? ' · ' + track.album : '' }}{{ track.genre ? ' · ' + track.genre : '' }}</span>
                    </div>
                    <button class="icon-btn edit-btn track-inline-action" title="Add to playlist" style="margin-right: 8px;" @click.stop="openAddToPlaylistMenu($event, track)">
                      <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M14 10H3v2h11v-2zm0-4H3v2h11V6zM3 16h7v-2H3v2zm11.41-2.83L13 14.59 11.59 13 10 14.59l3 3.01 4-4.01-1.59-1.42z"/></svg>
                    </button>
                    <button class="icon-btn like-btn track-inline-action" @click.stop="toggleLike(track)" style="margin-right: 8px;">
                      <svg v-if="track.is_liked" viewBox="0 0 24 24" fill="#1db954" width="16" height="16"><path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"/></svg>
                      <svg v-else viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M16.5 3c-1.74 0-3.41.81-4.5 2.09C10.91 3.81 9.24 3 7.5 3 4.42 3 2 5.42 2 8.5c0 3.78 3.4 6.86 8.55 11.54L12 21.35l1.45-1.32C18.6 15.36 22 12.28 22 8.5 22 5.42 19.58 3 16.5 3zm-4.4 15.55l-.1.1-.1-.1C7.14 14.24 4 11.39 4 8.5 4 6.5 5.5 5 7.5 5c1.54 0 3.04.99 3.57 2.36h1.87C13.46 5.99 14.96 5 16.5 5c2 0 3.5 1.5 3.5 3.5 0 2.89-3.14 5.74-7.9 10.05z"/></svg>
                    </button>
                    <span class="track-dur">{{ formatDuration(track.duration_secs) }}</span>
                  </div>
                </div>
              </template>

              <template v-else>
                <div class="search-summary">
                  <span><strong>{{ searchResults.length }}</strong> local tracks</span>
                  <span><strong>{{ searchArtistCount }}</strong> artists</span>
                  <span><strong>{{ searchAlbumCount }}</strong> albums</span>
                  <span v-if="soulseekReady"><strong>{{ searchSoulseekCountLabel }}</strong> Soulseek tracks</span>
                </div>

                <div class="section-head search-section-head">
                  <h2>Local Library</h2>
                </div>

                <div v-if="searchResults.length === 0" class="library-empty search-inline-empty">
                  Nothing in your library for "{{ searchQuery }}".
                </div>

                <div v-else class="track-list">
                  <div
                    v-for="(track, idx) in searchResults"
                    :key="track.id"
                    class="track-row"
                    :class="[rarityClass(track.rarity), { 'track-row-current': isCurrentTrack(track.id), 'track-row-next': isNextTrack(track.id) }]"
                    :style="rarityVars(track.rarity)"
                    @click="playPlaylistTrack(searchResults, idx)"
                    @contextmenu.prevent="openTrackContextMenu($event, track)"
                    @touchstart.passive="startTrackRowLongPress($event, track)"
                    @touchmove.passive="moveTrackRowLongPress"
                    @touchend="endTrackRowLongPress"
                    @touchcancel="endTrackRowLongPress"
                  >
                    <div class="track-cover-sm" :style="covers[track.id]
                      ? `background-image: url(${covers[track.id]}); background-size: cover; background-position: center`
                      : `background: linear-gradient(135deg, ${hashToColors(track.file_hash)[0]}, ${hashToColors(track.file_hash)[1]})`"
                    />
                    <div class="track-info">
                      <div class="track-title-row">
                        <span class="track-title">{{ track.title || track.path }}</span>
                        <span v-if="isCurrentTrack(track.id)" class="track-playback-badge current" title="Now playing">
                          <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M8 5v14l11-7z"/></svg>
                        </span>
                        <span v-else-if="isNextTrack(track.id)" class="track-playback-badge next" title="Up next">
                          <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M8 5v14l11-7z"/></svg>
                          <span class="track-playback-step">1</span>
                        </span>
                      </div>
                      <span class="track-album">{{ track.artist || 'Unknown' }}{{ track.album ? ' · ' + track.album : '' }}{{ track.genre ? ' · ' + track.genre : '' }}{{ track.year ? ' · ' + track.year : '' }}</span>
                    </div>
                    <button class="icon-btn edit-btn track-inline-action" title="Add to playlist" style="margin-right: 8px;" @click.stop="openAddToPlaylistMenu($event, track)">
                      <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M14 10H3v2h11v-2zm0-4H3v2h11V6zM3 16h7v-2H3v2zm11.41-2.83L13 14.59 11.59 13 10 14.59l3 3.01 4-4.01-1.59-1.42z"/></svg>
                    </button>
                    <button class="icon-btn like-btn track-inline-action" @click.stop="toggleLike(track)" style="margin-right: 8px;">
                      <svg v-if="track.is_liked" viewBox="0 0 24 24" fill="#1db954" width="16" height="16"><path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"/></svg>
                      <svg v-else viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M16.5 3c-1.74 0-3.41.81-4.5 2.09C10.91 3.81 9.24 3 7.5 3 4.42 3 2 5.42 2 8.5c0 3.78 3.4 6.86 8.55 11.54L12 21.35l1.45-1.32C18.6 15.36 22 12.28 22 8.5 22 5.42 19.58 3 16.5 3zm-4.4 15.55l-.1.1-.1-.1C7.14 14.24 4 11.39 4 8.5 4 6.5 5.5 5 7.5 5c1.54 0 3.04.99 3.57 2.36h1.87C13.46 5.99 14.96 5 16.5 5c2 0 3.5 1.5 3.5 3.5 0 2.89-3.14 5.74-7.9 10.05z"/></svg>
                    </button>
                    <span class="track-dur">{{ formatDuration(track.duration_secs) }}</span>
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
                  <div class="track-cover-sm soulseek-cover-sm" :style="soulseekCoverUrl(result)
                    ? `background-image: url(${soulseekCoverUrl(result)}); background-size: cover; background-position: center`
                    : ''">
                    <span v-if="!soulseekCoverUrl(result)">{{ result.coverFilename ? 'ART' : 'SL' }}</span>
                  </div>
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
                  <svg viewBox="0 0 24 24" fill="currentColor" width="20" height="20"><path d="M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z"/></svg>
                </button>
                <div>
                  <h2 style="margin:0;">Search and replace</h2>
                  <p class="soulseek-replace-track-copy">
                    Replace <strong>{{ trackReplaceDialog.track.title || trackReplaceDialog.track.path }}</strong> with a Soulseek match.
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
                <div class="track-cover-sm soulseek-cover-sm" :style="soulseekCoverUrl(result)
                  ? `background-image: url(${soulseekCoverUrl(result)}); background-size: cover; background-position: center`
                  : ''">
                  <span v-if="!soulseekCoverUrl(result)">{{ result.coverFilename ? 'ART' : 'SL' }}</span>
                </div>
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
                    <svg v-if="soulseekDownloadState(result)?.state === 'completed'" viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M9 16.17 4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z"/></svg>
                    <svg v-else viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M5 20h14v-2H5v2zm7-18-5.5 5.5 1.42 1.42L11 5.84V16h2V5.84l3.08 3.08 1.42-1.42L12 2z"/></svg>
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
                  <svg viewBox="0 0 24 24" fill="currentColor" width="20" height="20"><path d="M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z"/></svg>
                </button>
                <h2 style="margin:0;">Search and replace</h2>
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
                <button class="icon-btn" style="padding:4px;" @click="activeNav = 'home'">
                  <svg viewBox="0 0 24 24" fill="currentColor" width="20" height="20"><path d="M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z"/></svg>
                </button>
                <h2 style="margin:0;">Play History</h2>
              </div>
            </div>
            <div v-if="historyLoading" class="library-empty">Loading...</div>
            <div v-else-if="historyEntries.length === 0" class="library-empty">No history yet.</div>
            <div v-else class="history-list">
              <div
                v-for="(entry, idx) in historyEntries"
                :key="idx"
                class="track-row"
                :class="[rarityClass(entry.track.rarity), { 'track-row-current': isCurrentTrack(entry.track.id), 'track-row-next': isNextTrack(entry.track.id) }]"
                :style="rarityVars(entry.track.rarity)"
                @click="playLibraryTrack(libraryFlatList.findIndex(t => t.id === entry.track.id))"
                @contextmenu.prevent="openTrackContextMenu($event, entry.track)"
                @touchstart.passive="startTrackRowLongPress($event, entry.track)"
                @touchmove.passive="moveTrackRowLongPress"
                @touchend="endTrackRowLongPress"
                @touchcancel="endTrackRowLongPress"
              >
                <div class="track-cover-sm" :style="covers[entry.track.id]
                  ? `background-image: url(${covers[entry.track.id]}); background-size: cover; background-position: center`
                  : `background: linear-gradient(135deg, ${hashToColors(entry.track.file_hash)[0]}, ${hashToColors(entry.track.file_hash)[1]})`"
                />
                <div class="track-info">
                  <div class="track-title-row">
                    <span class="track-title">{{ entry.track.title || entry.track.path }}</span>
                    <span v-if="isCurrentTrack(entry.track.id)" class="track-playback-badge current" title="Now playing">
                      <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M8 5v14l11-7z"/></svg>
                    </span>
                    <span v-else-if="isNextTrack(entry.track.id)" class="track-playback-badge next" title="Up next">
                      <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M8 5v14l11-7z"/></svg>
                      <span class="track-playback-step">1</span>
                    </span>
                  </div>
                  <span class="track-album">{{ entry.track.artist || 'Unknown' }}{{ entry.track.album ? ' · ' + entry.track.album : '' }}</span>
                </div>
                <span class="history-time" style="margin-right:12px;">{{ formatHistoryDate(entry.played_at) }}</span>
                <button class="icon-btn like-btn track-inline-action" @click.stop="toggleLike(entry.track)" style="margin-right:8px;">
                  <svg v-if="entry.track.is_liked" viewBox="0 0 24 24" fill="#1db954" width="16" height="16"><path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"/></svg>
                  <svg v-else viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M16.5 3c-1.74 0-3.41.81-4.5 2.09C10.91 3.81 9.24 3 7.5 3 4.42 3 2 5.42 2 8.5c0 3.78 3.4 6.86 8.55 11.54L12 21.35l1.45-1.32C18.6 15.36 22 12.28 22 8.5 22 5.42 19.58 3 16.5 3zm-4.4 15.55l-.1.1-.1-.1C7.14 14.24 4 11.39 4 8.5 4 6.5 5.5 5 7.5 5c1.54 0 3.04.99 3.57 2.36h1.87C13.46 5.99 14.96 5 16.5 5c2 0 3.5 1.5 3.5 3.5 0 2.89-3.14 5.74-7.9 10.05z"/></svg>
                </button>
                <span class="track-dur">{{ formatDuration(entry.track.duration_secs) }}</span>
              </div>
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
                <div class="library-header">
                  <div style="display:flex; align-items:center; gap:12px;">
                    <button class="icon-btn" style="padding:4px;" @click="playlistView = null; loadPlaylists()">
                      <svg viewBox="0 0 24 24" fill="currentColor" width="20" height="20"><path d="M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z"/></svg>
                    </button>
                    <h2 style="margin:0;">{{ playlistView.name }}</h2>
                  </div>
                  <button v-if="playlistView.tracks.length" class="icon-btn text-action-btn" style="padding:6px 12px;" @click="playFromPlaylist(playlistView!.tracks, 0)">▶ Play</button>
                </div>
                <div v-if="playlistView.tracks.length === 0" class="library-empty">No tracks yet. Right-click any track to add.</div>
                <div v-else class="track-list">
                  <div
                    v-for="(track, idx) in playlistView.tracks"
                    :key="track.id"
                    class="track-row"
                    :class="[rarityClass(track.rarity), { 'track-row-current': isCurrentTrack(track.id), 'track-row-next': isNextTrack(track.id) }]"
                    :style="rarityVars(track.rarity)"
                    @click="playPlaylistTrack(playlistView!.tracks, idx)"
                    @contextmenu.prevent="openTrackContextMenu($event, track, playlistView!.id)"
                    @touchstart.passive="startTrackRowLongPress($event, track, playlistView!.id)"
                    @touchmove.passive="moveTrackRowLongPress"
                    @touchend="endTrackRowLongPress"
                    @touchcancel="endTrackRowLongPress"
                  >
                    <div class="track-cover-sm" :style="covers[track.id]
                      ? `background-image: url(${covers[track.id]}); background-size: cover; background-position: center`
                      : `background: linear-gradient(135deg, ${hashToColors(track.file_hash)[0]}, ${hashToColors(track.file_hash)[1]})`"
                    />
                    <div class="track-info">
                      <div class="track-title-row">
                        <span class="track-title">{{ track.title || track.path }}</span>
                        <span v-if="isCurrentTrack(track.id)" class="track-playback-badge current" title="Now playing">
                          <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M8 5v14l11-7z"/></svg>
                        </span>
                        <span v-else-if="isNextTrack(track.id)" class="track-playback-badge next" title="Up next">
                          <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M8 5v14l11-7z"/></svg>
                          <span class="track-playback-step">1</span>
                        </span>
                      </div>
                      <span class="track-album">{{ track.artist || 'Unknown' }}{{ track.album ? ' · ' + track.album : '' }}</span>
                    </div>
                    <button class="icon-btn track-inline-action" style="margin-right:8px;" title="Remove from playlist" @click.stop="removeTrackFromPlaylist(playlistView!.id, track.id)">
                      <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M19 13H5v-2h14v2z"/></svg>
                    </button>
                    <span class="track-dur">{{ formatDuration(track.duration_secs) }}</span>
                  </div>
                </div>
              </template>
              <!-- Playlist list -->
              <template v-else>
                <div class="library-header">
                  <h2>Playlists</h2>
                  <button class="icon-btn text-action-btn" style="padding:6px 12px;" @click="showNewPlaylistInput = !showNewPlaylistInput">+ New</button>
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
                  <button class="icon-btn" style="padding:6px 14px;" @click="createPlaylist">Create</button>
                </div>
                <div v-if="playlists.length === 0" class="library-empty">No playlists yet.</div>
                <div v-else class="track-list">
                  <div v-for="pl in playlists" :key="pl.id" class="track-row" style="cursor:pointer;" @click="openPlaylist(pl)">
                    <div class="track-cover-sm" style="background: linear-gradient(135deg, #333, #1a1a1a); display:flex; align-items:center; justify-content:center;">
                      <svg viewBox="0 0 24 24" fill="#a7a7a7" width="20" height="20"><path d="M15 6H3v2h12V6zm0 4H3v2h12v-2zM3 16h8v-2H3v2zM17 6v8.18c-.31-.11-.65-.18-1-.18-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3V8h3V6h-5z"/></svg>
                    </div>
                    <div class="track-info">
                      <span class="track-title">{{ pl.name }}</span>
                      <span class="track-album">{{ pl.track_count }} track{{ pl.track_count !== 1 ? 's' : '' }}</span>
                    </div>
                    <button class="icon-btn" :class="{ green: pl.pinned }" style="margin-right:4px;" :title="pl.pinned ? 'Unpin from Home' : 'Pin to Home'" @click.stop="togglePlaylistPinned(pl)">
                      <svg viewBox="0 0 24 24" fill="currentColor" width="15" height="15"><path d="M16 9V4l1-1V2H7v1l1 1v5l-2 2v1h5v8h2v-8h5v-1l-2-2z"/></svg>
                    </button>
                    <button class="icon-btn" style="margin-right:8px;" title="Delete playlist" @click.stop="deletePlaylist(pl.id)">
                      <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"/></svg>
                    </button>
                  </div>
                </div>
              </template>
            </template>

            <!-- ── Smart playlists ─────────────────────────────────────────── -->
            <template v-else>
              <!-- Editing a smart playlist -->
              <template v-if="editingSP">
                <div class="library-header">
                  <div style="display:flex; align-items:center; gap:12px;">
                    <button class="icon-btn" style="padding:4px;" @click="saveSP(); editingSP = null">
                      <svg viewBox="0 0 24 24" fill="currentColor" width="20" height="20"><path d="M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z"/></svg>
                    </button>
                    <input
                      class="library-search smart-name-input"
                      style="min-width:140px; max-width:260px;"
                      v-model="editingSP.name"
                      @input="saveSP()"
                      @click.stop
                    />
                  </div>
                  <span class="smart-track-count">{{ smartPlaylistTracks(editingSP).length }} tracks</span>
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
                    <select class="sp-select" :value="rule.field" @change="rule.field = ($event.target as HTMLSelectElement).value as SPField; onSPRuleFieldChange(rule)">
                      <option value="any">Any field</option>
                      <option value="title">Title</option>
                      <option value="artist">Artist</option>
                      <option value="album">Album</option>
                      <option value="genre">Genre</option>
                      <option value="tags">Tags</option>
                      <option value="rarity">Rarity</option>
                      <option value="path">Path</option>
                      <option value="extension">Extension</option>
                      <option value="track_number">Track #</option>
                      <option value="duration_secs">Duration</option>
                      <option value="year">Year</option>
                      <option value="play_count">Play count</option>
                      <option value="is_liked">Is liked</option>
                      <option value="date_added">Date added</option>
                      <option value="sort">Sort by</option>
                    </select>

                    <template v-if="spFieldType(rule.field) === 'text'">
                      <span class="sp-op-label">contains</span>
                      <input class="library-search sp-text-input" v-model="rule.value" placeholder="search…" @input="saveSP()" />
                    </template>
                    <template v-else-if="spFieldType(rule.field) === 'number'">
                      <select class="sp-select" :value="rule.op" @change="rule.op = ($event.target as HTMLSelectElement).value as SPOp; saveSP()">
                        <option value="gte">≥</option>
                        <option value="lte">≤</option>
                        <option value="eq">=</option>
                      </select>
                      <input class="library-search sp-num-input" type="number" min="0" v-model="rule.value" @input="saveSP()" />
                    </template>
                    <template v-else-if="spFieldType(rule.field) === 'bool'">
                      <button :class="['sp-match-btn', rule.op === 'is_true' && 'active']" @click="rule.op = 'is_true'; saveSP()">Yes</button>
                      <button :class="['sp-match-btn', rule.op === 'is_false' && 'active']" @click="rule.op = 'is_false'; saveSP()">No</button>
                    </template>
                    <template v-else-if="spFieldType(rule.field) === 'date'">
                      <select class="sp-select" :value="rule.op" @change="rule.op = ($event.target as HTMLSelectElement).value as SPOp; saveSP()">
                        <option value="gte">after</option>
                        <option value="lte">before</option>
                        <option value="eq">on</option>
                      </select>
                      <input class="library-search sp-text-input" type="date" v-model="rule.value" @change="saveSP()" />
                    </template>
                    <template v-else-if="spFieldType(rule.field) === 'sort'">
                      <span class="sp-op-label">by</span>
                      <select class="sp-select" :value="normalizeSPSortField(rule.value)" @change="rule.value = normalizeSPSortField(($event.target as HTMLSelectElement).value); saveSP()">
                        <option v-for="option in SP_SORT_FIELD_OPTIONS" :key="option.value" :value="option.value">{{ option.label }}</option>
                      </select>
                      <select class="sp-select" :value="normalizeSPSortOp(rule.op)" @change="rule.op = normalizeSPSortOp(($event.target as HTMLSelectElement).value); saveSP()">
                        <option value="sort_asc">ASC</option>
                        <option value="sort_desc">DESC</option>
                      </select>
                    </template>
                    <template v-else>
                      <div class="sp-multiselect">
                        <label v-for="v in spUniqueValues(rule.field)" :key="v" class="sp-chip" :class="{ selected: spIsSelected(rule, v) }" @click="spToggleValue(rule, v)">{{ v }}</label>
                        <span v-if="spUniqueValues(rule.field).length === 0" class="smart-empty-values">No values in library</span>
                      </div>
                    </template>

                    <button class="icon-btn" style="margin-left:auto; flex-shrink:0;" @click="removeSPRule(rule.id)" title="Remove rule">
                      <svg viewBox="0 0 24 24" fill="currentColor" width="14" height="14"><path d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/></svg>
                    </button>
                  </div>
                  <button class="sp-add-rule-btn" @click="addSPRule()">+ Add rule</button>
                </div>

                <!-- Preview -->
                <div class="sp-preview-header">Preview ({{ smartPlaylistTracks(editingSP).length }} tracks)</div>
                <div v-if="editingSP.rules.length === 0" class="library-empty">Add rules above to filter your library.</div>
                <div v-else-if="smartPlaylistTracks(editingSP).length === 0" class="library-empty">No tracks match the current rules.</div>
                <div v-else class="track-list sp-preview">
                  <div
                    v-for="track in smartPlaylistTracks(editingSP).slice(0, 50)"
                    :key="track.id"
                    class="track-row"
                    :class="[rarityClass(track.rarity), { 'track-row-current': isCurrentTrack(track.id), 'track-row-next': isNextTrack(track.id) }]"
                    :style="rarityVars(track.rarity)"
                    @click="playLibraryTrack(libraryFlatList.findIndex(t => t.id === track.id))"
                    @contextmenu.prevent="openTrackContextMenu($event, track)"
                    @touchstart.passive="startTrackRowLongPress($event, track)"
                    @touchmove.passive="moveTrackRowLongPress"
                    @touchend="endTrackRowLongPress"
                    @touchcancel="endTrackRowLongPress"
                  >
                    <div class="track-cover-sm" :style="covers[track.id]
                      ? `background-image: url(${covers[track.id]}); background-size: cover; background-position: center`
                      : `background: linear-gradient(135deg, ${hashToColors(track.file_hash)[0]}, ${hashToColors(track.file_hash)[1]})`"
                    />
                    <div class="track-info">
                      <div class="track-title-row">
                        <span class="track-title">{{ track.title || track.path }}</span>
                        <span v-if="isCurrentTrack(track.id)" class="track-playback-badge current" title="Now playing">
                          <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M8 5v14l11-7z"/></svg>
                        </span>
                        <span v-else-if="isNextTrack(track.id)" class="track-playback-badge next" title="Up next">
                          <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M8 5v14l11-7z"/></svg>
                          <span class="track-playback-step">1</span>
                        </span>
                      </div>
                      <span class="track-album">{{ track.artist || 'Unknown' }}{{ track.album ? ' · ' + track.album : '' }}{{ track.year ? ' · ' + track.year : '' }}{{ track.genre ? ' · ' + track.genre : '' }}</span>
                    </div>
                    <span class="track-dur">{{ formatDuration(track.duration_secs) }}</span>
                  </div>
                  <div v-if="smartPlaylistTracks(editingSP).length > 50" class="library-empty" style="padding:12px 0;">… and {{ smartPlaylistTracks(editingSP).length - 50 }} more</div>
                </div>
              </template>

              <!-- Viewing a smart playlist (read-only) -->
              <template v-else-if="smartView">
                <div class="library-header">
                  <div style="display:flex; align-items:center; gap:12px;">
                    <button class="icon-btn" style="padding:4px;" @click="smartView = null">
                      <svg viewBox="0 0 24 24" fill="currentColor" width="20" height="20"><path d="M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z"/></svg>
                    </button>
                    <h2 style="margin:0;">{{ smartView.name }}</h2>
                  </div>
                  <div style="display:flex;gap:8px;">
                    <button class="icon-btn text-action-btn" style="padding:6px 12px;" @click="editingSP = { ...smartView! }; smartView = null">Edit</button>
                    <button v-if="smartPlaylistTracks(smartView).length" class="icon-btn text-action-btn" style="padding:6px 12px;" @click="playFromPlaylist(smartPlaylistTracks(smartView!), 0)">▶ Play</button>
                  </div>
                </div>
                <div v-if="smartPlaylistTracks(smartView).length === 0" class="library-empty">No tracks match this smart playlist.</div>
                <div v-else class="track-list">
                  <div
                    v-for="(track, idx) in smartPlaylistTracks(smartView)"
                    :key="track.id"
                    class="track-row"
                    :class="[rarityClass(track.rarity), { 'track-row-current': isCurrentTrack(track.id), 'track-row-next': isNextTrack(track.id) }]"
                    :style="rarityVars(track.rarity)"
                    @click="playPlaylistTrack(smartPlaylistTracks(smartView!), idx)"
                    @contextmenu.prevent="openTrackContextMenu($event, track)"
                    @touchstart.passive="startTrackRowLongPress($event, track)"
                    @touchmove.passive="moveTrackRowLongPress"
                    @touchend="endTrackRowLongPress"
                    @touchcancel="endTrackRowLongPress"
                  >
                    <div class="track-cover-sm" :style="covers[track.id]
                      ? `background-image: url(${covers[track.id]}); background-size: cover; background-position: center`
                      : `background: linear-gradient(135deg, ${hashToColors(track.file_hash)[0]}, ${hashToColors(track.file_hash)[1]})`"
                    />
                    <div class="track-info">
                      <div class="track-title-row">
                        <span class="track-title">{{ track.title || track.path }}</span>
                        <span v-if="isCurrentTrack(track.id)" class="track-playback-badge current" title="Now playing">
                          <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M8 5v14l11-7z"/></svg>
                        </span>
                        <span v-else-if="isNextTrack(track.id)" class="track-playback-badge next" title="Up next">
                          <svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><path d="M8 5v14l11-7z"/></svg>
                          <span class="track-playback-step">1</span>
                        </span>
                      </div>
                      <span class="track-album">{{ track.artist || 'Unknown' }}{{ track.album ? ' · ' + track.album : '' }}{{ track.year ? ' · ' + track.year : '' }}{{ track.genre ? ' · ' + track.genre : '' }}</span>
                    </div>
                    <span class="track-dur">{{ formatDuration(track.duration_secs) }}</span>
                  </div>
                </div>
              </template>

              <!-- Smart playlist list -->
              <template v-else>
                <div class="library-header">
                  <h2>Flexible Playlists</h2>
                  <button class="icon-btn text-action-btn" style="padding:6px 12px;" @click="showNewSPInput = !showNewSPInput">+ New</button>
                </div>
                <div v-if="showNewSPInput" style="display:flex; gap:8px; padding: 0 0 14px;">
                  <input v-model="newSPName" class="library-search" placeholder="Smart playlist name…" style="flex:1;" @keydown.enter="createSmartPlaylist" @keydown.esc="showNewSPInput = false" />
                  <button class="icon-btn" style="padding:6px 14px;" @click="createSmartPlaylist">Create</button>
                </div>
                <div v-if="smartPlaylists.length === 0" class="library-empty">No flexible playlists yet.</div>
                <div v-else class="track-list">
                  <div v-for="sp in smartPlaylists" :key="sp.id" class="track-row" style="cursor:pointer;" @click="smartView = sp">
                    <div class="track-cover-sm" style="background: linear-gradient(135deg, #1a2a3a, #0d1b2a); display:flex; align-items:center; justify-content:center;">
                      <svg viewBox="0 0 24 24" fill="#4fc3f7" width="20" height="20"><path d="M19 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V5a2 2 0 0 0-2-2zm-7 3c1.93 0 3.5 1.57 3.5 3.5S13.93 13 12 13s-3.5-1.57-3.5-3.5S10.07 6 12 6zm7 13H5v-.23c0-.62.28-1.2.76-1.58C7.47 15.82 9.64 15 12 15s4.53.82 6.24 2.19c.48.38.76.97.76 1.58V19z"/></svg>
                    </div>
                    <div class="track-info">
                      <span class="track-title">{{ sp.name }}</span>
                      <span class="track-album">{{ sp.rules.length }} rule{{ sp.rules.length !== 1 ? 's' : '' }} · {{ smartPlaylistTracks(sp).length }} tracks</span>
                    </div>
                    <button class="icon-btn" :class="{ green: sp.pinned }" style="margin-right:4px;" :title="sp.pinned ? 'Unpin from Home' : 'Pin to Home'" @click.stop="toggleSmartPlaylistPinned(sp)">
                      <svg viewBox="0 0 24 24" fill="currentColor" width="15" height="15"><path d="M16 9V4l1-1V2H7v1l1 1v5l-2 2v1h5v8h2v-8h5v-1l-2-2z"/></svg>
                    </button>
                    <button class="icon-btn" style="margin-right:4px;" title="Edit" @click.stop="editingSP = { ...sp }; smartView = null">
                      <svg viewBox="0 0 24 24" fill="currentColor" width="14" height="14"><path d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04a1.003 1.003 0 0 0 0-1.42l-2.34-2.33a1.003 1.003 0 0 0-1.42 0l-1.83 1.83 3.75 3.75 1.84-1.83z"/></svg>
                    </button>
                    <button class="icon-btn" style="margin-right:8px;" title="Delete" @click.stop="deleteSmartPlaylist(sp.id)">
                      <svg viewBox="0 0 24 24" fill="currentColor" width="14" height="14"><path d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"/></svg>
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
            <div class="library-header">
              <h2>Duplicates</h2>
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
                  <svg viewBox="0 0 24 24" fill="currentColor" width="14" height="14"><path d="M17.65 6.35A7.958 7.958 0 0 0 12 4c-4.42 0-7.99 3.58-7.99 8s3.57 8 7.99 8c3.73 0 6.84-2.55 7.73-6h-2.08A5.99 5.99 0 0 1 12 18c-3.31 0-6-2.69-6-6s2.69-6 6-6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z"/></svg>
                  Rescan
                </button>
              </div>
            </div>

            <div v-if="dedupLoading" class="library-empty">Scanning library…</div>

            <div v-else-if="dedupError" class="settings-error dedup-error">{{ dedupError }}</div>

            <template v-else-if="dedupGroups.length === 0">
              <p class="search-empty-copy dedup-clean-copy">
                <svg viewBox="0 0 24 24" fill="#1db954" width="20" height="20"><path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-2 15l-5-5 1.41-1.41L10 14.17l7.59-7.59L19 8l-9 9z"/></svg>
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
                    <div class="track-cover-sm" :style="covers[track.id]
                      ? `background-image: url(${covers[track.id]}); background-size: cover; background-position: center`
                      : `background: linear-gradient(135deg, ${hashToColors(track.file_hash)[0]}, ${hashToColors(track.file_hash)[1]})`"
                    />
                    <div class="track-info">
                      <div class="track-title-row">
                        <span class="track-title">{{ track.title || track.path }}</span>
                        <span v-if="!dedupIsMarkedDuplicate(i, track.id)" class="dedup-keep-badge">Keep</span>
                        <span v-else class="dedup-remove-badge">Duplicate</span>
                      </div>
                      <span class="track-album">
                        {{ track.artist || 'Unknown' }}{{ track.album ? ' · ' + track.album : '' }}
                      </span>
                      <span class="dedup-track-meta">
                        <span>{{ track.path }}</span>
                        <span v-if="track.play_count > 0">▶ {{ track.play_count }}</span>
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
            <div class="library-header discovery-header">
              <div class="discovery-header-copy">
                <h2>Devices on Network</h2>
              </div>
              <div class="discovery-toolbar">
                <button class="sync-toggle" @click="openDeviceSettings" title="Device settings">
                  <span class="device-emoji">{{ deviceEmoji }}</span>
                  Settings
                </button>
                <button
                  class="sync-toggle"
                  :class="{ active: syncEnabled }"
                  @click="toggleSync"
                  :title="syncEnabled ? 'Sync on — click to disable' : 'Sync off — click to enable'"
                >
                  <svg viewBox="0 0 24 24" fill="currentColor" width="14" height="14"><path d="M12 4V1L8 5l4 4V6c3.31 0 6 2.69 6 6 0 1.01-.25 1.97-.7 2.8l1.46 1.46A7.93 7.93 0 0 0 20 12c0-4.42-3.58-8-8-8zm0 14c-3.31 0-6-2.69-6-6 0-1.01.25-1.97.7-2.8L5.24 7.74A7.93 7.93 0 0 0 4 12c0 4.42 3.58 8 8 8v3l4-4-4-4v3z"/></svg>
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
                        <svg v-if="peer.playback?.state === 'playing'" viewBox="0 0 24 24" fill="currentColor" width="14" height="14"><path d="M8 5v14l11-7z"/></svg>
                        <svg v-else-if="peer.playback?.state === 'paused'" viewBox="0 0 24 24" fill="currentColor" width="14" height="14"><path d="M6 5h4v14H6zm8 0h4v14h-4z"/></svg>
                        <svg v-else viewBox="0 0 24 24" fill="currentColor" width="14" height="14"><path d="M7 7h10v10H7z"/></svg>
                      </span>
                      {{ peerPlaybackLabel(peer) }}
                    </span>
                  </div>
                  <span class="track-album discovery-meta">
                    {{ peerNowPlayingText(peer) || 'No active playback' }} · {{ peer.host }}:{{ peer.port }}
                  </span>
                  <template v-if="syncProgress[peer.name]">
                    <div class="discovery-sync-block">
                      <div class="sync-bar-wrap" v-if="syncProgress[peer.name].phase === 'download'">
                        <div
                          class="sync-bar"
                          :style="{ width: syncProgress[peer.name].total > 0
                            ? (syncProgress[peer.name].done / syncProgress[peer.name].total * 100) + '%'
                            : '0%' }"
                        />
                      </div>
                      <span class="sync-status" :class="'sync-' + syncProgress[peer.name].phase">
                        <template v-if="syncProgress[peer.name].phase === 'index'">{{ syncProgress[peer.name].message || 'Fetching index...' }}</template>
                        <template v-else-if="syncProgress[peer.name].phase === 'download'">
                          Loading {{ syncProgress[peer.name].done }}/{{ syncProgress[peer.name].total }}
                        </template>
                        <template v-else-if="syncProgress[peer.name].phase === 'reindex'">Indexing</template>
                        <template v-else-if="syncProgress[peer.name].phase === 'done'">
                          {{ syncProgress[peer.name].message }}
                        </template>
                        <template v-else-if="syncProgress[peer.name].phase === 'error'">
                          {{ syncProgress[peer.name].message }}
                        </template>
                      </span>
                    </div>
                  </template>
                </div>
                <button
                  v-if="syncEnabled"
                  class="sync-now-btn discovery-sync-btn"
                  @click="syncPeer(peer)"
                  :disabled="['index','download','reindex'].includes(syncProgress[peer.name]?.phase ?? '')"
                  title="Sync now"
                >
                  <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M12 4V1L8 5l4 4V6c3.31 0 6 2.69 6 6 0 1.01-.25 1.97-.7 2.8l1.46 1.46A7.93 7.93 0 0 0 20 12c0-4.42-3.58-8-8-8zm0 14c-3.31 0-6-2.69-6-6 0-1.01.25-1.97.7-2.8L5.24 7.74A7.93 7.93 0 0 0 4 12c0 4.42 3.58 8 8 8v3l4-4-4-4v3z"/></svg>
                </button>
              </div>
            </div>
          </section>
        </template>

        <template v-else-if="activeNav === 'about'">
          <section>
            <div v-if="aboutLoading && !aboutInfo" class="library-empty">Loading About data…</div>
            <div v-else-if="aboutError && !aboutInfo" class="settings-error about-error">{{ aboutError }}</div>

            <template v-else>
              <div class="search-summary about-summary">
                <span class="about-version-line"><strong>Version</strong> v{{ aboutInfo?.current_version || 'n/a' }}</span>
                <button
                  class="sync-toggle"
                  :class="{ active: !!aboutUpdateStatus?.has_update }"
                  :title="aboutUpdateStatus?.has_update && aboutUpdateStatus.latest_version ? `Update available: v${aboutUpdateStatus.latest_version}` : 'Check GitHub Releases'"
                  :disabled="aboutCheckingUpdates"
                  @click="checkAboutUpdates()"
                >
                  {{ aboutCheckingUpdates ? 'Checking…' : 'Check updates' }}
                </button>
                <span v-if="aboutCheckingUpdates" class="about-update-inline">Checking updates…</span>
                <span v-else-if="aboutUpdateError" class="about-update-inline about-update-inline-error">{{ aboutUpdateError }}</span>
                <span v-else-if="aboutUpdateStatus" class="about-update-inline" :class="{ 'about-update-inline-live': aboutUpdateStatus.has_update }">
                  {{ aboutUpdateStatus.has_update && aboutUpdateStatus.latest_version
                    ? `Update available: v${aboutUpdateStatus.latest_version}`
                    : aboutUpdateStatus.latest_version
                      ? `No updates. Latest: v${aboutUpdateStatus.latest_version}`
                      : 'No updates' }}
                </span>
                <a
                  v-if="aboutUpdateStatus?.has_update && aboutUpdateStatus.release_url"
                  class="about-download-link"
                  href="#"
                  @click.prevent="openAboutDownloadLink()"
                >
                  Download v{{ aboutUpdateStatus.latest_version }}
                </a>
              </div>

              <h2>Changelog</h2>

              <div v-if="aboutInfo?.changelog.length">
                <div v-for="entry in aboutInfo!.changelog" :key="`${entry.short_hash}:${entry.committed_at}`">
                  {{ entry.subject }}
                </div>
              </div>
              <div v-else class="library-empty">No changelog entries were embedded into this build.</div>
            </template>
          </section>
        </template>
      </div>
    </main>

    <!-- Add to playlist menu -->
    <Teleport to="body">
      <div v-if="trackContextMenu" class="playlist-menu-backdrop" @click="trackContextMenu = null">
        <div
          class="playlist-menu track-context-menu"
          :style="{ top: trackContextMenu.y + 'px', left: trackContextMenu.x + 'px' }"
          @click.stop
        >
          <div class="playlist-menu-header">Track actions</div>
          <button class="playlist-menu-item" @click="toggleLikeFromTrackContext">{{ trackContextMenu.track.is_liked ? 'Unlike' : 'Like' }}</button>
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
        </div>
      </div>
    </Teleport>

    <Teleport to="body">
      <div v-if="homePinnedContextMenu" class="playlist-menu-backdrop" @click="homePinnedContextMenu = null">
        <div
          class="playlist-menu"
          :style="{ top: homePinnedContextMenu.y + 'px', left: homePinnedContextMenu.x + 'px' }"
          @click.stop
        >
          <div class="playlist-menu-header">{{ homePinnedContextMenu.item.kind === 'regular' ? 'Playlist actions' : 'Flexible playlist actions' }}</div>
          <button class="playlist-menu-item" @click="openHomePinnedFromContextMenu">Open</button>
          <button v-if="homePinnedContextMenu.item.trackCount > 0" class="playlist-menu-item" @click="playHomePinnedFromContextMenu">Play</button>
          <button v-if="homePinnedContextMenu.item.kind === 'smart'" class="playlist-menu-item" @click="editHomePinnedFromContextMenu">Edit rules</button>
          <button class="playlist-menu-item" @click="unpinHomePinnedFromContextMenu">Unpin from Home</button>
          <button class="playlist-menu-item danger" @click="deleteHomePinnedFromContextMenu">Delete</button>
        </div>
      </div>
    </Teleport>

    <Teleport to="body">
      <div v-if="addToPlaylistMenu" class="playlist-menu-backdrop" @click="addToPlaylistMenu = null">
        <div
          class="playlist-menu"
          :style="{ top: addToPlaylistMenu.y + 'px', left: addToPlaylistMenu.x + 'px' }"
          @click.stop
        >
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
        </div>
      </div>
    </Teleport>

    <!-- Dedup confirm modal -->
    <Transition name="modal">
      <div v-if="dedupConfirmOpen" class="modal-overlay" @click.self="dedupConfirmOpen = false">
        <div class="modal identify-modal">
          <div class="modal-header">
            <h3>Confirm marking</h3>
            <button class="icon-btn" @click="dedupConfirmOpen = false">
              <svg viewBox="0 0 24 24" fill="currentColor" width="18" height="18"><path d="M19 6.41 17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/></svg>
            </button>
          </div>
          <div class="modal-body">
            <p style="color:#b3b3b3; line-height:1.5;">
              <strong style="color:#fff;">{{ dedupMarkedTotal }}</strong>
              track(s) will be flagged as duplicates in the database. Files stay on disk. You can unmark them at any time.
            </p>
            <div v-if="dedupError" class="settings-error">{{ dedupError }}</div>
          </div>
          <div class="modal-footer">
            <button class="btn-secondary" @click="dedupConfirmOpen = false" :disabled="dedupApplying">Cancel</button>
            <button class="btn-primary" @click="applyDedup()" :disabled="dedupApplying">
              {{ dedupApplying ? 'Marking…' : 'Mark as duplicates' }}
            </button>
          </div>
        </div>
      </div>
    </Transition>

    <!-- Edit modal -->
    <Transition name="modal">
      <div v-if="editingTrack" class="modal-overlay" @click.self="editingTrack = null">
        <div class="modal edit-track-modal">
          <div class="modal-header">
            <h3>Edit Track</h3>
            <button class="icon-btn" @click="editingTrack = null">
              <svg viewBox="0 0 24 24" fill="currentColor" width="18" height="18"><path d="M19 6.41 17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/></svg>
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
              <span>Track #</span>
              <input v-model.number="editForm.track_number" type="number" min="0" />
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
              <select v-model="editForm.is_liked">
                <option :value="false">No</option>
                <option :value="true">Yes</option>
              </select>
            </label>
            <label class="field">
              <span>Date added</span>
              <input v-model="editForm.date_added" type="date" />
            </label>
            <label class="field">
              <span>Rarity</span>
              <select v-model="editForm.rarity">
                <option value="">Unset</option>
                <option v-for="rarity in TRACK_RARITY_OPTIONS" :key="rarity" :value="rarity">{{ rarity }}</option>
              </select>
            </label>
          </div>
          <div class="modal-footer">
            <button class="btn-secondary" @click="editingTrack = null">Cancel</button>
            <button class="btn-primary" @click="saveTrack">Save</button>
          </div>
        </div>
      </div>
    </Transition>

    <!-- Device settings modal -->
    <Transition name="modal">
      <div v-if="settingsOpen" class="modal-overlay" @click.self="settingsOpen = false">
        <div class="modal identify-modal">
          <div class="modal-header">
            <h3>Device settings</h3>
            <button class="icon-btn" title="Close" @click="settingsOpen = false">
              <svg viewBox="0 0 24 24" fill="currentColor" width="18" height="18"><path d="M19 6.41 17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/></svg>
            </button>
          </div>
          <div class="modal-body settings-body">
            <label class="field">
              <span>Device name</span>
              <input v-model="settingsDeviceName" placeholder="My Player" />
            </label>

            <div class="settings-label">Emoji</div>
            <div class="emoji-grid">
              <button
                v-for="emoji in EMOJI_OPTIONS"
                :key="emoji"
                class="emoji-cell"
                :class="{ active: settingsEmoji === emoji }"
                @click="settingsEmoji = emoji"
              >{{ emoji }}</button>
            </div>

            <div v-if="settingsError" class="settings-error">{{ settingsError }}</div>
          </div>
          <div class="modal-footer">
            <button class="btn-secondary" @click="settingsOpen = false">Cancel</button>
            <button class="btn-primary" :disabled="settingsSaving" @click="saveDeviceSettings">
              {{ settingsSaving ? 'Saving...' : 'Save' }}
            </button>
          </div>
        </div>
      </div>
    </Transition>

    <Transition name="modal">
      <div v-if="soulseekSettingsOpen" class="modal-overlay" @click.self="soulseekSettingsOpen = false">
        <div class="modal identify-modal">
          <div class="modal-header">
            <h3>Soulseek settings</h3>
            <button class="icon-btn" title="Close" @click="soulseekSettingsOpen = false">
              <svg viewBox="0 0 24 24" fill="currentColor" width="18" height="18"><path d="M19 6.41 17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/></svg>
            </button>
          </div>
          <div class="modal-body settings-body">
            <div class="settings-toggle-row">
              <div>
                <span class="settings-toggle-title">Soulseek search</span>
                <span class="settings-toggle-copy">Search the Soulseek network and save selected files directly into your local library.</span>
              </div>
              <button
                type="button"
                class="sync-toggle settings-toggle-action"
                :class="{ active: settingsSoulseekEnabled }"
                :aria-pressed="settingsSoulseekEnabled"
                :title="settingsSoulseekEnabled ? 'Soulseek search on — click to disable' : 'Soulseek search off — click to enable'"
                @click="settingsSoulseekEnabled = !settingsSoulseekEnabled"
              >
                <svg viewBox="0 0 24 24" fill="currentColor" width="14" height="14"><path d="M15.5 14h-.79l-.28-.27A6.47 6.47 0 0 0 16 9.5 6.5 6.5 0 1 0 9.5 16a6.47 6.47 0 0 0 4.23-1.57l.27.28v.79L20 21.5 21.5 20l-6-6zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z"/></svg>
                Search
              </button>
            </div>

            <template v-if="settingsSoulseekEnabled">
              <label class="field">
                <span>Soulseek username</span>
                <input v-model="settingsSoulseekUsername" autocomplete="username" placeholder="username" />
              </label>

              <label class="field">
                <span>Soulseek password</span>
                <input v-model="settingsSoulseekPassword" type="password" autocomplete="current-password" placeholder="password" />
              </label>

              <p class="settings-note">Downloaded Soulseek files are placed into the Soulseek folder inside your library.</p>
            </template>

            <div v-if="soulseekSettingsError" class="settings-error">{{ soulseekSettingsError }}</div>
          </div>
          <div class="modal-footer">
            <button class="btn-secondary" @click="soulseekSettingsOpen = false">Cancel</button>
            <button class="btn-primary" :disabled="soulseekSettingsSaving" @click="saveSoulseekSettings">
              {{ soulseekSettingsSaving ? 'Saving...' : 'Save' }}
            </button>
          </div>
        </div>
      </div>
    </Transition>

    <!-- Identify progress mini indicator (top-right when minimized) -->
    <div class="status-pills">
    <!-- Index progress pill (clickable to open log) -->
      <div v-if="indexRunning" class="identify-mini" :class="{ 'index-done': indexDone }" @click="indexLogOpen = true">
        <div v-if="!indexDone" class="identify-mini-spinner" />
        <svg v-else viewBox="0 0 24 24" fill="#1db954" width="16" height="16"><path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-2 15l-5-5 1.41-1.41L10 14.17l7.59-7.59L19 8l-9 9z"/></svg>
        <span v-if="!indexDone">{{ indexCurrent }}/{{ indexTotal }}</span>
        <span v-else>+{{ indexAdded }}</span>
      </div>
      <!-- Identify progress pill -->
      <div v-if="identifyRunning && identifyMinimized" class="identify-mini" @click="identifyMinimized = false">
        <div v-if="!identifyDone" class="identify-mini-spinner" />
        <span>{{ identifyCurrent }}/{{ identifyTotal }}</span>
        <span v-if="identifyDone" class="identify-mini-done">✓</span>
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
                <svg viewBox="0 0 24 24" fill="currentColor" width="18" height="18"><path d="M6 19h12v2H6z"/></svg>
              </button>
              <button v-if="identifyDone" class="icon-btn" title="Close" @click="identifyRunning = false">
                <svg viewBox="0 0 24 24" fill="currentColor" width="18" height="18"><path d="M19 6.41 17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/></svg>
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
              <svg viewBox="0 0 24 24" fill="currentColor" width="18" height="18"><path d="M19 6.41 17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/></svg>
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
                  <span class="ls-icon">
                    <span v-if="session.kind === 'local'">💿</span>
                    <span v-else>{{ session.emoji || '📱' }}</span>
                  </span>
                  <span class="ls-source">
                    <template v-if="session.kind === 'local'">Local library</template>
                    <template v-else>{{ session.device || 'Unknown device' }}</template>
                  </span>
                  <span class="ls-badge" v-if="session.status === 'running'">
                    <span class="ls-spinner" />
                    <template v-if="session.kind === 'sync' && session.filesAdded > 0">{{ session.filesAdded }} files…</template>
                    <template v-else>scanning…</template>
                  </span>
                  <span class="ls-badge ls-badge-done" v-else-if="session.status === 'done'">
                    <svg viewBox="0 0 24 24" fill="#1db954" width="12" height="12"><path d="M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z"/></svg>
                    <template v-if="session.filesAdded > 0">+{{ session.filesAdded }} files</template>
                    <template v-else>up to date</template>
                  </span>
                  <span class="ls-badge ls-badge-err" v-else>
                    <svg viewBox="0 0 24 24" fill="#ff5252" width="12" height="12"><path d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/></svg>
                    error
                  </span>
                  <span class="ls-time">{{ new Date(session.finishedAt || session.startedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) }}</span>
                </div>
                <!-- Error detail -->
                <div v-if="session.status === 'error' && session.errorMsg" class="ls-error-msg">{{ session.errorMsg }}</div>
                <!-- File list -->
                <div v-if="session.files.length" class="ls-files">
                  <div v-for="(f, i) in session.files" :key="i" class="ls-file-item">
                    <span class="ls-file-icon">♪</span>{{ f }}
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
    <footer class="player">
      <!-- Left: track info -->
      <div class="player-left" @click="nowPlaying && (showDetail = true)" style="cursor: pointer;">
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
        <button class="icon-btn" :class="{ green: isLiked }" @click="isLiked = !isLiked">
          <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"/></svg>
        </button>
      </div>

      <!-- Center: controls -->
      <div class="player-center">
        <div class="ctrl-row" :style="{ transform: `scale(${beatScale})`, transformOrigin: 'center', willChange: 'transform' }">
          <button class="icon-btn" :class="{ green: isShuffled, dot: isShuffled }" @click="isShuffled = !isShuffled" title="Shuffle">
            <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M10.59 9.17L5.41 4 4 5.41l5.17 5.17 1.42-1.41zM14.5 4l2.04 2.04L4 18.59 5.41 20 17.96 7.46 20 9.5V4h-5.5zm.33 9.41l-1.41 1.41 3.13 3.13L14.5 20H20v-5.5l-2.04 2.04-3.13-3.13z"/></svg>
          </button>
          <button class="icon-btn" title="Previous" @click="playPrev">
            <svg viewBox="0 0 24 24" fill="currentColor" width="20" height="20"><path d="M6 6h2v12H6zm3.5 6 8.5 6V6z"/></svg>
          </button>
          <button class="play-btn" @click="togglePlay">
            <svg v-if="!isPlaying" viewBox="0 0 24 24" fill="currentColor" width="20" height="20"><path d="M8 5v14l11-7z"/></svg>
            <svg v-else viewBox="0 0 24 24" fill="currentColor" width="20" height="20"><path d="M6 19h4V5H6v14zm8-14v14h4V5h-4z"/></svg>
          </button>
          <button class="icon-btn" title="Next" @click="playNext">
            <svg viewBox="0 0 24 24" fill="currentColor" width="20" height="20"><path d="M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z"/></svg>
          </button>
          <button class="icon-btn" :class="{ green: repeatMode > 0, dot: repeatMode > 0 }" @click="repeatMode = (repeatMode + 1) % 3" title="Repeat">
            <svg v-if="repeatMode < 2" viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4z"/></svg>
            <svg v-else viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4zm-4-2v-5h-1l-2 1v1h1.5v3H13z"/></svg>
          </button>
        </div>
        <div class="progress-row">
          <span class="time">{{ formatTime(currentTime) }}</span>
          <div class="bar" @click="seek">
            <div class="bar-fill" :style="`width:${displayProgressPercent}%`">
              <div class="bar-thumb" />
            </div>
          </div>
          <span class="time">{{ formatTime(duration) }}</span>
        </div>
      </div>

      <!-- Right: extras -->
      <div class="player-right">
        <div class="queue-menu-wrapper">
          <button class="icon-btn" title="Queue" @click.stop="showQueueMenu = !showQueueMenu">
            <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M3 13h2v-2H3v2zm0 4h2v-2H3v2zm0-8h2V7H3v2zm4 4h14v-2H7v2zm0 4h14v-2H7v2zM7 7v2h14V7H7z"/></svg>
          </button>
          <Transition name="dropdown">
            <div v-if="showQueueMenu" class="dropdown queue-dropdown">
              <div class="dropdown-header">Queue · {{ queueSource === 'recent' ? 'Recently played' : 'Library' }}</div>
              <div class="queue-now" v-if="nowPlaying">
                <span class="queue-now-label">Now playing</span>
                <div class="queue-item active">
                  <div class="queue-item-cover" :style="covers[nowPlaying.id]
                    ? `background-image: url(${covers[nowPlaying.id]}); background-size: cover; background-position: center`
                    : `background: linear-gradient(135deg, ${hashToColors(nowPlaying.file_hash)[0]}, ${hashToColors(nowPlaying.file_hash)[1]})`" />
                  <div class="queue-item-info">
                    <span class="queue-item-title">{{ nowPlaying.title || nowPlaying.path }}</span>
                    <span class="queue-item-artist">{{ nowPlaying.artist || 'Unknown' }}</span>
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
                  @click="jumpToQueueItem(i)"
                >
                  <div class="queue-item-cover" :style="covers[track.id]
                    ? `background-image: url(${covers[track.id]}); background-size: cover; background-position: center`
                    : `background: linear-gradient(135deg, ${hashToColors(track.file_hash)[0]}, ${hashToColors(track.file_hash)[1]})`" />
                  <div class="queue-item-info">
                    <span class="queue-item-title">{{ track.title || track.path }}</span>
                    <span class="queue-item-artist">{{ track.artist || 'Unknown' }}</span>
                  </div>
                  <span class="queue-item-dur">{{ formatDuration(track.duration_secs) }}</span>
                </div>
              </div>
              <div v-else class="queue-empty">Queue is empty</div>
            </div>
          </Transition>
        </div>
        <div class="device-menu-wrapper">
          <button class="icon-btn" title="Output device" @click.stop="toggleDeviceMenu">
            <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M17 2H7c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h10c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zM12 20c-1.66 0-3-1.34-3-3s1.34-3 3-3 3 1.34 3 3-1.34 3-3 3zm5-12H7V5h10v3z"/></svg>
          </button>
          <Transition name="dropdown">
            <div v-if="showDeviceMenu" class="dropdown device-dropdown">
              <div class="dropdown-header">Output device</div>
              <div class="device-list">
                <a
                  v-for="dev in outputDevices"
                  :key="dev.name"
                  href="#"
                  class="device-item"
                  @click.prevent="pickLocalDevice(dev.name)"
                >
                  <span class="device-check">{{ !remoteOutputPeer && dev.name === currentDevice ? '✓' : '' }}</span>
                  <span class="device-name">{{ dev.name }}</span>
                </a>
                <div v-if="remoteOutputDevices.length" class="device-section-label">Player devices</div>
                <a
                  v-for="peer in remoteOutputDevices"
                  :key="`${peer.host}:${peer.port}`"
                  href="#"
                  class="device-item"
                  @click.prevent="pickRemoteDevice(peer)"
                >
                  <span class="device-check">{{ isRemoteOutputPeer(peer) ? '✓' : '' }}</span>
                  <span class="device-name">{{ peerLabel(peer) }}</span>
                </a>
              </div>
              <div v-if="deviceMenuError" class="device-error">{{ deviceMenuError }}</div>
            </div>
          </Transition>
        </div>
        <div class="vol-wrap">
          <button class="icon-btn">
            <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16">
              <path v-if="volume > 0" d="M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z"/>
              <path v-else d="M16.5 12c0-1.77-1.02-3.29-2.5-4.03v2.21l2.45 2.45c.03-.2.05-.41.05-.63zm2.5 0c0 .94-.2 1.82-.54 2.64l1.51 1.51C20.63 14.91 21 13.5 21 12c0-4.28-2.99-7.86-7-8.77v2.06c2.89.86 5 3.54 5 6.71zM4.27 3L3 4.27 7.73 9H3v6h4l5 5v-6.73l4.25 4.25c-.67.52-1.42.93-2.25 1.18v2.06c1.38-.31 2.63-.95 3.69-1.81L19.73 21 21 19.73l-9-9L4.27 3zM12 4L9.91 6.09 12 8.18V4z"/>
            </svg>
          </button>
          <div class="bar vol" @click="setVolume">
            <div class="bar-fill" :style="`width:${volume}%`">
              <div class="bar-thumb" />
            </div>
          </div>
        </div>
      </div>
    </footer>
  </div>

  <!-- Player detail overlay — teleported to body to escape footer stacking context -->
  <Teleport to="body">
    <Transition name="detail">
      <div v-if="showDetail && nowPlaying" class="player-detail" @click.self="showDetail = false">
          <div class="detail-backdrop" aria-hidden="true">
            <div class="detail-backdrop-image" :style="detailBackdropImageStyle" />
            <div class="detail-backdrop-wash" :style="detailBackdropWashStyle" />
          </div>
          <div class="detail-sheet" :style="cardDragging ? { overflow: 'visible' } : {}">
            <!-- drag handle -->
            <div class="detail-handle" @click="showDetail = false" />
            <!-- cover 3D card -->
            <div class="detail-cover-wrap"
              :style="{
                ...(cardDragging ? { zIndex: 200, position: 'relative' } : {}),
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
                  :cover-url="currentCoverUrl"
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
                <div class="detail-spectrum-ring" :class="{ playing: isPlaying }" :style="detailSpectrumStyle" aria-hidden="true">
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
                <svg v-if="nowPlaying.is_liked" viewBox="0 0 24 24" fill="#1db954" width="22" height="22"><path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"/></svg>
                <svg v-else viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M16.5 3c-1.74 0-3.41.81-4.5 2.09C10.91 3.81 9.24 3 7.5 3 4.42 3 2 5.42 2 8.5c0 3.78 3.4 6.86 8.55 11.54L12 21.35l1.45-1.32C18.6 15.36 22 12.28 22 8.5 22 5.42 19.58 3 16.5 3zm-4.4 15.55l-.1.1-.1-.1C7.14 14.24 4 11.39 4 8.5 4 6.5 5.5 5 7.5 5c1.54 0 3.04.99 3.57 2.36h1.87C13.46 5.99 14.96 5 16.5 5c2 0 3.5 1.5 3.5 3.5 0 2.89-3.14 5.74-7.9 10.05z"/></svg>
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
                <svg viewBox="0 0 24 24" fill="currentColor" width="20" height="20"><path d="M10.59 9.17L5.41 4 4 5.41l5.17 5.17 1.42-1.41zM14.5 4l2.04 2.04L4 18.59 5.41 20 17.96 7.46 20 9.5V4h-5.5zm.33 9.41l-1.41 1.41 3.13 3.13L14.5 20H20v-5.5l-2.04 2.04-3.13-3.13z"/></svg>
              </button>
              <button class="icon-btn" @click="playPrev">
                <svg viewBox="0 0 24 24" fill="currentColor" width="32" height="32"><path d="M6 6h2v12H6zm3.5 6 8.5 6V6z"/></svg>
              </button>
              <button class="detail-play-btn" @click="togglePlay">
                <svg v-if="!isPlaying" viewBox="0 0 24 24" fill="currentColor" width="28" height="28"><path d="M8 5v14l11-7z"/></svg>
                <svg v-else viewBox="0 0 24 24" fill="currentColor" width="28" height="28"><path d="M6 19h4V5H6v14zm8-14v14h4V5h-4z"/></svg>
              </button>
              <button class="icon-btn" @click="playNext">
                <svg viewBox="0 0 24 24" fill="currentColor" width="32" height="32"><path d="M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z"/></svg>
              </button>
              <button class="icon-btn" :class="{ green: repeatMode > 0 }" @click="repeatMode = (repeatMode + 1) % 3">
                <svg v-if="repeatMode < 2" viewBox="0 0 24 24" fill="currentColor" width="20" height="20"><path d="M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4z"/></svg>
                <svg v-else viewBox="0 0 24 24" fill="currentColor" width="20" height="20"><path d="M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4zm-4-2v-5h-1l-2 1v1h1.5v3H13z"/></svg>
              </button>
            </div>
          </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style>
*, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
html, body, #app { height: 100%; overflow: hidden; }
body { background: #000; font-family: 'Helvetica Neue', Helvetica, Arial, sans-serif; }
a, button, [role="button"] {
  -webkit-tap-highlight-color: transparent;
  outline: none;
}
</style>

<style scoped>
/* ── Layout ── */
.app {
  display: grid;
  grid-template-columns: 220px 1fr;
  grid-template-rows: 1fr 90px;
  height: 100vh;
  color: #fff;
  background: #000;
  --fs-nav: 14px;
  --fs-h2: 22px;
  --fs-section-link: 11px;
  --fs-card-title: 13px;
  --fs-card-meta: 12px;
  --fs-player-title: 13px;
  --fs-player-meta: 11px;
  --fs-input: 13px;
  --fs-empty: 14px;
  --fs-eyebrow: 11px;
  --fs-group: 13px;
  --fs-track-side: 13px;
  --fs-track-title: 14px;
  --fs-track-meta: 12px;
  --fs-badge: 11px;
  --fs-modal-title: 18px;
  --fs-field-label: 12px;
  --fs-field-input: 14px;
  --fs-button: 13px;
  --fs-dropdown-label: 12px;
  --fs-dropdown-item: 14px;
  --fs-queue-title: 13px;
  --fs-queue-meta: 11px;
  --fs-queue-dur: 12px;
  --fs-status-pill: 13px;
  --fs-powered-by: 11px;
  --fs-body-sm: 12px;
  --fs-body-md: 13px;
  --fs-peer-title: 14px;
  --fs-peer-meta: 12px;
  --fs-control: 12px;
  --fs-detail-title: 18px;
  --fs-detail-artist: 14px;
  --fs-detail-time: 12px;
}

/* ── Sidebar ── */
.sidebar {
  grid-row: 1 / 2;
  background: #000;
  padding: 20px 8px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  overflow-y: auto;
}
.sidebar::-webkit-scrollbar { display: none; }

.brand {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 12px;
  margin-bottom: 14px;
  cursor: pointer;
}
.brand-name { font-size: 20px; font-weight: 800; letter-spacing: -0.5px; }

.nav-item {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 9px 12px;
  border-radius: 6px;
  color: #b3b3b3;
  text-decoration: none;
  font-size: var(--fs-nav);
  font-weight: 600;
  cursor: pointer;
  transition: color .12s;
  user-select: none;
  position: relative;
}
.nav-item:hover { color: #fff; }
.nav-item.active { color: #fff; }

.peer-badge {
  margin-left: auto;
  background: #1db954;
  color: #000;
  font-size: var(--fs-eyebrow); font-weight: 700;
  min-width: 18px; height: 18px;
  border-radius: 9px;
  display: flex; align-items: center; justify-content: center;
  padding: 0 5px;
}

.discovery-header {
  align-items: flex-end;
}
.discovery-header-copy {
  display: flex;
  flex-direction: column;
  gap: 6px;
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
  color: #fff;
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
  background: rgba(255, 255, 255, 0.08);
  color: #d7d7d7;
  flex-shrink: 0;
}
.discovery-status-pill.peer-status-playing { background: rgba(29, 185, 84, 0.16); color: #8de2a7; }
.discovery-status-pill.peer-status-paused { background: rgba(243, 201, 105, 0.16); color: #f3c969; }
.discovery-status-pill.peer-status-stopped,
.discovery-status-pill.peer-status-ended,
.discovery-status-pill.peer-status-idle { background: rgba(255, 255, 255, 0.06); color: #9a9a9a; }
.discovery-meta {
  white-space: normal;
}
.discovery-sync-block {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 2px;
  max-width: 420px;
}
.discovery-output-pill {
  flex-shrink: 0;
}

.sync-toggle {
  display: flex; align-items: center; gap: 6px;
  padding: 5px 12px; border-radius: 20px; border: 1px solid #535353;
  background: transparent; color: #a7a7a7; font-size: var(--fs-control); font-weight: 600;
  cursor: pointer; transition: all 0.15s;
}
.sync-toggle:hover { border-color: #fff; color: #fff; }
.sync-toggle.active { background: #1db954; border-color: #1db954; color: #000; }

.settings-body { gap: 12px; }
.settings-label { font-size: var(--fs-field-label); font-weight: 700; color: #a7a7a7; margin-top: 2px; }
.settings-toggle-row {
  padding: 0;
  border: none;
  background: none;
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
}
.settings-toggle-title {
  display: block;
  color: #fff;
  font-size: var(--fs-body-md);
  font-weight: 700;
}
.settings-toggle-copy {
  display: block;
  margin-top: 4px;
  color: #a7a7a7;
  font-size: var(--fs-body-sm);
  line-height: 1.45;
}
.settings-toggle-action {
  flex-shrink: 0;
  margin-top: 2px;
}
.settings-note {
  margin: -4px 0 0;
  color: #a7a7a7;
  font-size: var(--fs-body-sm);
  line-height: 1.45;
}
.emoji-grid {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  gap: 8px;
}
.emoji-cell {
  border: 1px solid #3a3a3a;
  border-radius: 8px;
  height: 44px;
  background: #181818;
  color: #fff;
  font-size: 24px;
  cursor: pointer;
}
.emoji-cell:hover { border-color: #777; background: #242424; }
.emoji-cell.active { border-color: #1db954; box-shadow: 0 0 0 1px #1db954 inset; }
.settings-error { color: #e9283e; font-size: var(--fs-body-sm); }

/* ── Duplicates view ── */
.dedup-toolbar {
  display: flex; gap: 8px; align-items: center; flex-wrap: wrap;
}
.dedup-filter-btn {
  padding: 5px 12px; border-radius: 20px; border: 1px solid #333;
  background: transparent; color: #a7a7a7; cursor: pointer;
  font-size: var(--fs-body-sm); transition: background 0.12s, color 0.12s, border-color 0.12s;
}
.dedup-filter-btn:hover { border-color: #555; color: #fff; }
.dedup-filter-btn.active { background: #1db954; color: #000; border-color: #1db954; }
.dedup-rescan-btn {
  display: flex; align-items: center; gap: 6px;
  padding: 5px 14px; font-size: var(--fs-body-sm);
}
.dedup-summary {
  margin: 10px 0 18px; color: #b3b3b3; font-size: var(--fs-body-sm);
}
.dedup-clean-copy {
  display: flex; align-items: center; gap: 8px; color: #a7a7a7;
}
.dedup-error { margin-top: 16px; }
.dedup-group {
  margin-bottom: 20px; border: 1px solid #282828; border-radius: 8px; overflow: hidden;
}
.dedup-group-header {
  padding: 10px 16px 8px; background: #1a1a1a;
  display: flex; align-items: flex-start; gap: 12px; flex-wrap: wrap;
}
.dedup-group-title {
  font-size: var(--fs-body-sm); color: #fff; font-weight: 600;
  display: flex; align-items: center; gap: 8px;
}
.dedup-track-count {
  font-size: 11px; color: #a7a7a7; font-weight: 400;
}
.dedup-reasons {
  display: flex; gap: 6px; flex-wrap: wrap; margin-left: auto;
}
.dedup-reason-tag {
  padding: 2px 8px; border-radius: 10px; background: #282828;
  font-size: 10px; color: #888; font-family: monospace;
}
.dedup-group-tracks { padding: 4px 0; }
.dedup-track-row {
  cursor: pointer;
}
.dedup-track-row.dedup-keep {
  background: rgba(29, 185, 84, 0.08);
}
.dedup-track-row.dedup-remove {
  opacity: 0.55;
}
.dedup-track-meta {
  display: flex; gap: 10px; font-size: 11px; color: #666; margin-top: 2px;
  flex-wrap: wrap;
}
.dedup-edited-tag {
  background: #2c3a2e; color: #1db954; padding: 1px 6px; border-radius: 4px;
  font-size: 10px;
}
.dedup-already-marked {
  outline: 1px solid #7b4a1a44;
  background: rgba(245, 160, 30, 0.05);
}
.dedup-flagged-tag {
  background: #3a2e1a; color: #f5a01e; padding: 1px 6px; border-radius: 4px;
  font-size: 10px;
}
.dedup-keep-badge {
  background: #1db954; color: #000; font-size: 10px; font-weight: 700;
  padding: 1px 6px; border-radius: 4px; margin-left: 8px;
}
.dedup-remove-badge {
  background: #c0392b22; color: #e05a4a; font-size: 10px;
  padding: 1px 6px; border-radius: 4px; margin-left: 8px;
}
.dedup-keep-radio {
  flex-shrink: 0; display: flex; align-items: center; justify-content: center;
  width: 24px; margin-left: 4px;
}
.dedup-radio-dot {
  width: 14px; height: 14px; border-radius: 50%; border: 2px solid #555;
  transition: border-color 0.12s, background 0.12s;
}
.dedup-radio-dot.active {
  border-color: #1db954; background: #1db954;
}
.dedup-actions {
  margin-top: 24px; margin-bottom: 12px; display: flex; justify-content: flex-end;
}

.sync-now-btn {
  flex-shrink: 0; width: 32px; height: 32px; border-radius: 50%;
  background: #282828; border: none; color: #a7a7a7;
  display: flex; align-items: center; justify-content: center;
  cursor: pointer; transition: background 0.15s, color 0.15s;
}
.sync-now-btn:hover:not(:disabled) { background: #1db954; color: #000; }
.sync-now-btn:disabled { opacity: 0.4; cursor: default; }
.discovery-sync-btn { margin-left: 10px; }

.sync-bar-wrap {
  height: 3px; background: #333; border-radius: 2px; overflow: hidden; margin-top: 2px;
}
.sync-bar { height: 100%; background: #1db954; border-radius: 2px; transition: width 0.3s; }
.sync-status { font-size: var(--fs-powered-by); color: #a7a7a7; }
.sync-done { color: #1db954; }
.sync-error { color: #e9283e; }

.icon-box {
  width: 26px; height: 26px;
  border-radius: 3px;
  display: flex; align-items: center; justify-content: center;
  font-size: 14px; font-weight: 800;
  flex-shrink: 0;
}
.icon-box.create { background: #b3b3b3; color: #000; }
.icon-box.liked  { background: linear-gradient(135deg, #450af5, #c4efd9); }

.sidebar-divider { height: 1px; background: #282828; margin: 8px 12px; }

/* ── Main area ── */
.main {
  grid-row: 1 / 2;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: linear-gradient(180deg, #4a2d8a 0%, #1f1b3a 30%, #121212 58%);
}

.topbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 24px;
  padding-top: calc(16px + env(safe-area-inset-top));
  flex-shrink: 0;
}

.nav-arrows { display: flex; gap: 8px; }
.arrow-btn {
  width: 32px; height: 32px;
  border-radius: 50%;
  background: rgba(0,0,0,.45);
  border: none; color: #fff;
  font-size: 22px; line-height: 1;
  cursor: pointer;
  display: flex; align-items: center; justify-content: center;
}
.arrow-btn:hover { background: rgba(0,0,0,.7); }

.duplicates-toggle {
  border: 1px solid #4a4a4a;
  background: rgba(20, 20, 20, 0.75);
  color: #d9dee8;
  font-size: 12px;
  font-weight: 700;
  letter-spacing: 0.02em;
  padding: 7px 12px;
  border-radius: 999px;
  cursor: pointer;
  transition: background .15s, border-color .15s, color .15s;
}

.duplicates-toggle:hover {
  background: rgba(36, 36, 36, 0.92);
}

.duplicates-toggle.active {
  background: rgba(29, 185, 84, 0.16);
  border-color: rgba(29, 185, 84, 0.7);
  color: #8cf7b0;
}

.user-menu-wrapper { position: relative; }
.user-btn {
  display: flex; align-items: center; gap: 8px;
  background: rgba(0,0,0,.5);
  border: none; color: #fff;
  padding: 4px 10px 4px 4px;
  border-radius: 20px;
  cursor: pointer; font-size: 13px; font-weight: 700;
}
.user-btn:hover { background: rgba(0,0,0,.8); }
.avatar {
  width: 28px; height: 28px; border-radius: 50%;
  background: #5a5a5a;
  display: flex; align-items: center; justify-content: center;
  font-size: 13px; font-weight: 700; text-transform: uppercase;
}

.dropdown {
  position: absolute; top: calc(100% + 8px); right: 0;
  background: #282828; border-radius: 4px; padding: 4px 0;
  min-width: 190px;
  box-shadow: 0 16px 32px rgba(0,0,0,.5);
  z-index: 200;
}
.dropdown a {
  display: flex; align-items: center; justify-content: space-between;
  padding: 10px 16px;
  color: #fff; text-decoration: none; font-size: 14px; cursor: pointer;
}
.dropdown a:hover { background: #3e3e3e; }
.dropdown-sep { height: 1px; background: #3e3e3e; margin: 4px 0; }

.dropdown-enter-active, .dropdown-leave-active { transition: opacity .12s, transform .12s; }
.dropdown-enter-from, .dropdown-leave-to { opacity: 0; transform: translateY(-4px); }

/* ── Content ── */
.content {
  flex: 1; overflow-y: auto;
  padding: 0 24px 32px;
}
.content::-webkit-scrollbar { width: 8px; }
.content::-webkit-scrollbar-thumb { background: #555; border-radius: 4px; }
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
  color: #b3b3b3; text-decoration: none;
}
.show-all:hover { color: #fff; text-decoration: underline; }

.grid {
  display: grid;
  grid-template-columns: repeat(6, 1fr);
  gap: 16px;
}
.card-list {
  width: 100%;
  display: flex;
  overflow-y: auto;
  scrollbar-width: none;
  gap: 16px;
}

.card {
  background: #181818; border-radius: 6px; padding: 14px;
  cursor: pointer; transition: background .18s; position: relative;
  width: 150px;
  flex-shrink: 0;
}
.card:hover { background: #282828 !important; }
.card.rarity-tint { background: color-mix(in srgb, var(--rc) 12%, #181818); }
.card:hover .hover-play { opacity: 1; transform: translateY(0); }

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
  background: #1db954; border: none;
  display: flex; align-items: center; justify-content: center;
  cursor: pointer;
  box-shadow: 0 8px 24px rgba(0,0,0,.5);
  transition: transform .1s, background .1s;
}
.green-circle:hover { transform: scale(1.06); background: #1ed760; }

.card-title {
  font-size: var(--fs-card-title); font-weight: 700;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  margin-bottom: 4px;
}
.card-artist {
  font-size: var(--fs-card-meta); color: #a7a7a7;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}

/* ── Player bar ── */
.player {
  grid-column: 1 / 3; grid-row: 2 / 3;
  background: #181818;
  border-top: 1px solid #282828;
  display: grid;
  grid-template-columns: 1fr 2fr 1fr;
  align-items: center;
  padding: 0 16px;
  overflow: visible;
  position: relative;
  z-index: 100;
}

.player-left { display: flex; align-items: center; gap: 12px; min-width: 0; }
.thumb { width: 56px; height: 56px; border-radius: 4px; flex-shrink: 0; }
.track-meta { min-width: 0; }
.track-name {
  font-size: var(--fs-player-title); font-weight: 600;
  overflow: hidden;
  mask-image: linear-gradient(to right, transparent 0%, black 5%, black 85%, transparent 100%);
  -webkit-mask-image: linear-gradient(to right, transparent 0%, black 5%, black 85%, transparent 100%);
}
.track-artist { font-size: var(--fs-player-meta); color: #a7a7a7; margin-top: 3px; }

.player-center {
  display: flex; flex-direction: column; align-items: center;
  gap: 6px; padding: 14px 0;
}
.ctrl-row { display: flex; align-items: center; gap: 16px; }

.icon-btn {
  background: none; border: none;
  color: #b3b3b3; cursor: pointer; padding: 4px;
  display: flex; align-items: center; justify-content: center;
  position: relative; border-radius: 4px;
  transition: color .1s;
}
.icon-btn:hover { color: #fff; }
.icon-btn.green { color: #1db954; }
.icon-btn.green:hover { color: #1ed760; }
.icon-btn.dot::after {
  content: '';
  position: absolute; bottom: -3px; left: 50%;
  transform: translateX(-50%);
  width: 4px; height: 4px; border-radius: 50%;
  background: #1db954;
}

.play-btn {
  width: 36px; height: 36px; border-radius: 50%;
  background: #fff; border: none; color: #000;
  display: flex; align-items: center; justify-content: center;
  cursor: pointer;
  transition: transform .1s, background .1s;
}
.play-btn:hover { transform: scale(1.06); background: #f0f0f0; }

.progress-row { display: flex; align-items: center; gap: 8px; width: 100%; }
.time { font-size: var(--fs-body-sm); color: #a7a7a7; min-width: 34px; text-align: center; }

.bar {
  flex: 1; height: 4px;
  background: #535353; border-radius: 2px;
  cursor: pointer; position: relative;
}
.bar:hover .bar-fill { background: #1db954; }
.bar:hover .bar-thumb { opacity: 1; }

.bar-fill {
  height: 100%; background: #fff;
  border-radius: 2px; position: relative;
  transition: background .1s;
}
.bar-thumb {
  position: absolute; right: -6px; top: 50%;
  transform: translateY(-50%);
  width: 12px; height: 12px; border-radius: 50%;
  background: #fff; opacity: 0; transition: opacity .1s;
}

.player-right {
  display: flex; align-items: center;
  justify-content: flex-end; gap: 6px;
}
.vol-wrap { display: flex; align-items: center; gap: 6px; width: 130px; }
.bar.vol { flex: 1; }

/* ── Library view ── */
.library-header {
  display: flex; align-items: center; justify-content: space-between;
  margin-bottom: 16px; gap: 16px;
}
.library-search {
  background: #282828; border: none; border-radius: 4px;
  color: #fff; font-size: var(--fs-input); padding: 8px 12px;
  width: 220px; outline: none;
}
.library-search::placeholder { color: #777; }
.library-search:focus { outline: 1px solid #555; }
.library-empty {
  color: #a7a7a7; font-size: var(--fs-empty); padding: 32px 0;
}
.about-summary {
  align-items: center;
  gap: 12px 18px;
  margin-bottom: 16px;
}
.about-error {
  margin: 0 0 16px;
}
.about-version-line {
  color: #fff;
  font-size: 28px;
  line-height: 1.15;
}
.about-version-line strong {
  color: #d9dee8;
  font-weight: 800;
}
.about-update-inline {
  color: #a7a7a7;
  font-size: var(--fs-body-md);
}
.about-update-inline-live {
  color: #8cf7b0;
}
.about-update-inline-error {
  color: #ff9d9d;
}
.about-download-link {
  color: #8cf7b0;
  font-size: var(--fs-body-md);
  text-decoration: none;
}
.about-download-link:hover {
  text-decoration: underline;
}
.search-empty-copy {
  color: #a7a7a7;
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
  color: #a7a7a7;
  font-size: var(--fs-body-md);
}
.search-summary strong {
  color: #fff;
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
  color: #8de2a7;
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
  background: linear-gradient(135deg, #21453a, #0f1820);
  color: #8de2a7;
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
  background: rgba(255, 255, 255, 0.08);
  color: #d7d7d7;
}
.soulseek-status-pill.state-completed {
  background: rgba(29, 185, 84, 0.16);
  color: #8de2a7;
}
.soulseek-status-pill.state-failed,
.soulseek-status-pill.state-timed_out,
.soulseek-status-pill.state-cancelled {
  background: rgba(233, 40, 62, 0.16);
  color: #ff98a3;
}
.soulseek-progress-copy {
  font-size: var(--fs-body-sm);
  color: #a7a7a7;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.soulseek-more-copy {
  padding-top: 10px;
}
.soulseek-inline-error {
  color: #ff98a3;
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
  color: #a7a7a7;
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
  color: #b3b3b3;
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
  border: 1px solid #555;
  border-radius: 999px;
  flex-shrink: 0;
}
.soulseek-download-icon-btn:hover:not(:disabled) {
  border-color: #fff;
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
.track-play-count {
  font-size: var(--fs-track-side);
  color: #a7a7a7;
  user-select: none;
}
.history-time {
  font-size: var(--fs-body-sm);
  color: #a7a7a7;
  white-space: nowrap;
}
.smart-name-input {
  font-size: var(--fs-modal-title);
  font-weight: 600;
  padding: 4px 10px;
}
.smart-track-count {
  font-size: var(--fs-body-sm);
  color: #a7a7a7;
}
.smart-empty-values {
  color: #777;
  font-size: var(--fs-body-sm);
}
.device-emoji {
  font-size: calc(var(--fs-peer-title) + 2px);
  line-height: 1;
}
.discovery-caption {
  color: #a7a7a7;
  font-size: var(--fs-body-sm);
}
.discovery-help-text {
  font-size: var(--fs-body-sm);
  color: #535353;
}
/* Playlist menu */
.playlist-menu-backdrop {
  position: fixed; inset: 0; z-index: 1000;
}
.playlist-menu {
  position: fixed; z-index: 1001;
  background: #282828; border: 1px solid #3a3a3a; border-radius: 8px;
  min-width: 180px; max-width: 240px;
  box-shadow: 0 8px 24px rgba(0,0,0,.5);
  overflow: hidden;
  -webkit-user-select: none;
  user-select: none;
  -webkit-touch-callout: none;
}
.playlist-menu-header {
  font-size: var(--fs-eyebrow); font-weight: 700; text-transform: uppercase;
  letter-spacing: .06em; color: #a7a7a7;
  padding: 10px 14px 6px;
  border-bottom: 1px solid #333;
  -webkit-user-select: none;
  user-select: none;
  -webkit-touch-callout: none;
}
.playlist-menu-empty {
  font-size: var(--fs-body-sm); color: #777; padding: 10px 14px;
  -webkit-user-select: none;
  user-select: none;
  -webkit-touch-callout: none;
}
.playlist-menu-item {
  display: block; width: 100%; text-align: left;
  background: none; border: none; color: #e0e0e0;
  font-size: var(--fs-button); padding: 9px 14px; cursor: pointer;
  -webkit-user-select: none;
  user-select: none;
  -webkit-touch-callout: none;
}
.playlist-menu-item:hover { background: #333; }
.playlist-menu-item.danger { color: #ffb7b7; }
.playlist-menu-item.danger:hover { background: rgba(255, 82, 82, 0.12); }

/* Smart Playlists */
.sp-match-row {
  display: flex; align-items: center; gap: 8px;
  font-size: var(--fs-body-md); color: #a7a7a7; padding: 0 0 14px;
}
.sp-match-btn {
  background: transparent; border: 1px solid #535353; color: #a7a7a7;
  border-radius: 20px; padding: 4px 12px; font-size: var(--fs-body-sm);
  font-weight: 600; cursor: pointer; transition: all .15s;
}
.sp-match-btn.active, .sp-match-btn:hover { background: #1db954; border-color: #1db954; color: #000; }
.sp-rules { display: flex; flex-direction: column; gap: 8px; margin-bottom: 20px; }
.sp-rule-row {
  display: flex; align-items: center; gap: 8px;
  background: #1e1e1e; border-radius: 12px; padding: 8px 12px;
  flex-wrap: wrap;
}
.sp-select {
  background: #282828; border: 1px solid #535353; border-radius: 20px;
  color: #a7a7a7; font-size: var(--fs-input); padding: 6px 34px 6px 12px; cursor: pointer;
  min-height: 34px;
  line-height: 1.2;
  appearance: none; -webkit-appearance: none;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='%23a7a7a7'%3E%3Cpath d='M7 10l5 5 5-5z'/%3E%3C/svg%3E");
  background-repeat: no-repeat; background-position: right 12px center; background-size: 12px 12px;
  outline: none; transition: border-color .15s, color .15s;
}
.sp-select:focus { border-color: #fff; color: #fff; }
.sp-select:hover { border-color: #fff; color: #fff; }
.sp-select option {
  background: #282828;
  color: #fff;
}
.sp-op-label { font-size: var(--fs-body-sm); color: #a7a7a7; white-space: nowrap; }
.sp-text-input { flex: 1; min-width: 100px; }
.sp-num-input { width: 80px; }
.sp-add-rule-btn {
  background: transparent; border: 1px solid #535353; color: #a7a7a7;
  border-radius: 20px; padding: 5px 14px; font-size: var(--fs-body-sm); font-weight: 600;
  cursor: pointer; transition: all .15s;
  align-self: flex-start;
}
.sp-add-rule-btn:hover { border-color: #fff; color: #fff; }
.sp-multiselect {
  display: flex; flex-wrap: wrap; gap: 6px; align-items: center; flex: 1;
}
.sp-chip {
  background: #282828; border: 1px solid #444; color: #a7a7a7;
  border-radius: 20px; padding: 3px 10px; font-size: var(--fs-body-sm);
  cursor: pointer; transition: background .12s, color .12s, border-color .12s;
  user-select: none;
}
.sp-chip.selected { background: #1db954; border-color: #1db954; color: #000; font-weight: 600; }
.sp-chip:hover:not(.selected) { border-color: #888; color: #e0e0e0; }
.sp-preview-header {
  font-size: var(--fs-eyebrow); font-weight: 700; text-transform: uppercase;
  letter-spacing: .06em; color: #a7a7a7;
  padding: 0 0 10px; border-bottom: 1px solid #282828; margin-bottom: 8px;
}
.sp-preview { opacity: 0.9; }
.track-groups { display: flex; flex-direction: column; gap: 24px; }
.group-artist {
  font-size: var(--fs-group); font-weight: 700; color: #a7a7a7;
  text-transform: uppercase; letter-spacing: .04em;
  padding-bottom: 8px; border-bottom: 1px solid #282828;
  margin-bottom: 4px;
}
.track-row {
  display: flex; align-items: center; gap: 12px;
  padding: 8px 8px; border-radius: 4px;
  cursor: pointer; user-select: none;
}
.track-row:hover { background: #282828 !important; }
.track-row.rarity-tint { background: color-mix(in srgb, var(--rc) 12%, #181818); }
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
.track-row.track-row-current {
  background: rgba(29, 185, 84, 0.14) !important;
  box-shadow: inset 0 0 0 1px rgba(29, 185, 84, 0.3);
}
.track-row.track-row-next {
  background: rgba(29, 185, 84, 0.08) !important;
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
.edit-btn {
  opacity: 0; transition: opacity .12s; flex-shrink: 0;
}
.track-row:hover .edit-btn { opacity: 1; }

/* ── Edit modal ── */
.modal-overlay {
  position: fixed; inset: 0;
  background: rgba(0, 0, 0, .65);
  display: flex; align-items: center; justify-content: center;
  z-index: 500;
}
.modal {
  background: #282828; border-radius: 8px;
  width: 400px; max-width: 90vw;
  box-shadow: 0 16px 48px rgba(0,0,0,.6);
}
.modal-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 20px 24px 12px;
}
.modal-header h3 { font-size: var(--fs-modal-title); font-weight: 700; }
.modal-body { padding: 8px 24px 16px; display: flex; flex-direction: column; gap: 14px; }
.field {
  display: flex; flex-direction: column; gap: 4px;
}
.field span {
  font-size: var(--fs-field-label); font-weight: 600; color: #a7a7a7; text-transform: uppercase; letter-spacing: .03em;
}
.field input,
.field select {
  background: #3e3e3e; border: none; border-radius: 4px;
  color: #fff; font-size: var(--fs-field-input); padding: 10px 12px; outline: none;
  min-height: 42px;
  line-height: 1.2;
}
.field select {
  cursor: pointer;
  appearance: none;
  -webkit-appearance: none;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='%23ffffff'%3E%3Cpath d='M7 10l5 5 5-5z'/%3E%3C/svg%3E");
  background-repeat: no-repeat;
  background-position: right 12px center;
  background-size: 12px 12px;
  padding-right: 40px;
}
.field select option {
  background: #282828;
  color: #fff;
}
.field input:focus,
.field select:focus { outline: 1px solid #1db954; }
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
  background: transparent; border: 1px solid #727272; border-radius: 20px;
  color: #fff; font-size: var(--fs-button); font-weight: 700;
  padding: 8px 24px; cursor: pointer;
}
.btn-secondary:hover { border-color: #fff; }
.btn-primary {
  background: #1db954; border: none; border-radius: 20px;
  color: #000; font-size: var(--fs-button); font-weight: 700;
  padding: 8px 28px; cursor: pointer;
}
.btn-primary:hover { background: #1ed760; transform: scale(1.02); }

.modal-enter-active, .modal-leave-active { transition: opacity .15s; }
.modal-enter-from, .modal-leave-to { opacity: 0; }
.nav-overlay-enter-active, .nav-overlay-leave-active { transition: opacity .25s; }
.nav-overlay-enter-from, .nav-overlay-leave-to { opacity: 0; }

/* ── Device popup ── */
.device-menu-wrapper { position: relative; }
.device-dropdown {
  position: absolute;
  top: auto;
  bottom: calc(100% + 12px);
  right: 0;
  min-width: 220px;
}
.dropdown-header {
  padding: 10px 16px 6px;
  font-size: var(--fs-dropdown-label);
  font-weight: 700;
  color: #a7a7a7;
  text-transform: uppercase;
  letter-spacing: .04em;
}
.device-list {
  max-height: 240px;
  overflow-y: auto;
}
.device-section-label {
  padding: 10px 16px 6px;
  color: #8b8b8b;
  font-size: var(--fs-eyebrow);
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: .04em;
}
.device-list::-webkit-scrollbar { width: 6px; }
.device-list::-webkit-scrollbar-thumb { background: #555; border-radius: 3px; }
.device-list::-webkit-scrollbar-track { background: transparent; }
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
}
.device-error {
  padding: 0 16px 12px;
  color: #e9283e;
  font-size: var(--fs-body-sm);
}

/* ── Queue popup ── */
.queue-menu-wrapper { position: relative; }
.queue-dropdown {
  position: absolute;
  top: auto;
  bottom: calc(100% + 12px);
  right: 0;
  min-width: 300px;
  max-width: 360px;
}
.queue-now-label, .queue-next-label {
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
.queue-item.active { background: #333; cursor: default; }
.queue-item-cover {
  width: 36px; height: 36px;
  border-radius: 3px;
  flex-shrink: 0;
}
.queue-item-info {
  flex: 1; min-width: 0;
  display: flex; flex-direction: column; gap: 2px;
}
.queue-item-title {
  font-size: var(--fs-queue-title); font-weight: 600; color: #fff;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.queue-item.active .queue-item-title { color: #1db954; }
.queue-item-artist {
  font-size: var(--fs-queue-meta); color: #a7a7a7;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.queue-item-dur {
  font-size: var(--fs-queue-dur); color: #a7a7a7; flex-shrink: 0;
}
.queue-list {
  max-height: 260px;
  overflow-y: auto;
}
.queue-list::-webkit-scrollbar { width: 6px; }
.queue-list::-webkit-scrollbar-thumb { background: #555; border-radius: 3px; }
.queue-list::-webkit-scrollbar-track { background: transparent; }
.queue-empty {
  padding: 16px;
  color: #a7a7a7;
  font-size: var(--fs-body-md);
  text-align: center;
}

/* ── Rarity animations ── */
.rarity-animated { position: relative; overflow: hidden; }
.rarity-animated::before {
  content: '';
  position: absolute; inset: 0;
  pointer-events: none;
  border-radius: inherit;
}

/* Epic: diagonal moving stripes */
.rarity-epic::before {
  background: repeating-linear-gradient(
    -45deg,
    var(--rc) 48%,
    var(--rc) 48%,
    transparent 40px,
    transparent 80px
  );
  background-size: 113px 113px;
  opacity: .15;
  animation: rarity-slide 3s linear infinite;
  image-rendering: pixelated;
}
@keyframes rarity-slide {
  from { background-position: 0 0; }
  to   { background-position: 113px 113px; }
}

/* Legendary: pulsing glow that breathes */
.rarity-legendary::before {
  background: radial-gradient(ellipse at 30% 50%, var(--rc), transparent 70%);
  opacity: 0;
  animation: rarity-pulse 2.5s ease-in-out infinite;
}
@keyframes rarity-pulse {
  0%, 100% { opacity: .06; transform: scale(1); }
  50%      { opacity: .22; transform: scale(1.15); }
}

/* Mythic: repeating diagonal light streaks */
.rarity-mythic::before {
  inset: -10px 0 0 0;
  background: repeating-linear-gradient(
    105deg,
    transparent 0px,
    transparent 20px,
    var(--rc) 20px,
    var(--rc) 36px
  );
  opacity: .2;
  animation: rarity-sweep 1.5s linear infinite;
}
@keyframes rarity-sweep {
  from { background-position: 0 0; }
  to   { background-position: 34.77px 9.32px; }
}

/* ── Identify progress ── */
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
  background: #282828;
  border: 1px solid #3e3e3e;
  border-radius: 20px;
  padding: 6px 14px;
  cursor: pointer;
  font-size: var(--fs-status-pill);
  font-weight: 600;
  color: #fff;
  box-shadow: 0 4px 16px rgba(0,0,0,.5);
  transition: background .12s;
}
.identify-mini:hover { background: #333; cursor: pointer; }
.identify-mini.index-done { border-color: #1db954; }
.identify-mini-spinner {
  width: 14px; height: 14px;
  border: 2px solid #555;
  border-top-color: #1db954;
  border-radius: 50%;
  animation: ident-spin .8s linear infinite;
}
.identify-mini-done { color: #1db954; font-size: 14px; }
@keyframes ident-spin { to { transform: rotate(360deg); } }
.identify-modal { width: 440px; }
.powered-by { font-size: var(--fs-powered-by); font-weight: 400; color: #888; margin-left: 6px; }
.powered-by a { color: #1db954; text-decoration: none; }
.powered-by a:hover { text-decoration: underline; }
.identify-bar-wrap {
  height: 4px;
  background: #535353;
  border-radius: 2px;
  overflow: hidden;
  margin-bottom: 8px;
}
.identify-bar-fill {
  height: 100%;
  background: #1db954;
  border-radius: 2px;
  transition: width .3s;
}
.identify-status {
  font-size: var(--fs-body-sm);
  color: #a7a7a7;
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
.identify-results::-webkit-scrollbar-thumb { background: #555; border-radius: 3px; }
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
  color: #ccc;
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
.ir-found { color: #1db954; }
.ir-not_found { color: #a7a7a7; }
.ir-error { color: #ff5252; }
.ir-fingerprinting { color: #4fc3f7; }
.ir-looking_up { color: #ffa726; }
.ir-done { color: #1db954; font-weight: 600; }
.identify-empty {
  color: #a7a7a7;
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
.index-results::-webkit-scrollbar-thumb { background: #555; border-radius: 3px; }
.index-results::-webkit-scrollbar-track { background: transparent; }
.index-empty { padding: 40px 20px; text-align: center; color: #a7a7a7; font-size: var(--fs-empty); }

/* Progress bar shown above the list while scanning */
.log-progress-wrap {
  position: relative;
  height: 4px;
  background: #333;
  border-radius: 2px;
  margin-bottom: 12px;
  overflow: hidden;
}
.log-progress-bar {
  height: 100%;
  background: #1db954;
  border-radius: 2px;
  transition: width 0.3s;
}
.log-progress-label {
  position: absolute;
  right: 0;
  top: 7px;
  font-size: var(--fs-powered-by);
  color: #a7a7a7;
}

/* Session card */
.log-session {
  background: #1a1a1a;
  border-radius: 8px;
  padding: 10px 12px;
  border-left: 3px solid #333;
}
.ls-local  { border-left-color: #4fc3f7; }
.ls-sync   { border-left-color: #1db954; }
.ls-error  { border-left-color: #ff5252; }

.ls-header {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-body-md);
}
.ls-icon { font-size: 16px; flex-shrink: 0; }
.ls-source { font-weight: 600; color: #fff; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ls-time { font-size: var(--fs-powered-by); color: #666; flex-shrink: 0; }

.ls-badge {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: var(--fs-body-sm);
  color: #a7a7a7;
  white-space: nowrap;
  flex-shrink: 0;
}
.ls-badge-done { color: #1db954; }
.ls-badge-err  { color: #ff5252; }

.ls-spinner {
  display: inline-block;
  width: 10px; height: 10px;
  border: 2px solid #555;
  border-top-color: #fff;
  border-radius: 50%;
  animation: spin 0.7s linear infinite;
}
.ls-error-msg {
  margin-top: 6px;
  font-size: var(--fs-body-sm);
  color: #ff7070;
  word-break: break-all;
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
  color: #ccc;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.ls-file-icon { color: #777; flex-shrink: 0; font-style: normal; }

/* Hidden on desktop; appears above footer on mobile/tablet */
.mobile-seek-wrap { display: none; }

/* ── Burger & mobile nav overlay (hidden on desktop) ── */
.burger-btn { display: none; }
.mobile-nav-overlay { display: none; }
.sidebar-mobile-header { display: none; }

/* ── Responsive: tablets and small screens ── */
@media (max-width: 768px) {
  .app {
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: 1fr calc(130px + env(safe-area-inset-bottom));
    --fs-nav: 15px;
    --fs-h2: 20px;
    --fs-section-link: 10px;
    --fs-card-title: 12px;
    --fs-card-meta: 11px;
    --fs-player-title: 12px;
    --fs-player-meta: 10px;
    --fs-input: 12px;
    --fs-empty: 13px;
    --fs-eyebrow: 10px;
    --fs-group: 12px;
    --fs-track-side: 12px;
    --fs-track-title: 13px;
    --fs-track-meta: 11px;
    --fs-badge: 10px;
    --fs-modal-title: 16px;
    --fs-field-label: 11px;
    --fs-field-input: 13px;
    --fs-button: 12px;
    --fs-dropdown-label: 11px;
    --fs-dropdown-item: 13px;
    --fs-queue-title: 12px;
    --fs-queue-meta: 10px;
    --fs-queue-dur: 11px;
    --fs-status-pill: 12px;
    --fs-powered-by: 10px;
    --fs-body-sm: 11px;
    --fs-body-md: 12px;
    --fs-peer-title: 13px;
    --fs-peer-meta: 11px;
    --fs-control: 11px;
    --fs-detail-title: 17px;
    --fs-detail-artist: 13px;
    --fs-detail-time: 11px;
  }

  /* Burger button */
  .burger-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    color: #fff;
    cursor: pointer;
    padding: 6px;
    margin-right: 8px;
    flex-shrink: 0;
  }

  /* Dark overlay */
  .mobile-nav-overlay {
    display: block;
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    z-index: 399;
  }

  /* Sidebar becomes a slide-in drawer */
  .sidebar {
    position: fixed;
    top: 0;
    left: 0;
    bottom: 0;
    width: 260px;
    z-index: 400;
    flex-direction: column;
    padding: 0 0 24px;
    padding-top: env(safe-area-inset-top);
    gap: 0;
    overflow-x: hidden;
    overflow-y: auto;
    border-bottom: none;
    border-right: 1px solid #282828;
    background: #121212;
    transform: translateX(-100%);
    transition: transform 0.28s cubic-bezier(0.4, 0, 0.2, 1);
  }
  .sidebar.sidebar-open {
    transform: translateX(0);
  }

  /* Close button row at top of drawer */
  .sidebar-mobile-header {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    padding: 12px 12px 4px;
    flex-shrink: 0;
  }
  .sidebar-close-btn {
    color: #b3b3b3;
    padding: 8px;
  }

  .sidebar nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 4px 8px;
    width: 100%;
  }
  .sidebar .sidebar-divider { display: block; }
  .nav-item {
    gap: 12px;
    padding: 12px 14px;
    font-size: var(--fs-nav);
    white-space: nowrap;
    flex-shrink: 0;
    border-radius: 6px;
  }
  .nav-item svg { width: 20px; height: 20px; }

  /* Main spans the full grid now that sidebar is out of flow */
  .main { grid-row: 1 / 2; grid-column: 1; }

  .content { padding: 0 14px 24px; }

  .topbar {
    padding: 10px 14px;
    padding-top: calc(10px + env(safe-area-inset-top));
    gap: 8px;
  }

  .duplicates-toggle {
    margin-left: auto;
    font-size: 11px;
    padding: 6px 10px;
    white-space: nowrap;
  }

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
  .track-num { display: none; }
  .edit-btn { opacity: 1; }
  .track-inline-action { display: none !important; }

  .player {
    grid-column: 1;
    grid-row: 2 / 3;
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
    width: calc(100%);
  }
  .player-right { display: none; }
  .player-left .thumb { width: 40px; height: 40px; }
  .player-left .track-meta { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex-grow: 1; }
  .track-name { font-size: var(--fs-player-title); font-weight: 600; color: #fff; }
  .track-artist { font-size: var(--fs-player-meta); color: #a7a7a7; }
  .player-center {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 0;
    align-items: stretch;
    order: 0;
  }
  .ctrl-row { gap: 8px; justify-content: center; }
  .icon-btn { padding: 6px; }
  .play-btn { width: 40px; height: 40px; }
  .progress-row .time { display: none; }
  .player-right .vol-wrap { display: none; }
  .progress-row { display: none; }
  .bar { flex: 1; }

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
    background: #3a3a3a;
    overflow: hidden;
    transition: height .15s ease;
  }
  .mobile-seek-wrap.active .mobile-seek-track {
    height: 10px;
  }
  .mobile-seek-fill {
    height: 100%;
    background: #fff;
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
    background: #fff;
    opacity: 0;
    transition: opacity .15s ease;
  }
  .mobile-seek-wrap.active .mobile-seek-thumb {
    opacity: 1;
  }

  .status-pills { top: auto; bottom: calc(150px + env(safe-area-inset-bottom)); right: 14px; flex-direction: column; align-items: end; }

  .modal { width: 95vw; }
  .identify-modal { width: 95vw; }
  .queue-dropdown { min-width: 260px; max-width: 92vw; }
  .device-dropdown { min-width: 200px; }
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

  .nav-item svg { width: 18px; height: 18px; }
  .nav-item { padding: 10px 12px; font-size: var(--fs-nav); }

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

  .player {
    grid-template-columns: 1fr auto 1fr;
    padding: 0 8px;
    padding-bottom: env(safe-area-inset-top);
    /* height: 64px; */
  }
  .player-left { width: 100%; }
  .player-left .thumb { width: 40px; height: 40px; }
  /* .player-left .track-meta { flex-grow: unset; } */
  .ctrl-row { gap: 8px; }

  .player-right .device-menu-wrapper { display: none; }

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
  background: rgba(0,0,0,.45);
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
  overflow-y: auto;
  overflow-x: hidden;
  background: linear-gradient(180deg, rgba(10,12,18,.8), rgba(10,12,18,.94));
  backdrop-filter: blur(10px) saturate(1.05);
  border-radius: 20px 20px 0 0;
  border: 1px solid rgba(255,255,255,.08);
  box-shadow: 0 -10px 60px rgba(0,0,0,.38);
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
  background: #444;
  margin: 12px auto 0;
  cursor: pointer;
  flex-shrink: 0;
}

.detail-cover-wrap {
  position: relative;
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
  box-shadow: 24px 18px 42px rgba(0,0,0,0.4);
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
  border: 1px solid rgba(255,255,255,0.08);
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
  font-size: var(--fs-detail-title); font-weight: 700; color: #fff;
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
  font-size: var(--fs-detail-artist); color: #a7a7a7;
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
  background: #3a3a3a;
  border-radius: 999px;
  pointer-events: none;
}

.detail-bar-fill {
  position: absolute;
  inset: 0 auto 0 0;
  height: 4px;
  background: #fff;
  border-radius: 999px;
  pointer-events: none;
}

.detail-bar-thumb {
  position: absolute;
  right: -6px; top: 50%;
  transform: translateY(-50%);
  width: 12px; height: 12px;
  border-radius: 50%;
  background: #fff;
}

.detail-bar-track {
  flex-shrink: 0;
}

.detail-time-row {
  display: flex;
  justify-content: space-between;
  font-size: var(--fs-detail-time);
  color: #a7a7a7;
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
  background: #fff;
  border: none;
  display: flex; align-items: center; justify-content: center;
  cursor: pointer;
  color: #000;
}

.detail-play-btn:hover { background: #e0e0e0; }

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
    background: rgba(0,0,0,.72);
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