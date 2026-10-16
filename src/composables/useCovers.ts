import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { Track } from "../types";

type CoverRequest = Pick<Track, "id">;

const MAX_CACHED_COVERS = 100;
const MAX_CONCURRENT_COVER_LOADS = 3;

const covers = ref<Record<number, string | null>>({});
const coverLoadQueue: CoverRequest[] = [];
const queuedCoverIds = new Set<number>();
const coverLastUsed = new Map<number, number>();
let activeCoverLoads = 0;

function trimCoverCache() {
  while (coverLastUsed.size > MAX_CACHED_COVERS) {
    const oldest = coverLastUsed.keys().next().value as number | undefined;
    if (oldest === undefined) return;
    coverLastUsed.delete(oldest);
    delete covers.value[oldest];
  }
}

function pumpCoverQueue() {
  while (activeCoverLoads < MAX_CONCURRENT_COVER_LOADS && coverLoadQueue.length) {
    const track = coverLoadQueue.shift()!;
    queuedCoverIds.delete(track.id);
    activeCoverLoads += 1;
    invoke<string | null>('get_track_cover', { id: track.id })
      .then((url) => {
        covers.value[track.id] = url;
        coverLastUsed.delete(track.id);
        coverLastUsed.set(track.id, Date.now());
        trimCoverCache();
      })
      .catch(() => {
        covers.value[track.id] = null;
      })
      .finally(() => {
        activeCoverLoads -= 1;
        pumpCoverQueue();
      });
  }
}

function fetchCovers(tracks: CoverRequest[]) {
  for (const track of tracks) {
    if (covers.value[track.id] !== undefined || queuedCoverIds.has(track.id)) continue;
    queuedCoverIds.add(track.id);
    coverLoadQueue.push(track);
  }
  pumpCoverQueue();
}

function clearCovers() {
  covers.value = {};
  coverLastUsed.clear();
  coverLoadQueue.length = 0;
  queuedCoverIds.clear();
}

export function useCovers() {
  return { covers, fetchCovers, clearCovers };
}
