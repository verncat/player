import type { Track } from '../types';
export interface Contribution {
  id: string; kind: 'page' | 'settings' | 'search' | 'track-action' | 'player-widget';
  label: string; description: string; icon?: string; order: number; slot?: 'player.actions' | 'player.status';
}
export interface PluginManifest {
  id: string; name: string; version: string; apiVersion: number; description: string;
  image?: string | null;
  platforms: string[]; permissions: string[]; contributions: Contribution[];
  author?: string | { name: string; url?: string | null } | null;
  homepage?: string | null; repository?: string | null;
  androidNative?: { packageName: string; entryClass: string; certificateSha256: string; apkSha256: string } | null;
}
export interface InstallSource { url: string; repository?: string | null; revision?: string | null }
export interface PluginPreview { manifest: PluginManifest; fingerprint: string; source?: InstallSource }
export interface PluginDownload extends PluginPreview { content: string; source: InstallSource }
export interface AndroidPluginDiscovery { supported: boolean; plugins: Array<PluginPreview & { content: string }>; warnings: string[] }
export interface PluginInfo {
  manifest: PluginManifest; builtin: boolean; supported: boolean; enabled: boolean;
  generation: number; migrated: boolean; fingerprint: string;
  installSource?: InstallSource | null;
}
export interface ResourceIdentity {
  pluginId: string; resourceId: string; title: string; artist: string; album: string;
  durationSeconds: number | null; coverHash: string | null;
}
export interface PlaybackSnapshot {
  title: string; artist: string; album: string; position: number; duration: number;
  playing: boolean; resource?: ResourceIdentity | null;
}
export interface PluginSource { sourceToken: string; resource: ResourceIdentity }
export interface ResultAction { id: string; label: string; disabled?: boolean }
export interface SearchItem {
  id: string; title: string; subtitle?: string; coverHash?: string | null;
  status?: string; actions?: ResultAction[];
}
export interface UiNode {
  type: 'group' | 'text' | 'input' | 'button' | 'toggle' | 'results';
  title?: string; children?: UiNode[]; text?: string; tone?: string;
  key?: string; label?: string; value?: string | boolean; inputType?: 'text' | 'password';
  action?: string; payload?: Record<string, unknown>; disabled?: boolean; items?: SearchItem[];
}
export interface PluginEvent { pluginId: string; generation: number; revision: number; topic: string; payload: unknown }
export interface HostOptions {
  snapshot(): PlaybackSnapshot;
  transport(action: 'pause' | 'resume' | 'stop' | 'seek', position?: number): Promise<void>;
  playSource(plugin: PluginInfo, source: PluginSource): Promise<void>;
  adopt(pluginId: string, resourceId: string, track: Track): Promise<void>;
  navigate(route: string, context?: Record<string, unknown>): void;
}
export interface UiContribution extends Contribution { pluginId: string; key: string }
