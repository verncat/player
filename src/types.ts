
export interface AudioDevice { name: string; sample_rates: number[] }
export interface DeviceList { devices: AudioDevice[]; current: string | null; current_sample_rate: number | null }

export interface SoulseekStatus {
  enabled: boolean;
  configured: boolean;
  username: string | null;
  activeSession: boolean;
}

export interface SoulseekSearchResult {
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

export interface SoulseekDownloadEvent {
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
  local_preview_path?: string | null;
  preview_growing?: boolean;
  soulseek_preview?: boolean;
  soulseek_username?: string | null;
  soulseek_filename?: string | null;
  soulseek_size?: number | null;
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
