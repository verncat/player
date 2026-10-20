import type { AppIconName } from '../icons';

export const SETTINGS_SECTIONS = [
  { id: 'library', label: 'Library', description: 'Music folder and library tools', icon: 'library' },
  { id: 'playback', label: 'Playback', description: 'Audio output and sample rate', icon: 'playback' },
  { id: 'devices', label: 'Devices & Sync', description: 'Device identity and local network', icon: 'network' },
  { id: 'soulseek', label: 'Soulseek', description: 'Search connection and downloads', icon: 'search' },
  { id: 'discord', label: 'Discord RPC', description: 'Share your listening activity', icon: 'presence' },
  { id: 'appearance', label: 'Appearance', description: 'Motion and now playing visuals', icon: 'appearance' },
  { id: 'about', label: 'About', description: 'Version, updates and release notes', icon: 'info' },
] as const satisfies readonly { id: string; label: string; description: string; icon: AppIconName }[];

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
