export const SETTINGS_SECTIONS = [
  { id: 'library', label: 'Library', description: 'Music folder and library tools', icon: 'M4 6H2v14h16v-2H4zm2-4h16v14H6zm3 4v2h10V6zm0 4v2h7v-2z' },
  { id: 'playback', label: 'Playback', description: 'Audio output and sample rate', icon: 'M12 3a9 9 0 0 0-9 9v7h6v-8H5a7 7 0 0 1 14 0h-4v8h6v-7a9 9 0 0 0-9-9z' },
  { id: 'devices', label: 'Devices & Sync', description: 'Device identity and local network', icon: 'M1 9l2 2a13 13 0 0 1 18 0l2-2A16 16 0 0 0 1 9zm4 4 2 2a7 7 0 0 1 10 0l2-2a10 10 0 0 0-14 0zm4 4 3 3 3-3a4 4 0 0 0-6 0z' },
  { id: 'soulseek', label: 'Soulseek', description: 'Search connection and downloads', icon: 'M10 3a7 7 0 1 0 4.9 12l5.6 5.6 1.4-1.4-5.6-5.6A7 7 0 0 0 10 3zm0 2a5 5 0 1 1 0 10 5 5 0 0 1 0-10z' },
  { id: 'appearance', label: 'Appearance', description: 'Motion and now playing visuals', icon: 'M12 3a9 9 0 1 0 0 18h2a3 3 0 0 0 0-6h-1a1 1 0 0 1 0-2h3a5 5 0 0 0 5-5c0-3-4-5-9-5zM7 8h2v2H7zm4-2h2v2h-2zm5 1h2v2h-2z' },
  { id: 'about', label: 'About', description: 'Version, updates and release notes', icon: 'M11 10h2v7h-2zm0-4h2v2h-2zm1-4a10 10 0 1 0 0 20 10 10 0 0 0 0-20zm0 2a8 8 0 1 1 0 16 8 8 0 0 1 0-16z' },
] as const;

export type SettingsSection = typeof SETTINGS_SECTIONS[number]['id'];

export interface Contributor {
  login: string;
  profileUrl: string;
  avatarPath: string | null;
}

export interface DeviceSettings {
  emoji: string;
  device_name: string;
  sync_enabled: boolean;
  soulseek_enabled: boolean;
  soulseek_username: string;
  soulseek_password: string;
}

export interface AboutInfo {
  current_version: string;
  platform: string;
  build_commit: string | null;
  release_repo: string | null;
  changelog: Array<{ subject: string; short_hash: string; committed_at: string }>;
}

export interface AboutUpdateStatus {
  current_version: string;
  latest_version: string | null;
  has_update: boolean;
  release_url: string | null;
  release_notes: string | null;
  asset_url: string | null;
  checked_repo: string | null;
  message: string;
}
