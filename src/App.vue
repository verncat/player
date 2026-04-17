<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openPath } from "@tauri-apps/plugin-opener";

interface AudioDevice { name: string }
interface DeviceList { devices: AudioDevice[]; current: string | null }

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
const outputDevices = ref<AudioDevice[]>([]);
const currentDevice = ref<string | null>(null);
const activeNav = ref("home");
const libraryTracks = ref<Track[]>([]);
const libraryLoading = ref(false);
const searchQuery = ref("");
const editingTrack = ref<Track | null>(null);
const editForm = ref({ title: '', artist: '', album: '', track_number: null as number | null });
const covers = ref<Record<number, string | null>>({});

/* ── Beat animation ── */
const beatScale = ref(1);
let beatRafId: number | null = null;
let beatStartTime = 0;
const BEAT_AMP = 0.10;    // max scale overshoot (1.10× at peak)
const BEAT_TAU = 130;     // exponential decay time-constant in ms

function startBeatAnimation(lagMs = 0) {
  beatStartTime = performance.now() - lagMs;
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
type QueueSource = 'recent' | 'library' | 'playlist';
const queueSource = ref<QueueSource>('library');
const queueSourceIndex = ref(0);       // index into source list of the LAST item pushed
const queuePlaylistTracks = ref<Track[]>([]); // tracks for 'playlist' source
const queue = ref<Track[]>([]);         // upcoming tracks (max 5 visible)
const nowPlaying = ref<Track | null>(null);
const recentTracks = ref<Track[]>([]);
const showQueueMenu = ref(false);

interface PlayHistoryEntry {
  played_at: number; // unix timestamp (seconds)
  track: Track;
}
const historyEntries = ref<PlayHistoryEntry[]>([]);
const historyLoading = ref(false);

// ── Playlists ──────────────────────────────────────────────────────────────
interface Playlist {
  id: number;
  name: string;
  created_at: number;
  track_count: number;
}
const playlists = ref<Playlist[]>([]);
const playlistView = ref<{ id: number; name: string; tracks: Track[] } | null>(null);
const showNewPlaylistInput = ref(false);
const newPlaylistName = ref('');
// context menu for "add to playlist"
const addToPlaylistMenu = ref<{ track: Track; x: number; y: number } | null>(null);

async function loadPlaylists() {
  playlists.value = await invoke<Playlist[]>('get_playlists');
}

async function createPlaylist() {
  const name = newPlaylistName.value.trim();
  if (!name) return;
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

async function openPlaylist(pl: Playlist) {
  const tracks = await invoke<Track[]>('get_playlist_tracks', { playlistId: pl.id });
  playlistView.value = { id: pl.id, name: pl.name, tracks };
}

async function addTrackToPlaylist(playlistId: number, trackId: number) {
  await invoke('add_track_to_playlist', { playlistId, trackId });
  await loadPlaylists();
  if (playlistView.value?.id === playlistId) {
    const tracks = await invoke<Track[]>('get_playlist_tracks', { playlistId });
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

function openAddToPlaylistMenu(e: MouseEvent, track: Track) {
  e.stopPropagation();
  addToPlaylistMenu.value = { track, x: e.clientX, y: e.clientY };
}

// ── Smart Playlists ──────────────────────────────────────────────────────────
type SPField = 'any' | 'title' | 'artist' | 'album' | 'genre' | 'extension' | 'year' | 'play_count' | 'is_liked';
type SPOp = 'contains' | 'in' | 'eq' | 'gte' | 'lte' | 'is_true' | 'is_false';
interface SPRule { id: string; field: SPField; op: SPOp; value: string; }
interface SmartPlaylist { id: string; name: string; match: 'all' | 'any'; rules: SPRule[]; }

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
    const rows = await invoke<{ id: string; name: string; match_mode: string; rules_json: string }[]>('get_smart_playlists');
    smartPlaylists.value = rows.map(r => ({
      id: r.id, name: r.name, match: r.match_mode as 'all' | 'any',
      rules: JSON.parse(r.rules_json || '[]'),
    }));
  } catch { smartPlaylists.value = []; }
}
async function createSmartPlaylist() {
  const name = newSPName.value.trim();
  if (!name) return;
  const sp: SmartPlaylist = { id: crypto.randomUUID(), name, match: 'all', rules: [] };
  await invoke('save_smart_playlist', {
    id: sp.id, name: sp.name, matchMode: sp.match, rulesJson: '[]',
  });
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
function saveSP() {
  if (!editingSP.value) return;
  const idx = smartPlaylists.value.findIndex(p => p.id === editingSP.value!.id);
  if (idx !== -1) smartPlaylists.value[idx] = { ...editingSP.value };
  const sp = editingSP.value;
  invoke('save_smart_playlist', {
    id: sp.id, name: sp.name, matchMode: sp.match,
    rulesJson: JSON.stringify(sp.rules),
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
  if (f === 'is_liked') { rule.op = 'is_true'; rule.value = ''; }
  else if (f === 'year' || f === 'play_count') { rule.op = 'gte'; rule.value = '0'; }
  else if (f === 'genre' || f === 'extension' || f === 'artist' || f === 'album') { rule.op = 'in'; rule.value = '[]'; }
  else { rule.op = 'contains'; rule.value = ''; }
  saveSP();
}
function spFieldType(field: SPField): 'text' | 'multiselect' | 'number' | 'bool' {
  if (field === 'is_liked') return 'bool';
  if (field === 'year' || field === 'play_count') return 'number';
  if (field === 'genre' || field === 'extension' || field === 'artist' || field === 'album') return 'multiselect';
  return 'text';
}
function spUniqueValues(field: SPField): string[] {
  const set = new Set<string>();
  for (const t of libraryTracks.value) {
    if (field === 'genre' && t.genre) set.add(t.genre);
    else if (field === 'extension') { const e = t.path.split('.').pop()?.toLowerCase(); if (e) set.add(e); }
    else if (field === 'artist' && t.artist) set.add(t.artist);
    else if (field === 'album' && t.album) set.add(t.album);
  }
  return [...set].sort();
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
function matchesRule(track: Track, rule: SPRule): boolean {
  switch (rule.field) {
    case 'any': {
      const q = rule.value.toLowerCase();
      return !q || !!(track.title?.toLowerCase().includes(q) || track.artist?.toLowerCase().includes(q) ||
        track.album?.toLowerCase().includes(q) || track.genre?.toLowerCase().includes(q) ||
        track.path.toLowerCase().includes(q));
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
    case 'extension': {
      const ext = track.path.split('.').pop()?.toLowerCase() || '';
      if (rule.op === 'in') { const s: string[] = JSON.parse(rule.value || '[]'); return s.length === 0 || s.includes(ext); }
      return ext.includes(rule.value.toLowerCase());
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
    default: return true;
  }
}
function smartPlaylistTracks(sp: SmartPlaylist): Track[] {
  if (sp.rules.length === 0) return [];
  return libraryTracks.value.filter(t =>
    sp.match === 'all' ? sp.rules.every(r => matchesRule(t, r)) : sp.rules.some(r => matchesRule(t, r))
  );
}
// ────────────────────────────────────────────────────────────────────────────


interface Peer { name: string; host: string; port: number; addresses: string[]; device_name?: string; device_emoji?: string }
const peers = ref<Peer[]>([]);

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
const settingsEmoji = ref('🎵');
const settingsDeviceName = ref('');
const settingsSaving = ref(false);
const settingsError = ref('');

interface DeviceSettings {
  emoji: string;
  device_name: string;
}

async function openDeviceSettings() {
  settingsError.value = '';
  const cfg = await invoke<DeviceSettings>('get_device_settings');
  settingsEmoji.value = cfg.emoji || '🎵';
  settingsDeviceName.value = cfg.device_name || '';
  settingsOpen.value = true;
}

async function saveDeviceSettings() {
  settingsSaving.value = true;
  settingsError.value = '';
  try {
    await invoke('set_device_settings', {
      emoji: settingsEmoji.value,
      deviceName: settingsDeviceName.value,
    });
    deviceEmoji.value = settingsEmoji.value;
    settingsOpen.value = false;
  } catch (e: any) {
    settingsError.value = String(e ?? 'Failed to save settings');
  } finally {
    settingsSaving.value = false;
  }
}

async function toggleSync() {
  syncEnabled.value = !syncEnabled.value;
  await invoke('sync_set_enabled', { enabled: syncEnabled.value });
  if (syncEnabled.value) {
    // Kick off sync with all currently known peers
    for (const peer of peers.value) {
      invoke('sync_with_peer', { peerHost: peer.host, peerName: peer.name, peerAddresses: peer.addresses, peerPort: peer.port }).catch(() => {});
    }
  }
}

function syncPeer(peer: Peer) {
  invoke('sync_with_peer', { peerHost: peer.host, peerName: peer.name, peerAddresses: peer.addresses, peerPort: peer.port }).catch(() => {});
}

const currentTrack = computed(() => {
  if (nowPlaying.value) {
    return {
      title: nowPlaying.value.title || nowPlaying.value.path,
      artist: nowPlaying.value.artist || 'Unknown',
      colors: hashToColors(nowPlaying.value.file_hash),
    };
  }
  return { title: 'No track', artist: '', colors: ['#282828', '#181818'] };
});

const mobileSeekActive = ref(false);
const seekPreviewPos = ref<number | null>(null);
const showDetail = ref(false);

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

const cardTransform = computed(() => {
  const scale = cardDragging.value ? 1.06 : 1;
  return `perspective(600px) translateX(${cardTX.value}px) translateY(${cardTY.value}px) scale(${scale}) rotateX(${cardRotX.value}deg) rotateY(${cardRotY.value}deg)`;
});

// Specular highlight — bright spot moves opposite to tilt direction
const cardGloss = computed(() => {
  const rx = cardRotX.value;
  const ry = cardRotY.value;
  const sx = 50 - ry * 2.8;
  const sy = 50 + rx * 2.8;
  const intensity = Math.hypot(rx, ry) / MAX_TILT;
  const alpha = (0.1 + intensity * 0.25).toFixed(2);
  return `radial-gradient(ellipse 60% 55% at ${sx}% ${sy}%, rgba(255,255,255,${alpha}) 0%, rgba(255,255,255,0.03) 55%, transparent 75%)`;
});

// Glossy sheen — white bands of light sweep across the card as it tilts,
// tinted subtly by rarity color. Like light reflecting off a glossy surface.
const cardRainbow = computed(() => {
  const rx = cardRotX.value / MAX_TILT; // -1..1
  const ry = cardRotY.value / MAX_TILT;
  const intensity = Math.min(Math.hypot(rx, ry), 1);

  const rarity = nowPlaying.value?.rarity ?? null;
  const baseColor = rarityColors[rarity ?? ''] ?? '#ffffff';
  const r = parseInt(baseColor.slice(1, 3), 16);
  const g = parseInt(baseColor.slice(3, 5), 16);
  const b = parseInt(baseColor.slice(5, 7), 16);

  const sweep = ry * 0.9 - rx * 0.3;
  const band1 = 15 + sweep * 75;  // primary band sweeps 15%→90%
  const band2 = band1 + 30;       // secondary band
  const angle = 132 + ry * 18;

  // White gloss alpha — strong enough to be clearly visible
  const gloss1 = (0.3 + intensity * 0.55).toFixed(2);
  const gloss2 = (0.15 + intensity * 0.30).toFixed(2);
  // Rarity tint mixed in at the edge of each band (very subtle)
  const tint1  = (0.20 + intensity * 0.35).toFixed(2);
  const tint2  = (0.10 + intensity * 0.18).toFixed(2);

  const W1 = 12; // primary band half-width %
  const W2 = 8;  // secondary band half-width %

  // Band: hard edge → rarity tint → pure white hotspot → rarity tint → hard edge
  const glossBand = (cx: number, w: number, gA: string, tA: string) => {
    const e0 = (cx - w).toFixed(1);
    const e1 = (cx + w).toFixed(1);
    const t0 = (cx - w * 0.55).toFixed(1);
    const t1 = (cx + w * 0.55).toFixed(1);
    return [
      `transparent ${e0}%`,
      `rgba(${r},${g},${b},${tA}) ${e0}%`,
      `rgba(255,255,255,${gA}) ${t0}%`,
      `rgba(255,255,255,${gA}) ${t1}%`,
      `rgba(${r},${g},${b},${tA}) ${e1}%`,
      `transparent ${e1}%`,
    ].join(', ');
  };

  return `linear-gradient(${angle.toFixed(0)}deg,
    ${glossBand(band1, W1, gloss1, tint1)},
    ${glossBand(band2, W2, gloss2, tint2)}
  )`;
});

// Shadow shifts with tilt — card appears to lift off the surface
const cardShadow = computed(() => {
  const rx = cardRotX.value;
  const ry = cardRotY.value;
  const sx = (ry * 1.8).toFixed(1);
  const sy = (-rx * 1.8 + 16).toFixed(1);
  const blur = (40 + Math.hypot(rx, ry) * 1.2).toFixed(0);
  return `${sx}px ${sy}px ${blur}px rgba(0,0,0,0.6), 0 2px 8px rgba(0,0,0,0.35)`;
});

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
  } else {
    stopAmbient();
    cancelAnimationFrame(cardSpringRaf);
  }
});
// ────────────────────────────────────────────────────────────────────────────

const displayProgressPercent = computed(() => {
  const pos = seekPreviewPos.value ?? currentTime.value;
  return duration.value > 0 ? (pos / duration.value) * 100 : 0;
});

/** Return the full ordered list for a given source */
function sourceList(src: QueueSource): Track[] {
  if (src === 'recent') return recentTracks.value.length ? recentTracks.value : libraryTracks.value.slice(0, 12);
  if (src === 'playlist') return queuePlaylistTracks.value;
  // 'library' – flattened in grouped order (same as libraryFlatList)
  return libraryFlatList.value;
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
  while (queue.value.length < 5) {
    const nextIdx = queueSourceIndex.value + 1;
    if (nextIdx >= list.length) {
      if (queueSource.value === 'playlist' && repeatMode.value !== 1) break; // stop at end of playlist
      queueSourceIndex.value = -1; // wrap (library or repeat-all)
    } else {
      queueSourceIndex.value = nextIdx;
    }
    if (queueSourceIndex.value >= 0) queue.value.push(list[queueSourceIndex.value]);
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
  await invoke('playback_seek', { position: pos });
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

async function playTrackFrom(src: QueueSource, index: number) {
  const list = sourceList(src);
  if (!list.length) return;
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
  await invoke('playback_play', { path: track.path });
  if (!wasPlaying) {
    await invoke('playback_pause');
    isPlaying.value = false;
    stopTicker();
  } else {
    await refreshPlaybackState();
    track.play_count++;
    invoke('record_play', { id: track.id }).then(() => loadRecent());
    startTicker();
  }
  syncAndroid();
}

/** Advance to next track in queue */
async function playNext() {
  if (!queue.value.length) {
    isPlaying.value = false;
    stopTicker();
    await invoke('playback_stop');
    syncAndroid();
    return;
  }
  const next = queue.value.shift()!;
  nowPlaying.value = next;
  duration.value = next.duration_secs || 0;
  currentTime.value = 0;
  refillQueue();
  await invoke('playback_play', { path: next.path });
  await refreshPlaybackState();
  next.play_count++;
  invoke('record_play', { id: next.id }).then(() => loadRecent());
  startTicker();
  syncAndroid();
}

/** Go to previous track (restart current if >3s in, else go back in source) */
async function playPrev() {
  if (currentTime.value > 3) {
    currentTime.value = 0;
    await invoke('playback_seek', { position: 0 });
    return;
  }
  const list = sourceList(queueSource.value);
  if (!list.length) return;
  const curIdx = list.findIndex(t => t.id === nowPlaying.value?.id);
  const prevIdx = curIdx > 0 ? curIdx - 1 : list.length - 1;
  await playTrackFrom(queueSource.value, prevIdx);
}

function jumpToQueueItem(index: number) {
  // remove everything before that item, play it
  queue.value.splice(0, index);
  playNext();
  showQueueMenu.value = false;
}

interface PlaybackStatus { playing: boolean; finished: boolean; position: number; duration: number; }

async function refreshPlaybackState() {
  const st = await invoke<PlaybackStatus>('playback_status');
  isPlaying.value = st.playing;
  currentTime.value = Math.floor(st.position);
  if (st.duration > 0) duration.value = Math.floor(st.duration);
  return st;
}

function formatTime(s: number) {
  const m = Math.floor(s / 60);
  return `${m}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
}

let ticker: ReturnType<typeof setInterval> | null = null;
let androidSyncCounter = 0;

function stopTicker() {
  if (ticker) { clearInterval(ticker); ticker = null; }
}

async function handleFinishedPlayback() {
  if (repeatMode.value === 2) {
    if (nowPlaying.value) {
      await invoke('playback_play', { path: nowPlaying.value.path });
      startTicker();
      syncAndroid();
    }
  } else {
    await playNext();
  }
}

function startTicker() {
  stopTicker();
  androidSyncCounter = 0;
  ticker = setInterval(async () => {
    try {
      const st = await refreshPlaybackState();
      // sync Android notification progress ~every 4 ticks (≈1 s)
      if (++androidSyncCounter >= 4) { androidSyncCounter = 0; syncAndroid(); }
      if (st.finished) {
        await handleFinishedPlayback();
      }
    } catch (_) { /* ignore polling errors */ }
  }, 250);
}

async function togglePlay() {
  if (!nowPlaying.value) {
    if (libraryTracks.value.length) {
      await playTrackFrom('library', 0);
    }
    return;
  }
  if (isPlaying.value) {
    await invoke('playback_pause');
    stopTicker();
  } else {
    await invoke('playback_resume');
    startTicker();
  }
  await refreshPlaybackState();
  syncAndroid();
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
  await invoke('playback_seek', { position: pos });
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
    const res = await invoke<DeviceList>('get_output_devices');
    outputDevices.value = res.devices;
    currentDevice.value = res.current;
  }
  showDeviceMenu.value = !showDeviceMenu.value;
}

async function pickDevice(name: string) {
  const useDefault = name === currentDevice.value;
  await invoke('set_output_device', { name: useDefault ? null : name });
  currentDevice.value = useDefault ? null : name;
  showDeviceMenu.value = false;
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

  if (e.code === 'Space') {
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

const filteredTracks = computed(() => {
  const q = searchQuery.value.toLowerCase();
  if (!q) return libraryTracks.value;
  return libraryTracks.value.filter(t =>
    (t.title && t.title.toLowerCase().includes(q)) ||
    (t.artist && t.artist.toLowerCase().includes(q)) ||
    (t.album && t.album.toLowerCase().includes(q))
  );
});

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
  };
  editingTrack.value = track;
}

async function saveTrack() {
  if (!editingTrack.value) return;
  await invoke('update_track', {
    id: editingTrack.value.id,
    title: editForm.value.title || null,
    artist: editForm.value.artist || null,
    album: editForm.value.album || null,
    trackNumber: editForm.value.track_number || null,
  });
  editingTrack.value = null;
  await loadLibrary();
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
  await invoke('identify_tracks', { ids });
}

async function identifySingle(track: Track) {
  identifyResults.value = [];
  identifyCurrent.value = 0;
  identifyTotal.value = 1;
  identifyRunning.value = true;
  identifyMinimized.value = false;
  identifyDone.value = false;
  await invoke('identify_tracks', { ids: [track.id] });
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

onMounted(() => {
  document.addEventListener('click', onDocClick);
  document.addEventListener('keydown', onKeyDown);
  loadLibrary();
  loadRecent();
  loadPlaylists();
  loadSmartPlaylists();
  invoke<DeviceSettings>('get_device_settings')
    .then((cfg) => {
      if (cfg?.emoji) deviceEmoji.value = cfg.emoji;
    })
    .catch(() => {});
  
  // Listen for app coming back to foreground (Android)
  listen('tauri://resumed', async () => {
    const st = await refreshPlaybackState();
    if (st.finished) {
      await handleFinishedPlayback();
      return;
    }
    if (st.playing && !ticker) {
      startTicker();
    }
  });
  
  listen('library-changed', () => { loadLibrary(); loadRecent(); });
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
      loadLibrary(); loadPlaylists(); loadSmartPlaylists(); loadHistory();
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
    if (action === 'play')       { if (!isPlaying.value) await togglePlay(); }
    else if (action === 'pause') { if (isPlaying.value) await togglePlay(); }
    else if (action === 'next')  { await playNext(); }
    else if (action === 'prev')  { await playPrev(); }
    else if (action.startsWith('seek:')) {
      const pos = parseFloat(action.slice(5));
      if (!isNaN(pos)) {
        currentTime.value = Math.round(pos);
        await invoke('playback_seek', { position: pos });
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
  stopTicker();
});
</script>

<template>
  <div class="app">
    <!-- Sidebar -->
    <aside class="sidebar">
      <!-- <div class="brand">
        <svg viewBox="0 0 24 24" fill="#1db954" width="32" height="32">
          <path d="M12 2C6.477 2 2 6.477 2 12s4.477 10 10 10 10-4.477 10-10S17.523 2 12 2zm4.586 14.424a.622.622 0 0 1-.857.207c-2.348-1.435-5.304-1.76-8.785-.964a.622.622 0 1 1-.277-1.215c3.809-.87 7.076-.496 9.712 1.115a.622.622 0 0 1 .207.857zm1.223-2.722a.78.78 0 0 1-1.072.257c-2.687-1.652-6.785-2.131-9.965-1.166a.78.78 0 1 1-.453-1.492c3.632-1.102 8.147-.568 11.233 1.329a.78.78 0 0 1 .257 1.072zm.105-2.835C14.692 8.95 9.375 8.775 6.297 9.71a.937.937 0 1 1-.543-1.793c3.541-1.073 9.43-.865 13.152 1.337a.937.937 0 0 1-.992 1.613z"/>
        </svg>
        <span class="brand-name">Player</span>
      </div> -->

      <nav>
        <a class="nav-item" :class="{ active: activeNav === 'home' }" @click.prevent="activeNav = 'home'" href="#">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z"/></svg>
          Home
        </a>
        <a class="nav-item" :class="{ active: activeNav === 'search' }" @click.prevent="activeNav = 'search'" href="#">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M15.5 14h-.79l-.28-.27A6.471 6.471 0 0 0 16 9.5 6.5 6.5 0 1 0 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z"/></svg>
          Search
        </a>
        <a class="nav-item" :class="{ active: activeNav === 'library' }" @click.prevent="activeNav = 'library'" href="#">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M4 6H2v14c0 1.1.9 2 2 2h14v-2H4V6zm16-4H8c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm-1 9H9V9h10v2zm-4 4H9v-2h6v2zm4-8H9V5h10v2z"/></svg>
          Your Library
        </a>
        <a class="nav-item" :class="{ active: activeNav === 'playlists' }" @click.prevent="activeNav = 'playlists'; playlistView = null; playlistTab = 'regular'" href="#">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M15 6H3v2h12V6zm0 4H3v2h12v-2zM3 16h8v-2H3v2zM17 6v8.18c-.31-.11-.65-.18-1-.18-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3V8h3V6h-5z"/></svg>
          Playlists
        </a>
        <a class="nav-item" :class="{ active: activeNav === 'playlists' && playlistTab === 'smart' }" @click.prevent="activeNav = 'playlists'; playlistTab = 'smart'; playlistView = null; editingSP = null; smartView = null" href="#">
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
        <a class="nav-item" :class="{ active: activeNav === 'discovery' }" @click.prevent="activeNav = 'discovery'" href="#">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8 3 3 3-3a4.237 4.237 0 0 0-6 0zm-4-4 2 2a7.074 7.074 0 0 1 10 0l2-2C15.14 9.14 8.87 9.14 5 13z"/></svg>
          Devices
          <span v-if="peers.length" class="peer-badge">{{ peers.length }}</span>
        </a>
        <a class="nav-item" href="#" @click.prevent="openDataDir">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M10 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z"/></svg>
          Open Data Folder
        </a>
        <a class="nav-item" href="#" @click.prevent="doReindex">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M17.65 6.35A7.958 7.958 0 0 0 12 4c-4.42 0-7.99 3.58-7.99 8s3.57 8 7.99 8c3.73 0 6.84-2.55 7.73-6h-2.08A5.99 5.99 0 0 1 12 18c-3.31 0-6-2.69-6-6s2.69-6 6-6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z"/></svg>
          Reindex
        </a>
        <a class="nav-item" href="#" @click.prevent="startIdentify">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="m22 2-2.5 1.4L17.1 2l1.4 2.5L17.1 7l2.4-1.4L22 7l-1.4-2.5zm-7.63 5.29a.996.996 0 0 0-1.41 0L1.29 18.96a.996.996 0 0 0 0 1.41l2.34 2.34c.39.39 1.02.39 1.41 0L16.7 11.05a.996.996 0 0 0 0-1.41l-2.33-2.35zM5.21 19.38l-1.59-1.59 8.93-8.93 1.59 1.59-8.93 8.93z"/></svg>
          Identify
        </a>
      </nav>
    </aside>

    <!-- Main -->
    <main class="main">
      <header class="topbar">
        <div class="nav-arrows">
          <button class="arrow-btn">&lsaquo;</button>
          <button class="arrow-btn">&rsaquo;</button>
        </div>
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

      <div class="content">
        <!-- Home view -->
        <template v-if="activeNav === 'home'">
          <section>
            <div class="section-head">
              <h2>Recently played</h2>
              <a class="show-all" href="#" @click.prevent="activeNav = 'history'; loadHistory()">Show all</a>
            </div>
            <div v-if="recentTracks.length === 0 && libraryTracks.length === 0" class="library-empty">No tracks yet.</div>
            <div v-else class="card-list">
              <div v-for="(track, idx) in (recentTracks.length ? recentTracks : libraryTracks.slice(0, 12))" :key="track.id + '-' + idx"
                class="card" :class="rarityClass(track.rarity)" :style="rarityVars(track.rarity)"
                @click="playTrackFrom('recent', idx)">
                <div class="cover" :style="covers[track.id]
                  ? `background-image: url(${covers[track.id]}); background-size: cover; background-position: center`
                  : `background: linear-gradient(135deg, ${hashToColors(track.file_hash)[0]}, ${hashToColors(track.file_hash)[1]})`">
                  <div class="hover-play">
                    <button class="green-circle">
                      <svg viewBox="0 0 24 24" fill="black" width="18" height="18"><path d="M8 5v14l11-7z"/></svg>
                    </button>
                  </div>
                </div>
                <div class="card-title">{{ track.title || track.path }}</div>
                <div class="card-artist">{{ track.artist || 'Unknown' }}</div>
              </div>
            </div>
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
                v-model="searchQuery"
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
                  class="track-row" :class="rarityClass(track.rarity)"
                  :style="rarityVars(track.rarity)"
                  @click="playTrackFrom('library', libraryFlatList.indexOf(track))"
                >
                  <div class="track-cover-sm" :style="covers[track.id]
                    ? `background-image: url(${covers[track.id]}); background-size: cover; background-position: center`
                    : `background: linear-gradient(135deg, ${hashToColors(track.file_hash)[0]}, ${hashToColors(track.file_hash)[1]})`"
                  />
                  <span class="track-num">{{ track.track_number ?? '–' }}</span>
                  <div class="track-info">
                    <span class="track-title">{{ track.title || track.path }}</span>
                    <span class="track-album">{{ track.album || '' }}</span>
                  </div>
                  <button class="icon-btn edit-btn" title="Identify" @click.stop="identifySingle(track)">
                    <svg viewBox="0 0 24 24" fill="currentColor" width="14" height="14"><path d="m22 2-2.5 1.4L17.1 2l1.4 2.5L17.1 7l2.4-1.4L22 7l-1.4-2.5zm-7.63 5.29a.996.996 0 0 0-1.41 0L1.29 18.96a.996.996 0 0 0 0 1.41l2.34 2.34c.39.39 1.02.39 1.41 0L16.7 11.05a.996.996 0 0 0 0-1.41l-2.33-2.35zM5.21 19.38l-1.59-1.59 8.93-8.93 1.59 1.59-8.93 8.93z"/></svg>
                  </button>
                  <button class="icon-btn edit-btn" title="Edit" @click.stop="openEditor(track)">
                    <svg viewBox="0 0 24 24" fill="currentColor" width="14" height="14"><path d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04a1.003 1.003 0 0 0 0-1.42l-2.34-2.33a1.003 1.003 0 0 0-1.42 0l-1.83 1.83 3.75 3.75 1.84-1.83z"/></svg>
                  </button>
                  <span title="Play Count" v-if="track.play_count > 0" style="margin-right: 8px; font-size: 13px; color: #a7a7a7; user-select: none;">
                    ▶ {{ track.play_count }}
                  </span>
                  <button class="icon-btn like-btn" @click.stop="toggleLike(track)" style="margin-right: 8px;">
                    <svg v-if="track.is_liked" viewBox="0 0 24 24" fill="#1db954" width="16" height="16"><path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"/></svg>
                    <svg v-else viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M16.5 3c-1.74 0-3.41.81-4.5 2.09C10.91 3.81 9.24 3 7.5 3 4.42 3 2 5.42 2 8.5c0 3.78 3.4 6.86 8.55 11.54L12 21.35l1.45-1.32C18.6 15.36 22 12.28 22 8.5 22 5.42 19.58 3 16.5 3zm-4.4 15.55l-.1.1-.1-.1C7.14 14.24 4 11.39 4 8.5 4 6.5 5.5 5 7.5 5c1.54 0 3.04.99 3.57 2.36h1.87C13.46 5.99 14.96 5 16.5 5c2 0 3.5 1.5 3.5 3.5 0 2.89-3.14 5.74-7.9 10.05z"/></svg>
                  </button>
                  <button class="icon-btn edit-btn" title="Add to playlist" style="margin-right: 8px;" @click.stop="openAddToPlaylistMenu($event, track)">
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
            <h2>Search</h2>
            <p style="color:#a7a7a7">Search coming soon.</p>
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
                :class="rarityClass(entry.track.rarity)"
                :style="rarityVars(entry.track.rarity)"
                @click="playTrackFrom('library', libraryFlatList.findIndex(t => t.id === entry.track.id))"
              >
                <div class="track-cover-sm" :style="covers[entry.track.id]
                  ? `background-image: url(${covers[entry.track.id]}); background-size: cover; background-position: center`
                  : `background: linear-gradient(135deg, ${hashToColors(entry.track.file_hash)[0]}, ${hashToColors(entry.track.file_hash)[1]})`"
                />
                <div class="track-info">
                  <span class="track-title">{{ entry.track.title || entry.track.path }}</span>
                  <span class="track-album">{{ entry.track.artist || 'Unknown' }}{{ entry.track.album ? ' · ' + entry.track.album : '' }}</span>
                </div>
                <span style="font-size:12px; color:#a7a7a7; margin-right:12px; white-space:nowrap;">{{ formatHistoryDate(entry.played_at) }}</span>
                <button class="icon-btn like-btn" @click.stop="toggleLike(entry.track)" style="margin-right:8px;">
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
                  <button v-if="playlistView.tracks.length" class="icon-btn" style="padding:6px 12px; font-size:13px;" @click="playFromPlaylist(playlistView!.tracks, 0)">▶ Play</button>
                </div>
                <div v-if="playlistView.tracks.length === 0" class="library-empty">No tracks yet. Right-click any track to add.</div>
                <div v-else class="track-list">
                  <div
                    v-for="(track, idx) in playlistView.tracks"
                    :key="track.id"
                    class="track-row"
                    :class="rarityClass(track.rarity)"
                    :style="rarityVars(track.rarity)"
                    @click="playFromPlaylist(playlistView!.tracks, idx)"
                  >
                    <div class="track-cover-sm" :style="covers[track.id]
                      ? `background-image: url(${covers[track.id]}); background-size: cover; background-position: center`
                      : `background: linear-gradient(135deg, ${hashToColors(track.file_hash)[0]}, ${hashToColors(track.file_hash)[1]})`"
                    />
                    <div class="track-info">
                      <span class="track-title">{{ track.title || track.path }}</span>
                      <span class="track-album">{{ track.artist || 'Unknown' }}{{ track.album ? ' · ' + track.album : '' }}</span>
                    </div>
                    <button class="icon-btn" style="margin-right:8px;" title="Remove from playlist" @click.stop="removeTrackFromPlaylist(playlistView!.id, track.id)">
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
                  <button class="icon-btn" style="padding:6px 12px; font-size:13px;" @click="showNewPlaylistInput = !showNewPlaylistInput">+ New</button>
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
                      class="library-search"
                      style="font-size:16px; font-weight:600; min-width:140px; max-width:260px; padding:4px 10px;"
                      v-model="editingSP.name"
                      @input="saveSP()"
                      @click.stop
                    />
                  </div>
                  <span style="font-size:12px; color:#a7a7a7;">{{ smartPlaylistTracks(editingSP).length }} tracks</span>
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
                      <option value="extension">Extension</option>
                      <option value="year">Year</option>
                      <option value="play_count">Play count</option>
                      <option value="is_liked">Is liked</option>
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
                    <template v-else>
                      <div class="sp-multiselect">
                        <label v-for="v in spUniqueValues(rule.field)" :key="v" class="sp-chip" :class="{ selected: spIsSelected(rule, v) }" @click="spToggleValue(rule, v)">{{ v }}</label>
                        <span v-if="spUniqueValues(rule.field).length === 0" style="color:#777; font-size:12px;">No values in library</span>
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
                    :class="rarityClass(track.rarity)"
                    :style="rarityVars(track.rarity)"
                    @click="playTrackFrom('library', libraryFlatList.findIndex(t => t.id === track.id))"
                  >
                    <div class="track-cover-sm" :style="covers[track.id]
                      ? `background-image: url(${covers[track.id]}); background-size: cover; background-position: center`
                      : `background: linear-gradient(135deg, ${hashToColors(track.file_hash)[0]}, ${hashToColors(track.file_hash)[1]})`"
                    />
                    <div class="track-info">
                      <span class="track-title">{{ track.title || track.path }}</span>
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
                    <button class="icon-btn" style="padding:6px 12px; font-size:13px;" @click="editingSP = { ...smartView! }; smartView = null">Edit</button>
                    <button v-if="smartPlaylistTracks(smartView).length" class="icon-btn" style="padding:6px 12px; font-size:13px;" @click="playFromPlaylist(smartPlaylistTracks(smartView!), 0)">▶ Play</button>
                  </div>
                </div>
                <div v-if="smartPlaylistTracks(smartView).length === 0" class="library-empty">No tracks match this smart playlist.</div>
                <div v-else class="track-list">
                  <div
                    v-for="(track, idx) in smartPlaylistTracks(smartView)"
                    :key="track.id"
                    class="track-row"
                    :class="rarityClass(track.rarity)"
                    :style="rarityVars(track.rarity)"
                    @click="playFromPlaylist(smartPlaylistTracks(smartView!), idx)"
                  >
                    <div class="track-cover-sm" :style="covers[track.id]
                      ? `background-image: url(${covers[track.id]}); background-size: cover; background-position: center`
                      : `background: linear-gradient(135deg, ${hashToColors(track.file_hash)[0]}, ${hashToColors(track.file_hash)[1]})`"
                    />
                    <div class="track-info">
                      <span class="track-title">{{ track.title || track.path }}</span>
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
                  <button class="icon-btn" style="padding:6px 12px; font-size:13px;" @click="showNewSPInput = !showNewSPInput">+ New</button>
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

        <!-- Discovery view -->
        <template v-else-if="activeNav === 'discovery'">
          <section>
            <div class="library-header">
              <h2>Devices on Network</h2>
              <div style="display:flex; align-items:center; gap:12px">
                <button class="sync-toggle" @click="openDeviceSettings" title="Device settings">
                  <span style="font-size:16px; line-height:1">{{ deviceEmoji }}</span>
                  Settings
                </button>
                <span style="color:#a7a7a7; font-size:12px">mDNS · auto discovery</span>
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
            <div v-if="!peers.length" class="discovery-empty">
              <svg viewBox="0 0 24 24" fill="currentColor" width="48" height="48" style="color:#535353"><path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8 3 3 3-3a4.237 4.237 0 0 0-6 0zm-4-4 2 2a7.074 7.074 0 0 1 10 0l2-2C15.14 9.14 8.87 9.14 5 13z"/></svg>
              <p>No other instances found</p>
              <p style="font-size:12px; color:#535353">Make sure devices are on the same Wi-Fi network</p>
            </div>
            <div v-else class="peer-list">
              <div v-for="peer in peers" :key="peer.host" class="peer-item">
                <div class="peer-icon">{{ peer.device_emoji || syncProgress[peer.name]?.device_emoji || '🎵' }}</div>
                <div class="peer-info">
                  <span class="peer-name">{{ peer.device_name || peerDeviceNames[peer.name] || peer.name }}</span>
                  <span class="peer-addr">{{ peer.host }}:{{ peer.port }}</span>
                  <!-- sync progress for this peer -->
                  <template v-if="syncProgress[peer.name]">
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
                  </template>
                </div>
                <button
                  v-if="syncEnabled"
                  class="sync-now-btn"
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
      </div>
    </main>

    <!-- Add to playlist menu -->
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

    <!-- Edit modal -->
    <Transition name="modal">
      <div v-if="editingTrack" class="modal-overlay" @click.self="editingTrack = null">
        <div class="modal">
          <div class="modal-header">
            <h3>Edit Track</h3>
            <button class="icon-btn" @click="editingTrack = null">
              <svg viewBox="0 0 24 24" fill="currentColor" width="18" height="18"><path d="M19 6.41 17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/></svg>
            </button>
          </div>
          <div class="modal-body">
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
                  @click.prevent="pickDevice(dev.name)"
                >
                  <span class="device-check">{{ dev.name === currentDevice ? '✓' : '' }}</span>
                  <span class="device-name">{{ dev.name }}</span>
                </a>
              </div>
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
              <div class="detail-cover detail-cover-3d"
                :style="[
                  covers[nowPlaying.id]
                    ? `background-image: url(${covers[nowPlaying.id]}); background-size: cover; background-position: center`
                    : `background: linear-gradient(135deg, ${currentTrack.colors[0]}, ${currentTrack.colors[1]})`,
                  { transform: cardTransform, boxShadow: cardShadow }
                ]"
              >
                <div class="card-gloss" :style="{ background: cardGloss }" />
                <div class="card-rainbow" :style="{ background: cardRainbow }" />
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
                <div class="detail-bar-fill" :style="`width:${displayProgressPercent}%`">
                  <div class="detail-bar-thumb" />
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
  font-size: 14px;
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
  font-size: 11px; font-weight: 700;
  min-width: 18px; height: 18px;
  border-radius: 9px;
  display: flex; align-items: center; justify-content: center;
  padding: 0 5px;
}

.discovery-empty {
  display: flex; flex-direction: column; align-items: center;
  gap: 12px; padding: 60px 0; color: #a7a7a7; font-size: 14px;
}
.peer-list { display: flex; flex-direction: column; gap: 8px; }
.peer-item {
  display: flex; align-items: center; gap: 14px;
  background: #181818; border-radius: 8px;
  padding: 14px 16px;
}
.peer-icon {
  width: 40px; height: 40px; border-radius: 6px; background: #282828;
  display: flex; align-items: center; justify-content: center;
  color: #a7a7a7; flex-shrink: 0;
}
.peer-info { display: flex; flex-direction: column; gap: 3px; min-width: 0; flex: 1; }
.peer-name { font-size: 14px; font-weight: 600; color: #fff; }
.peer-alias { font-size: 11px; color: #7b7b7b; }
.peer-addr { font-size: 12px; color: #a7a7a7; }

.sync-toggle {
  display: flex; align-items: center; gap: 6px;
  padding: 5px 12px; border-radius: 20px; border: 1px solid #535353;
  background: transparent; color: #a7a7a7; font-size: 12px; font-weight: 600;
  cursor: pointer; transition: all 0.15s;
}
.sync-toggle:hover { border-color: #fff; color: #fff; }
.sync-toggle.active { background: #1db954; border-color: #1db954; color: #000; }

.settings-body { gap: 12px; }
.settings-label { font-size: 12px; font-weight: 700; color: #a7a7a7; margin-top: 2px; }
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
.settings-error { color: #e9283e; font-size: 12px; }

.sync-now-btn {
  flex-shrink: 0; width: 32px; height: 32px; border-radius: 50%;
  background: #282828; border: none; color: #a7a7a7;
  display: flex; align-items: center; justify-content: center;
  cursor: pointer; transition: background 0.15s, color 0.15s;
}
.sync-now-btn:hover:not(:disabled) { background: #1db954; color: #000; }
.sync-now-btn:disabled { opacity: 0.4; cursor: default; }

.sync-bar-wrap {
  height: 3px; background: #333; border-radius: 2px; overflow: hidden; margin-top: 2px;
}
.sync-bar { height: 100%; background: #1db954; border-radius: 2px; transition: width 0.3s; }
.sync-status { font-size: 11px; color: #a7a7a7; }
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
section h2 { font-size: 22px; font-weight: 800; margin-bottom: 16px; }

.section-head {
  display: flex; align-items: baseline; justify-content: space-between;
  margin-bottom: 16px;
}
.section-head h2 { margin-bottom: 0; }
.history-list {
  display: flex; flex-direction: column; gap: 2px;
}
.show-all {
  font-size: 11px; font-weight: 700;
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
  font-size: 13px; font-weight: 700;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  margin-bottom: 4px;
}
.card-artist {
  font-size: 12px; color: #a7a7a7;
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
  font-size: 13px; font-weight: 600;
  overflow: hidden;
  mask-image: linear-gradient(to right, transparent 0%, black 5%, black 85%, transparent 100%);
  -webkit-mask-image: linear-gradient(to right, transparent 0%, black 5%, black 85%, transparent 100%);
}
.track-artist { font-size: 11px; color: #a7a7a7; margin-top: 3px; }

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
.time { font-size: 11px; color: #a7a7a7; min-width: 34px; text-align: center; }

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
  color: #fff; font-size: 13px; padding: 8px 12px;
  width: 220px; outline: none;
}
.library-search::placeholder { color: #777; }
.library-search:focus { outline: 1px solid #555; }
.library-empty {
  color: #a7a7a7; font-size: 14px; padding: 32px 0;
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
}
.playlist-menu-header {
  font-size: 11px; font-weight: 700; text-transform: uppercase;
  letter-spacing: .06em; color: #a7a7a7;
  padding: 10px 14px 6px;
  border-bottom: 1px solid #333;
}
.playlist-menu-empty {
  font-size: 12px; color: #777; padding: 10px 14px;
}
.playlist-menu-item {
  display: block; width: 100%; text-align: left;
  background: none; border: none; color: #e0e0e0;
  font-size: 13px; padding: 9px 14px; cursor: pointer;
}
.playlist-menu-item:hover { background: #333; }

/* Smart Playlists */
.sp-match-row {
  display: flex; align-items: center; gap: 8px;
  font-size: 13px; color: #a7a7a7; padding: 0 0 14px;
}
.sp-match-btn {
  background: transparent; border: 1px solid #535353; color: #a7a7a7;
  border-radius: 20px; padding: 4px 12px; font-size: 12px;
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
  color: #a7a7a7; font-size: 13px; padding: 5px 28px 5px 12px; cursor: pointer;
  appearance: none; -webkit-appearance: none;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='%23a7a7a7'%3E%3Cpath d='M7 10l5 5 5-5z'/%3E%3C/svg%3E");
  background-repeat: no-repeat; background-position: right 10px center;
  outline: none; transition: border-color .15s, color .15s;
}
.sp-select:focus { border-color: #fff; color: #fff; }
.sp-select:hover { border-color: #fff; color: #fff; }
.sp-op-label { font-size: 12px; color: #a7a7a7; white-space: nowrap; }
.sp-text-input { flex: 1; min-width: 100px; }
.sp-num-input { width: 80px; }
.sp-add-rule-btn {
  background: transparent; border: 1px solid #535353; color: #a7a7a7;
  border-radius: 20px; padding: 5px 14px; font-size: 12px; font-weight: 600;
  cursor: pointer; transition: all .15s;
  align-self: flex-start;
}
.sp-add-rule-btn:hover { border-color: #fff; color: #fff; }
.sp-multiselect {
  display: flex; flex-wrap: wrap; gap: 6px; align-items: center; flex: 1;
}
.sp-chip {
  background: #282828; border: 1px solid #444; color: #a7a7a7;
  border-radius: 20px; padding: 3px 10px; font-size: 12px;
  cursor: pointer; transition: background .12s, color .12s, border-color .12s;
  user-select: none;
}
.sp-chip.selected { background: #1db954; border-color: #1db954; color: #000; font-weight: 600; }
.sp-chip:hover:not(.selected) { border-color: #888; color: #e0e0e0; }
.sp-preview-header {
  font-size: 11px; font-weight: 700; text-transform: uppercase;
  letter-spacing: .06em; color: #a7a7a7;
  padding: 0 0 10px; border-bottom: 1px solid #282828; margin-bottom: 8px;
}
.sp-preview { opacity: 0.9; }
.track-groups { display: flex; flex-direction: column; gap: 24px; }
.group-artist {
  font-size: 13px; font-weight: 700; color: #a7a7a7;
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
  font-size: 13px; color: #a7a7a7; flex-shrink: 0;
}
.track-info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.track-title {
  font-size: 14px; font-weight: 500; color: #fff;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.track-album {
  font-size: 12px; color: #a7a7a7;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.track-dur {
  font-size: 13px; color: #a7a7a7; flex-shrink: 0;
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
.modal-header h3 { font-size: 18px; font-weight: 700; }
.modal-body { padding: 8px 24px 16px; display: flex; flex-direction: column; gap: 14px; }
.field {
  display: flex; flex-direction: column; gap: 4px;
}
.field span {
  font-size: 12px; font-weight: 600; color: #a7a7a7; text-transform: uppercase; letter-spacing: .03em;
}
.field input {
  background: #3e3e3e; border: none; border-radius: 4px;
  color: #fff; font-size: 14px; padding: 10px 12px; outline: none;
}
.field input:focus { outline: 1px solid #1db954; }
.modal-footer {
  display: flex; justify-content: flex-end; gap: 10px;
  padding: 12px 24px 20px;
}
.btn-secondary {
  background: transparent; border: 1px solid #727272; border-radius: 20px;
  color: #fff; font-size: 13px; font-weight: 700;
  padding: 8px 24px; cursor: pointer;
}
.btn-secondary:hover { border-color: #fff; }
.btn-primary {
  background: #1db954; border: none; border-radius: 20px;
  color: #000; font-size: 13px; font-weight: 700;
  padding: 8px 28px; cursor: pointer;
}
.btn-primary:hover { background: #1ed760; transform: scale(1.02); }

.modal-enter-active, .modal-leave-active { transition: opacity .15s; }
.modal-enter-from, .modal-leave-to { opacity: 0; }

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
  font-size: 12px;
  font-weight: 700;
  color: #a7a7a7;
  text-transform: uppercase;
  letter-spacing: .04em;
}
.device-list {
  max-height: 240px;
  overflow-y: auto;
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
  font-size: 14px;
  cursor: pointer;
}
.device-item:hover { background: #3e3e3e; }
.device-check {
  width: 18px;
  text-align: center;
  color: #1db954;
  font-size: 14px;
  flex-shrink: 0;
}
.device-name {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
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
  font-size: 11px;
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
  font-size: 13px; font-weight: 600; color: #fff;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.queue-item.active .queue-item-title { color: #1db954; }
.queue-item-artist {
  font-size: 11px; color: #a7a7a7;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.queue-item-dur {
  font-size: 12px; color: #a7a7a7; flex-shrink: 0;
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
  font-size: 13px;
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
  font-size: 13px;
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
.powered-by { font-size: 11px; font-weight: 400; color: #888; margin-left: 6px; }
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
  font-size: 12px;
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
  font-size: 13px;
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
  font-size: 13px;
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
.index-empty { padding: 40px 20px; text-align: center; color: #a7a7a7; font-size: 14px; }

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
  font-size: 11px;
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
  font-size: 13px;
}
.ls-icon { font-size: 16px; flex-shrink: 0; }
.ls-source { font-weight: 600; color: #fff; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ls-time { font-size: 11px; color: #666; flex-shrink: 0; }

.ls-badge {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 12px;
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
  font-size: 12px;
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
  font-size: 12px;
  color: #ccc;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.ls-file-icon { color: #777; flex-shrink: 0; font-style: normal; }

/* Hidden on desktop; appears above footer on mobile/tablet */
.mobile-seek-wrap { display: none; }

/* ── Responsive: tablets and small screens ── */
@media (max-width: 768px) {
  .app {
    grid-template-columns: 1fr;
    grid-template-rows: auto 1fr 130px;
  }

  .sidebar {
    grid-row: 1 / 2;
    flex-direction: row;
    padding: 8px 12px;
    padding-top: calc(8px + env(safe-area-inset-top));
    gap: 0;
    overflow-x: auto;
    overflow-y: hidden;
    border-bottom: 1px solid #282828;
  }
  .sidebar nav {
    display: flex;
    gap: 2px;
    width: 100%;
  }
  .sidebar .sidebar-divider { display: none; }
  .nav-item {
    gap: 6px;
    padding: 8px 10px;
    font-size: 12px;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .nav-item svg { width: 18px; height: 18px; }

  .main { grid-row: 2 / 3; }

  .content { padding: 0 14px 24px; }

  .topbar { padding: 10px 14px; }

  .card-list {
    grid-template-columns: repeat(auto-fill, minmax(130px, 1fr));
    gap: 14px;
  }
  .cover { aspect-ratio: 1; height: auto; }

  .library-header { flex-direction: column; gap: 10px; align-items: stretch; }
  .library-search { width: 100%; }

  .track-row { gap: 8px; padding: 6px 4px; }
  .track-cover-sm { width: 32px; height: 32px; }
  .track-num { display: none; }
  .edit-btn { opacity: 1; }

  .player {
    grid-column: 1;
    grid-row: 3 / 4;
    display: flex;
    flex-direction: column;
    padding: 0px 10px;
    justify-content: center;
    /* padding-bottom: env(safe-area-inset-bottom); */
    gap: 8px;
  }
  .player-left {
    display: flex;
    align-items: center;
    gap: 8px;
    justify-content: flex-start;
    order: -1;
    min-width: 0;
    padding: 0 15px;
    width: calc(100%);
  }
  .player-right { display: none; }
  .player-left .thumb { width: 40px; height: 40px; }
  .player-left .track-meta { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex-grow: 1; }
  .track-name { font-size: 12px; font-weight: 600; color: #fff; }
  .track-artist { font-size: 10px; color: #a7a7a7; }
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
    bottom: 125px;
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
  .nav-item svg { width: 16px; height: 16px; }
  .nav-item { padding: 6px 8px; font-size: 11px; }

  .card-list { grid-template-columns: repeat(auto-fill, minmax(110px, 1fr)); gap: 10px; }

  .track-title { font-size: 13px; }
  .track-album { display: none; }
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
}

.detail-sheet {
  width: 100%;
  max-width: 480px;
  max-height: 95vh;
  overflow-y: auto;
  background: #121212;
  border-radius: 20px 20px 0 0;
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
  aspect-ratio: 1 / 1;
  flex-shrink: 0;
  width: min(28vh, 260px, 80vw);
  height: auto;
  align-self: center;
  perspective: 600px;
  cursor: grab;
  user-select: none;
}

.detail-cover {
  width: 100%;
  height: 100%;
  border-radius: 12px;
  object-fit: cover;
  position: relative;
  transform-style: preserve-3d;
  will-change: transform;
}

.detail-cover-3d {
  transform-origin: center center;
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
  font-size: 18px; font-weight: 700; color: #fff;
  overflow: hidden;
  flex: 1;
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
  font-size: 14px; color: #a7a7a7;
  flex: 1;
  overflow: hidden;
  mask-image: linear-gradient(to right, transparent 0%, black 5%, black 85%, transparent 100%);
  -webkit-mask-image: linear-gradient(to right, transparent 0%, black 5%, black 85%, transparent 100%);
}

.detail-seek-wrap { display: flex; flex-direction: column; gap: 6px; width: 100%; }

.detail-bar {
  height: 28px; display: flex; align-items: center; cursor: pointer;
}

.detail-bar::before {
  content: '';
  position: absolute; left: 24px; right: 24px; height: 4px;
  background: #3a3a3a; border-radius: 999px;
  pointer-events: none;
}

.detail-seek-wrap { position: relative; }

.detail-bar {
  position: relative;
  height: 28px;
}

.detail-bar-fill {
  position: absolute;
  top: 50%; left: 0;
  transform: translateY(-50%);
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
  position: absolute;
  inset: 50% 0;
  transform: translateY(-50%);
  height: 4px;
  background: #3a3a3a;
  border-radius: 999px;
}

.detail-time-row {
  display: flex;
  justify-content: space-between;
  font-size: 12px;
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