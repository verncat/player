
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
  local_preview_path?: string | null;
  preview_growing?: boolean;
  soulseek_preview?: boolean;
  soulseek_username?: string | null;
  soulseek_filename?: string | null;
  soulseek_size?: number | null;
}

export const rarityColors: Record<string, string> = {
  Common: '#b0b0b0',
  Uncommon: '#1db954',
  Rare: '#4fc3f7',
  Epic: '#ba68c8',
  Legendary: '#ffa726',
  Mythic: '#ff5252',
};
export const animatedRarities = new Set(['Epic', 'Legendary', 'Mythic']);
export const TRACK_RARITY_OPTIONS = ['Common', 'Uncommon', 'Rare', 'Epic', 'Legendary', 'Mythic'] as const;
