import { computed, inject, onMounted, onUnmounted, provide, ref, watch, type InjectionKey } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import prelude from './worker.js?raw';
import { postPluginMessage } from './messages';
import type { HostOptions, InstallSource, PluginEvent, PluginInfo, PluginSource, UiContribution, UiNode } from './types';
import type { Track } from '../types';

interface Runtime {
  url: string; worker: Worker; ready: boolean; nextId: number; seen: Set<number>; events: PluginEvent[];
  pending: Map<number, { resolve(value: unknown): void; reject(error: Error): void; timer: ReturnType<typeof setTimeout> }>;
}
export function createPluginHost(options: HostOptions) {
  const plugins = ref<PluginInfo[]>([]);
  const errors = ref<Record<string, string>>({});
  const running = ref<string[]>([]);
  const revision = ref(0);
  const loading = ref(true);
  const runtimes = new Map<string, Runtime>();
  const operations = new Map<string, Promise<unknown>>();
  const backlog = new Map<string, PluginEvent[]>();
  const contributionList = computed<UiContribution[]>(() => plugins.value
    .filter(p => p.enabled && p.supported && running.value.includes(p.manifest.id))
    .flatMap(p => p.manifest.contributions.map(c => ({ ...c, pluginId: p.manifest.id, key: `${p.manifest.id}/${c.id}` })))
    .sort((a, b) => a.order - b.order || a.key.localeCompare(b.key)));
  const contributions = (kind: UiContribution['kind']) => computed(() => contributionList.value.filter(c => c.kind === kind));
  let disposed = false;
  let unlisten: UnlistenFn | undefined;
  function invalidate() { revision.value++; }
  function permitted(info: PluginInfo, permission: string) {
    if (!info.enabled || !info.manifest.permissions.includes(permission)) throw new Error(`PERMISSION_DENIED: ${permission}`);
  }
  function call(id: string, method: string, args: unknown = {}, timeout = 45000): Promise<unknown> {
    const runtime = runtimes.get(id);
    if (!runtime) return Promise.reject(new Error('Plugin is not running'));
    const requestId = ++runtime.nextId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { runtime.pending.delete(requestId); reject(new Error(`TIMEOUT: ${method}`)); }, timeout);
      runtime.pending.set(requestId, { resolve, reject, timer });
      try { postPluginMessage(runtime.worker, { type: 'request', id: requestId, method, args }); }
      catch (error) { clearTimeout(timer); runtime.pending.delete(requestId); reject(error); }
    });
  }
  function terminate(id: string) {
    const runtime = runtimes.get(id); if (!runtime) return;
    runtimes.delete(id); runtime.worker.terminate(); URL.revokeObjectURL(runtime.url);
    for (const pending of runtime.pending.values()) { clearTimeout(pending.timer); pending.reject(new Error('CANCELLED: plugin stopped')); }
    running.value = running.value.filter(p => p !== id); invalidate();
  }
  function event(runtime: Runtime, payload: PluginEvent) {
    if (runtime.seen.has(payload.revision)) return;
    runtime.seen.add(payload.revision);
    if (runtime.seen.size > 2048) runtime.seen.delete(runtime.seen.values().next().value!);
    if (!runtime.ready) runtime.events.push(payload);
    else postPluginMessage(runtime.worker, { type: 'event', topic: payload.topic, payload: payload.payload });
  }
  async function route(info: PluginInfo, method: string, args: Record<string, unknown>) {
    const current = plugins.value.find(p => p.manifest.id === info.manifest.id);
    if (!current?.enabled || current.generation !== info.generation) throw new Error('DISABLED: plugin activation ended');
    if (method === 'host.play') { permitted(info, 'playback.control'); await options.playSource(info, args as unknown as PluginSource); return null; }
    if (method === 'host.transport') {
      permitted(info, 'playback.control');
      if (!['pause', 'resume', 'stop', 'seek'].includes(String(args.action))) throw new Error('Unknown transport action');
      if (args.action === 'seek' && (typeof args.position !== 'number' || !Number.isFinite(args.position) || args.position < 0)) throw new Error('Invalid seek position');
      await options.transport(args.action as 'pause' | 'resume' | 'stop' | 'seek', args.position as number | undefined); return null;
    }
    if (method === 'host.adopt') {
      if (!info.manifest.permissions.some(p => p === 'library.import' || p === 'library.replace')) throw new Error('PERMISSION_DENIED: library');
      await options.adopt(info.manifest.id, String(args.resourceId), args.track as Track); return null;
    }
    if (method === 'host.navigate') {
      permitted(info, 'ui');
      const page = info.manifest.contributions.find(c => c.kind === 'page' && c.id === args.pageId);
      if (!page) throw new Error('Page not registered');
      options.navigate(`plugins/${info.manifest.id}/${page.id}`, args.context as Record<string, unknown>); return null;
    }
    return invoke('plugin_call', { pluginId: info.manifest.id, generation: info.generation, method, args });
  }
  async function start(info: PluginInfo) {
    const id = info.manifest.id; if (runtimes.has(id) || !info.enabled || !info.supported) return;
    try {
      const loaded = await invoke<{ script: string; events: PluginEvent[] }>('plugin_load', { pluginId: id });
      if (disposed || !plugins.value.some(p => p.manifest.id === id && p.enabled && p.generation === info.generation)) return;
      const url = URL.createObjectURL(new Blob([prelude, '\n', loaded.script], { type: 'text/javascript' }));
      let worker: Worker;
      try { worker = new Worker(url, { name: id }); } catch (error) { URL.revokeObjectURL(url); throw error; }
      const runtime: Runtime = { url, worker, ready: false, pending: new Map(), nextId: 0, seen: new Set(), events: backlog.get(id) || [] };
      backlog.delete(id); runtimes.set(id, runtime);
      worker.onmessage = async ({ data }) => {
        if (runtimes.get(id) !== runtime || !data || typeof data !== 'object') return;
        if (data.type === 'response') {
          const pending = runtime.pending.get(data.id); if (!pending) return;
          runtime.pending.delete(data.id); clearTimeout(pending.timer);
          if (data.error) pending.reject(new Error(String(data.error))); else pending.resolve(data.result);
        } else if (data.type === 'call') {
          try { const result = await route(info, String(data.method), data.args || {}); if (runtimes.get(id) === runtime) postPluginMessage(worker, { type: 'reply', id: data.id, result }); }
          catch (e) { if (runtimes.get(id) === runtime) postPluginMessage(worker, { type: 'reply', id: data.id, error: String(e) }); }
        } else if (data.type === 'invalidate') invalidate();
        // Worker logs are deliberately not forwarded: third-party code may log credentials.
      };
      worker.onerror = (e) => { errors.value = { ...errors.value, [id]: e.message || 'Plugin worker failed' }; terminate(id); };
      worker.onmessageerror = () => { errors.value = { ...errors.value, [id]: 'Invalid worker message' }; terminate(id); };
      if (info.manifest.permissions.includes('playback.read')) postPluginMessage(worker, { type: 'playback', snapshot: options.snapshot() });
      await call(id, 'activate', { pluginId: id, generation: info.generation }, 15000);
      URL.revokeObjectURL(url);
      if (runtimes.get(id) !== runtime) return;
      runtime.ready = true; running.value = [...running.value, id];
      const events = [...loaded.events, ...runtime.events].sort((a, b) => a.revision - b.revision);
      runtime.events = []; runtime.seen.clear();
      for (const e of events) if (e.generation === info.generation) event(runtime, e);
      errors.value = { ...errors.value, [id]: '' }; invalidate();
    } catch (e) {
      errors.value = { ...errors.value, [id]: String(e) }; terminate(id);
      if (!disposed && plugins.value.some(p => p.manifest.id === id && p.enabled && p.generation === info.generation)) {
        try { const disabled = await invoke<PluginInfo>('plugins_set_enabled', { pluginId: id, enabled: false }); plugins.value = plugins.value.map(p => p.manifest.id === id ? disabled : p); } catch { /* The error remains visible. */ }
      }
    }
  }
  async function refresh() { plugins.value = await invoke<PluginInfo[]>('plugins_list'); }
  async function setEnabled(id: string, enabled: boolean) {
    if (operations.has(id)) return operations.get(id);
    const operation = (async () => {
      try {
        const info = await invoke<PluginInfo>('plugins_set_enabled', { pluginId: id, enabled });
        plugins.value = plugins.value.map(p => p.manifest.id === id ? info : p);
        if (!enabled) {
          if (runtimes.has(id)) { try { await call(id, 'deactivate', {}, 2000); } catch { /* termination is the fallback */ } }
          terminate(id);
        } else await start(info);
      } catch (e) {
        // Native close can fail after revocation has already been persisted.
        await refresh();
        if (!plugins.value.find(p => p.manifest.id === id)?.enabled) terminate(id);
        errors.value = { ...errors.value, [id]: String(e) }; throw e;
      }
    })();
    operations.set(id, operation); try { await operation; } finally { operations.delete(id); }
  }
  async function retry(id: string) { terminate(id); const info = plugins.value.find(p => p.manifest.id === id); if (!info) return; if (!info.enabled) await setEnabled(id, true); else await start(info); }
  async function install(content: string, fingerprint: string, source?: InstallSource) { await invoke('plugins_install', { content, fingerprint, source }); await refresh(); }
  async function uninstall(id: string) { terminate(id); await invoke('plugins_uninstall', { pluginId: id }); await refresh(); }
  watch(options.snapshot, snapshot => {
    for (const [id, runtime] of runtimes) if (plugins.value.find(p => p.manifest.id === id)?.manifest.permissions.includes('playback.read')) postPluginMessage(runtime.worker, { type: 'playback', snapshot });
  });
  onMounted(async () => {
    try {
      unlisten = await listen<PluginEvent>('plugin-event', ({ payload }) => {
        const info = plugins.value.find(p => p.manifest.id === payload.pluginId);
        const runtime = runtimes.get(payload.pluginId);
        if (runtime && info?.enabled && info.generation === payload.generation) event(runtime, payload);
        else if (!runtime) { const events = backlog.get(payload.pluginId) || []; events.push(payload); if (events.length > 512) events.shift(); backlog.set(payload.pluginId, events); }
      });
      await refresh();
      const warnings = await invoke<string[]>('plugins_diagnostics'); if (warnings.length) errors.value = { ...errors.value, host: warnings.join(' ') };
      let legacy: string | null = null; try { legacy = localStorage.getItem('player.discord-presence'); } catch { /* Storage access must not prevent plugin startup. */ }
      const discord = plugins.value.find(p => p.manifest.id === 'player.discord-rpc');
      if (legacy !== null && discord && !discord.migrated) {
        await invoke('plugins_migrate_enabled', { pluginId: discord.manifest.id, enabled: legacy === 'true' });
        try { localStorage.removeItem('player.discord-presence'); } catch { /* Native migration marker is authoritative. */ } await refresh();
      }
      await Promise.all(plugins.value.map(start));
    } catch (e) { errors.value = { ...errors.value, host: String(e) }; }
    finally { loading.value = false; }
  });
  onUnmounted(() => { disposed = true; unlisten?.(); for (const id of runtimes.keys()) terminate(id); });
  return { plugins, errors, running, revision, loading, contributions, call, refresh, setEnabled, retry, install, uninstall };
}
export type PluginHost = ReturnType<typeof createPluginHost>;
const key: InjectionKey<PluginHost> = Symbol('PlayerPluginHost');
export function providePluginHost(host: PluginHost) { provide(key, host); }
export function usePluginHost(): PluginHost { const host = inject(key); if (!host) throw new Error('Plugin host not provided'); return host; }
export function validateUiNode(node: unknown, depth = 0): UiNode {
  if (!node || typeof node !== 'object' || depth > 12) throw new Error('Invalid plugin view');
  const n = node as UiNode;
  if (!['group', 'text', 'input', 'button', 'toggle', 'results'].includes(n.type)) throw new Error('Unknown plugin UI node');
  for (const key of ['title', 'text', 'label', 'action', 'key'] as const) if (n[key] !== undefined && (typeof n[key] !== 'string' || n[key]!.length > 4000)) throw new Error('Invalid UI text');
  if (n.type === 'input' && (!n.key || !/^[a-z0-9-]{1,64}$/.test(n.key))) throw new Error('Invalid field key');
  if (n.value !== undefined && typeof n.value !== 'string' && typeof n.value !== 'boolean') throw new Error('Invalid field value');
  if (n.children) { if (!Array.isArray(n.children) || n.children.length > 100) throw new Error('Too many view children'); n.children.forEach(c => validateUiNode(c, depth + 1)); }
  if (n.items) {
    if (!Array.isArray(n.items) || n.items.length > 1000) throw new Error('Too many results');
    for (const item of n.items) {
      if (!item || typeof item.id !== 'string' || item.id.length > 8000 || typeof item.title !== 'string' || item.title.length > 4000 || (item.subtitle !== undefined && (typeof item.subtitle !== 'string' || item.subtitle.length > 8000))) throw new Error('Invalid search item');
      if (item.actions && (!Array.isArray(item.actions) || item.actions.length > 10 || item.actions.some(a => typeof a.id !== 'string' || typeof a.label !== 'string' || a.label.length > 200))) throw new Error('Invalid result action');
    }
  }
  return n;
}
