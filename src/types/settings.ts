import type { AppIconName } from '../icons';

export const SETTINGS_SECTIONS = [
  { id: 'library', label: 'Library', description: 'Music folder and library tools', icon: 'library' },
  { id: 'playback', label: 'Playback', description: 'Audio output and sample rate', icon: 'playback' },
  { id: 'devices', label: 'Devices & Sync', description: 'Device identity and local network', icon: 'network' },
  { id: 'plugins', label: 'Plugins', description: 'Install and manage extensions', icon: 'settings' },
  { id: 'appearance', label: 'Appearance', description: 'Motion, theme and colors', icon: 'appearance' },
  { id: 'about', label: 'About', description: 'Version, updates and release notes', icon: 'info' },
] as const satisfies readonly { id: string; label: string; description: string; icon: AppIconName }[];

export type SettingsSection = typeof SETTINGS_SECTIONS[number]['id'] | 'plugins/add' | `plugin:${string}`;

export interface SettingsSubsection { id: SettingsSection; label: string; description: string }
export const SETTINGS_SUBSECTIONS: Partial<Record<typeof SETTINGS_SECTIONS[number]['id'], readonly SettingsSubsection[]>> = {
  plugins: [{ id: 'plugins/add', label: 'Add plugins', description: 'Browse and install extensions' }],
};

export interface Contributor {
  login: string;
  profileUrl: string;
  avatarPath: string | null;
}

export interface DeviceSettings {
  emoji: string;
  device_name: string;
  sync_enabled: boolean;
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
