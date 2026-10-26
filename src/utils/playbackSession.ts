// Only queue context is cached. The native engine owns the active track and transport state.
export interface SavedPlaybackContext {
  version: 1;
  libraryDir: string;
  sourceId: string;
  trackId: number;
  source: 'library' | 'playlist' | 'recent' | 'plugin';
  sourceIds: number[];
  queueIds: number[];
  sourceIndex: number;
  pagedSource: unknown;
  shuffled: boolean;
  repeat: number;
  remotePeer: { name: string; host: string; port: number; addresses: string[] } | null;
}

export const PLAYBACK_SESSION_KEY = 'player.playback-session.v1';

export function readPlaybackContext(raw: string | null): SavedPlaybackContext | null {
  if (!raw || raw.length > 250_000) return null;
  try {
    const value = JSON.parse(raw);
    const ids = (values: unknown): values is number[] => Array.isArray(values) && values.length <= 5000
      && values.every(id => Number.isSafeInteger(id) && id > 0);
    if (!value || value.version !== 1 || typeof value.libraryDir !== 'string'
      || typeof value.sourceId !== 'string' || !Number.isSafeInteger(value.trackId)
      || !['library', 'playlist', 'recent', 'plugin'].includes(value.source)
      || !ids(value.sourceIds) || !ids(value.queueIds) || !Number.isSafeInteger(value.sourceIndex)
      || value.sourceIndex < -1 || value.sourceIndex >= Math.max(1, value.sourceIds.length)
      || typeof value.shuffled !== 'boolean' || ![0, 1, 2].includes(value.repeat)) return null;
    if (value.remotePeer !== null && (typeof value.remotePeer?.name !== 'string' || typeof value.remotePeer?.host !== 'string'
      || !Number.isInteger(value.remotePeer.port) || value.remotePeer.port < 1 || value.remotePeer.port > 65535
      || !Array.isArray(value.remotePeer.addresses) || !value.remotePeer.addresses.every((address: unknown) => typeof address === 'string'))) return null;
    return value;
  } catch { return null; }
}

export function matchesPlaybackContext(context: SavedPlaybackContext | null, libraryDir: string, sourceId: string | null, trackId: number): context is SavedPlaybackContext {
  return !!context && context.libraryDir === libraryDir && context.sourceId === sourceId && context.trackId === trackId;
}
