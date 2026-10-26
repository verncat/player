import type { Track } from '../types';
import type { ResourceIdentity } from './types';
const ids = new Map<string, number>();
export function pluginTrack(resource: ResourceIdentity, sourceToken = '', generation = 0): Track {
  const key = JSON.stringify([resource.pluginId, resource.resourceId]);
  if (!ids.has(key)) ids.set(key, -1 - ids.size);
  return {
    id: ids.get(key)!, path: '', title: resource.title, artist: resource.artist, album: resource.album,
    album_artist: null, album_artist_or_artist: resource.artist, disc_number: null, cover_hash: resource.coverHash,
    track_number: null, duration_secs: resource.durationSeconds, file_hash: null, rarity: null,
    manually_edited: false, is_liked: false, play_count: 0, year: null, genre: null, tags: null,
    date_added: null, is_duplicate: false,
    plugin_source: { resource, sourceToken, generation },
  };
}
