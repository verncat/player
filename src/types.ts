
export interface AudioDevice { name: string; sample_rates: number[] }
export interface DeviceList { devices: AudioDevice[]; current: string | null; current_sample_rate: number | null }

export interface Track {
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
  album_artist: string | null;
  disc_number: number | null;
  /** Who the album is by: the album artist tag, or the track artist when the
   *  album has none (see library_tracks in library.rs). */
  album_artist_or_artist: string | null;
  /** Names the track's cover for coverUrl() in utils/covers.ts; null when it
   *  has none. */
  cover_hash: string | null;
  plugin_source?: {
    resource: import('./plugins/types').ResourceIdentity;
    sourceToken: string;
    generation: number;
  };
}

export interface AlbumSummary {
  album: string;
  album_artist_or_artist: string;
  year: number | null;
  track_count: number;
  /** The cover of the album's first track (in album order) that has one. */
  cover_hash: string | null;
}

export interface TrackPage {
  tracks: Track[];
  total: number;
  offset: number;
  has_more: boolean;
}

export const animatedRarities = new Set(['Epic', 'Legendary', 'Mythic']);
export const TRACK_RARITY_OPTIONS = ['Common', 'Uncommon', 'Rare', 'Epic', 'Legendary', 'Mythic'] as const;
export type TrackRarity = typeof TRACK_RARITY_OPTIONS[number];
