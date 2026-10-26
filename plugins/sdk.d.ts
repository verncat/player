/** Player Plugin API 1. Standalone declarations for authors of packaged scripts. */
interface PlayerPluginResource {
  pluginId: string; resourceId: string; title: string; artist: string; album: string;
  durationSeconds: number | null; coverHash: string | null;
}
interface PlayerPluginPlayback {
  title: string; artist: string; album: string; position: number; duration: number;
  playing: boolean; resource?: PlayerPluginResource | null;
}
interface PlayerPluginSearchItem {
  id: string; title: string; subtitle?: string; coverHash?: string | null; status?: string;
  actions?: Array<{ id: string; label: string; disabled?: boolean }>;
}
interface PlayerPluginNode {
  type: 'group' | 'text' | 'input' | 'button' | 'toggle' | 'results';
  title?: string; text?: string; tone?: string; children?: PlayerPluginNode[];
  key?: string; label?: string; value?: string | boolean; inputType?: 'text' | 'password';
  action?: string; payload?: Record<string, unknown>; disabled?: boolean;
  items?: PlayerPluginSearchItem[];
}
interface PlayerPluginContext {
  readonly pluginId: string;
  readonly generation: number;
  readonly signal: AbortSignal;
  call<T = unknown>(method: string, args?: Record<string, unknown>): Promise<T>;
  /** Android APK extension only; requires native.android. Unrestricted native code. */
  android: { call<T = unknown>(method: string, args?: Record<string, unknown>): Promise<T> };
  on(topic: string, listener: (payload: unknown) => void | Promise<void>): () => void;
  playback: {
    subscribe(listener: (snapshot: PlayerPluginPlayback) => void): () => void;
    pause(): Promise<void>; resume(): Promise<void>; stop(): Promise<void>; seek(position: number): Promise<void>;
    play(source: { sourceToken: string; resource: PlayerPluginResource }): Promise<void>;
    adopt(args: { resourceId: string; track: { id: number; path: string; [key: string]: unknown } }): Promise<void>;
  };
  ui: { invalidate(): void; navigate(pageId: string, context?: Record<string, unknown>): Promise<void> };
  log(message: string): void;
}
interface PlayerPluginDefinition {
  activate?(): void | Promise<void>;
  deactivate?(): void | Promise<void>;
  render?(args: { viewId: string; context: Record<string, unknown> }): PlayerPluginNode | Promise<PlayerPluginNode>;
  action?(args: { action: string; payload: Record<string, unknown>; fields: Record<string, string | boolean>; context?: Record<string, unknown> }): unknown | Promise<unknown>;
  search?(args: { providerId: string; query: string; cursor?: string | null; limit?: number }): Promise<{ items: PlayerPluginSearchItem[]; nextCursor: string | null }>;
  renderSearch?(args: { ids: string[] }): PlayerPluginSearchItem[] | null | Promise<PlayerPluginSearchItem[] | null>;
}
declare function registerPlugin(factory: (context: PlayerPluginContext) => PlayerPluginDefinition): void;
