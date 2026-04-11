<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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

const recentlyPlayed = [
  { id: 1, title: "Binary Rhapsody", artist: "Ada Lovelace", colors: ["#c2185b", "#7b1fa2"] },
  { id: 2, title: "Code Symphony", artist: "Grace Hopper", colors: ["#00acc1", "#283593"] },
  { id: 3, title: "The Enigma Variati...", artist: "Alan Turing", colors: ["#546e7a", "#37474f"] },
  { id: 4, title: "Kernel Blues", artist: "Linus Torvalds", colors: ["#8e24aa", "#c2185b"] },
  { id: 5, title: "World Wide Web ...", artist: "Tim Berners-Lee", colors: ["#d81b60", "#e64a19"] },
  { id: 6, title: "Apollo Overture", artist: "Margaret Hamilton", colors: ["#3949ab", "#7b1fa2"] },
];

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

/* ── Queue state ── */
type QueueSource = 'recent' | 'library';
const queueSource = ref<QueueSource>('library');
const queueSourceIndex = ref(0);       // index into source list of the LAST item pushed
const queue = ref<Track[]>([]);         // upcoming tracks (max 5 visible)
const nowPlaying = ref<Track | null>(null);
const showQueueMenu = ref(false);

const currentTrack = computed(() => {
  if (nowPlaying.value) {
    return {
      title: nowPlaying.value.title || nowPlaying.value.path,
      artist: nowPlaying.value.artist || 'Unknown',
      colors: ['#00acc1', '#283593'],
    };
  }
  return { title: 'No track', artist: '', colors: ['#282828', '#181818'] };
});

const progressPercent = computed(() => (duration.value > 0 ? (currentTime.value / duration.value) * 100 : 0));

/** Return the full ordered list for a given source */
function sourceList(src: QueueSource): Track[] {
  if (src === 'recent') return libraryTracks.value.slice(0, 12);
  // 'library' – flattened in grouped order
  const flat: Track[] = [];
  for (const [, tracks] of groupedByArtist.value) flat.push(...tracks);
  return flat.length ? flat : libraryTracks.value;
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
    queueSourceIndex.value = (queueSourceIndex.value + 1) % list.length;
    queue.value.push(list[queueSourceIndex.value]);
  }
}

/** Start playing a specific track from a given source list at a given index */
async function playTrackFrom(src: QueueSource, index: number) {
  const list = sourceList(src);
  if (!list.length) return;
  queueSource.value = src;
  const track = list[index];
  nowPlaying.value = track;
  duration.value = track.duration_secs || 0;
  currentTime.value = 0;
  isPlaying.value = true;
  // rebuild queue starting after this index
  queueSourceIndex.value = index;
  queue.value = [];
  refillQueue();
  await invoke('playback_play', { path: track.path });
  startTicker();
}

/** Advance to next track in queue */
async function playNext() {
  if (!queue.value.length) {
    isPlaying.value = false;
    stopTicker();
    await invoke('playback_stop');
    return;
  }
  const next = queue.value.shift()!;
  nowPlaying.value = next;
  duration.value = next.duration_secs || 0;
  currentTime.value = 0;
  refillQueue();
  if (!isPlaying.value) {
    isPlaying.value = true;
  }
  await invoke('playback_play', { path: next.path });
  startTicker();
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

function formatTime(s: number) {
  const m = Math.floor(s / 60);
  return `${m}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
}

let ticker: ReturnType<typeof setInterval> | null = null;

function stopTicker() {
  if (ticker) { clearInterval(ticker); ticker = null; }
}

function startTicker() {
  stopTicker();
  ticker = setInterval(async () => {
    try {
      const st = await invoke<PlaybackStatus>('playback_status');
      currentTime.value = Math.floor(st.position);
      if (st.duration > 0) duration.value = Math.floor(st.duration);
      if (st.finished) {
        // track ended — advance
        if (repeatMode.value === 2) {
          // repeat-one
          if (nowPlaying.value) {
            await invoke('playback_play', { path: nowPlaying.value.path });
          }
        } else {
          await playNext();
        }
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
  isPlaying.value = !isPlaying.value;
  if (isPlaying.value) {
    await invoke('playback_resume');
    startTicker();
  } else {
    await invoke('playback_pause');
    stopTicker();
  }
}

async function seek(e: MouseEvent) {
  const el = e.currentTarget as HTMLElement;
  const ratio = (e.clientX - el.getBoundingClientRect().left) / el.offsetWidth;
  const pos = Math.max(0, Math.min(1, ratio)) * duration.value;
  currentTime.value = Math.round(pos);
  await invoke('playback_seek', { position: pos });
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

async function doReindex() {
  await invoke('reindex');
  await loadLibrary();
}

onMounted(() => {
  document.addEventListener('click', onDocClick);
  loadLibrary();
  listen('library-changed', () => loadLibrary());
});
onUnmounted(() => {
  document.removeEventListener('click', onDocClick);
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
        <a class="nav-item" href="#" @click.prevent="invoke('open_data_dir')">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M10 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z"/></svg>
          Open Data Folder
        </a>
        <a class="nav-item" href="#" @click.prevent="doReindex">
          <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M17.65 6.35A7.958 7.958 0 0 0 12 4c-4.42 0-7.99 3.58-7.99 8s3.57 8 7.99 8c3.73 0 6.84-2.55 7.73-6h-2.08A5.99 5.99 0 0 1 12 18c-3.31 0-6-2.69-6-6s2.69-6 6-6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z"/></svg>
          Reindex
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
            <h2>Recently played</h2>
            <div v-if="libraryTracks.length === 0" class="library-empty">No tracks yet.</div>
            <div v-else class="card-list">
              <div v-for="(track, idx) in libraryTracks.slice(0, 12)" :key="track.id"
                class="card" :class="rarityClass(track.rarity)" :style="rarityVars(track.rarity)"
                @click="playTrackFrom('recent', idx)">
                <div class="cover" :style="covers[track.id]
                  ? `background-image: url(${covers[track.id]}); background-size: cover; background-position: center`
                  : 'background: linear-gradient(135deg, #1db954, #191414)'">
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
                  @click="playTrackFrom('library', libraryTracks.indexOf(track))"
                >
                  <div class="track-cover-sm" :style="covers[track.id]
                    ? `background-image: url(${covers[track.id]}); background-size: cover; background-position: center`
                    : 'background: linear-gradient(135deg, #1db954, #191414)'"
                  />
                  <span class="track-num">{{ track.track_number ?? '–' }}</span>
                  <div class="track-info">
                    <span class="track-title">{{ track.title || track.path }}</span>
                    <span class="track-album">{{ track.album || '' }}</span>
                  </div>
                  <button class="icon-btn edit-btn" title="Edit" @click.stop="openEditor(track)">
                    <svg viewBox="0 0 24 24" fill="currentColor" width="14" height="14"><path d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04a1.003 1.003 0 0 0 0-1.42l-2.34-2.33a1.003 1.003 0 0 0-1.42 0l-1.83 1.83 3.75 3.75 1.84-1.83z"/></svg>
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
      </div>
    </main>

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

    <!-- Player bar -->
    <footer class="player">
      <!-- Left: track info -->
      <div class="player-left">
        <div class="thumb" :style="nowPlaying && covers[nowPlaying.id]
          ? `background-image: url(${covers[nowPlaying.id]}); background-size: cover; background-position: center`
          : `background: linear-gradient(135deg, ${currentTrack.colors[0]}, ${currentTrack.colors[1]})`" />
        <div class="track-meta">
          <div class="track-name">{{ currentTrack.title }}</div>
          <div class="track-artist">{{ currentTrack.artist }}</div>
        </div>
        <button class="icon-btn" :class="{ green: isLiked }" @click="isLiked = !isLiked">
          <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16"><path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"/></svg>
        </button>
      </div>

      <!-- Center: controls -->
      <div class="player-center">
        <div class="ctrl-row">
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
            <div class="bar-fill" :style="`width:${progressPercent}%`">
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
                    : 'background: linear-gradient(135deg, #1db954, #191414)'" />
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
                    : 'background: linear-gradient(135deg, #1db954, #191414)'" />
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
</template>

<style>
*, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
html, body, #app { height: 100%; overflow: hidden; }
body { background: #000; font-family: 'Helvetica Neue', Helvetica, Arial, sans-serif; }
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
}
.nav-item:hover { color: #fff; }
.nav-item.active { color: #fff; }

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
  overflow-y: scroll;
  background: linear-gradient(180deg, #4a2d8a 0%, #1f1b3a 30%, #121212 58%);
}

.topbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 24px;
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
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
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
</style>