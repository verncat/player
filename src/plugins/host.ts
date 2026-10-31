import { computed, inject, onMounted, onUnmounted, provide, ref, watch, type InjectionKey } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { HostOptions, InstallSource, PluginEvent, PluginInfo, PluginSource, UiContribution, UiNode } from './types';
import type { Track } from '../types';

interface NativeHostCall { requestId: number; pluginId: string; generation: number; method: string; args: Record<string, unknown> }
export function createPluginHost(options: HostOptions) {
  const plugins = ref<PluginInfo[]>([]);
  const errors = ref<Record<string, string>>({});
  const running = ref<string[]>([]);
  const revision = ref(0);
  const loading = ref(true);
  const operations = new Map<string, Promise<unknown>>();
  const unlisteners: UnlistenFn[] = [];
  const backlog = new Map<string, PluginEvent[]>();
  const seenEvents = new Map<string, Set<number>>();
  const eventTails = new Map<string, Promise<void>>();
  const contributionList = computed<UiContribution[]>(() => plugins.value
    .filter(p => p.enabled && p.supported && running.value.includes(p.manifest.id))
    .flatMap(p => p.manifest.contributions.map(c => ({ ...c, pluginId: p.manifest.id, key: `${p.manifest.id}/${c.id}` })))
    .sort((a, b) => a.order - b.order || a.key.localeCompare(b.key)));
  const contributions = (kind: UiContribution['kind']) => computed(() => contributionList.value.filter(c => c.kind === kind));
  let disposed = false;
  function invalidate() { revision.value++; }
  function permitted(info: PluginInfo, permission: string) {
    if (!info.enabled || !info.manifest.permissions.includes(permission)) throw new Error(`PERMISSION_DENIED: ${permission}`);
  }
  function nativeCall(info: PluginInfo, method: string, args: unknown = {}): Promise<unknown> {
    // IPC carries data only; no plugin executable code enters the webview.
    return invoke('plugin_native_call', { pluginId: info.manifest.id, generation: info.generation, method, args: JSON.parse(JSON.stringify(args)) });
  }
  function call(id: string, method: string, args: unknown = {}): Promise<unknown> {
    const info = plugins.value.find(p => p.manifest.id === id);
    if (!info?.enabled || !running.value.includes(id)) return Promise.reject(new Error('Native plugin is not running'));
    return nativeCall(info, method, args);
  }
  function terminate(id: string) { running.value = running.value.filter(p => p !== id); invalidate(); }
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

  function deliverEvent(info: PluginInfo, event: PluginEvent): Promise<void> {
    const id = info.manifest.id;
    const seen = seenEvents.get(id) || new Set<number>();
    if (seen.has(event.revision)) return Promise.resolve();
    seen.add(event.revision); if (seen.size > 2048) seen.delete(seen.values().next().value!); seenEvents.set(id, seen);
    const pending = (eventTails.get(id) || Promise.resolve()).catch(() => {}).then(async () => {
      if (disposed || !plugins.value.some(p => p.manifest.id === id && p.enabled && p.generation === event.generation)) return;
      await nativeCall(info, 'event', { topic: event.topic, payload: event.payload }); invalidate();
    });
    eventTails.set(id, pending); return pending;
  }
  async function start(info: PluginInfo) {
    const id = info.manifest.id;
    if (running.value.includes(id) || !info.enabled || !info.supported || disposed) return;
    try {
      const loaded = await invoke<{ events: PluginEvent[] }>('plugin_load', { pluginId: id });
      await nativeCall(info, 'activate', { pluginId: id, generation: info.generation });
      if (disposed || !plugins.value.some(p => p.manifest.id === id && p.enabled && p.generation === info.generation)) return;
      if (info.manifest.permissions.includes('playback.read')) await nativeCall(info, 'playback', options.snapshot());
      running.value = [...running.value, id];
      const events = [...loaded.events, ...(backlog.get(id) || [])].filter(e => e.generation === info.generation).sort((a,b) => a.revision - b.revision);
      backlog.delete(id); await Promise.all(events.map(event => deliverEvent(info, event)));
      errors.value = { ...errors.value, [id]: '' }; invalidate();
    } catch (e) {
      if (disposed || !plugins.value.some(p => p.manifest.id === id && p.enabled && p.generation === info.generation)) return;
      errors.value = { ...errors.value, [id]: String(e) }; terminate(id);
      try { const disabled = await invoke<PluginInfo>('plugins_set_enabled', { pluginId: id, enabled: false }); plugins.value = plugins.value.map(p => p.manifest.id === id ? disabled : p); } catch { /* Preserve the original startup error. */ }
    }
  }
  async function refresh() { plugins.value = await invoke<PluginInfo[]>('plugins_list'); }
  async function setEnabled(id: string, enabled: boolean) {
    if (operations.has(id)) return operations.get(id);
    const operation = (async () => {
      try {
        const info = await invoke<PluginInfo>('plugins_set_enabled', { pluginId: id, enabled });
        plugins.value = plugins.value.map(p => p.manifest.id === id ? info : p);
        if (enabled) { seenEvents.delete(id); await start(info); } else { terminate(id); backlog.delete(id); seenEvents.delete(id); }
      } catch (e) {
        await refresh(); if (!plugins.value.find(p => p.manifest.id === id)?.enabled) terminate(id);
        errors.value = { ...errors.value, [id]: String(e) }; throw e;
      }
    })();
    operations.set(id, operation); try { await operation; } finally { operations.delete(id); }
  }
  async function retry(id: string) { if (plugins.value.find(p => p.manifest.id === id)?.enabled) await setEnabled(id, false); await setEnabled(id, true); }
  async function install(content: string, fingerprint: string, source?: InstallSource) { await invoke('plugins_install', { content, fingerprint, source }); await refresh(); }
  async function uninstall(id: string) { terminate(id); await invoke('plugins_uninstall', { pluginId: id }); await refresh(); }
  // Coalesce playback snapshots while a native call is in flight.
  const latestPlayback = new Map<string, ReturnType<HostOptions['snapshot']>>();
  const sendingPlayback = new Set<string>();
  async function publishPlayback(info: PluginInfo, snapshot: ReturnType<HostOptions['snapshot']>) {
    const id = info.manifest.id; latestPlayback.set(id, snapshot);
    if (sendingPlayback.has(id)) return;
    sendingPlayback.add(id);
    try {
      while (!disposed && running.value.includes(id) && latestPlayback.has(id)) {
        const latest = latestPlayback.get(id)!; latestPlayback.delete(id);
        await nativeCall(info, 'playback', latest);
      }
    } catch (e) { errors.value = { ...errors.value, [id]: String(e) }; }
    finally { sendingPlayback.delete(id); latestPlayback.delete(id); }
  }
  watch(options.snapshot, snapshot => {
    for (const info of plugins.value) if (running.value.includes(info.manifest.id) && info.manifest.permissions.includes('playback.read')) void publishPlayback(info, snapshot);
  });
  onMounted(async () => {
    try {
      unlisteners.push(await listen<NativeHostCall>('plugin-host-call', async ({ payload }) => {
        const info = plugins.value.find(p => p.manifest.id === payload.pluginId && p.enabled && p.generation === payload.generation);
        let result: unknown = null; let error: string | null = null;
        try { if (!info) throw new Error('Plugin activation ended'); result = await route(info, payload.method, payload.args || {}); }
        catch (e) { error = String(e); }
        await invoke('plugin_native_reply', { requestId: payload.requestId, pluginId: payload.pluginId, generation: payload.generation, result: result ?? null, error });
      }));
      unlisteners.push(await listen<{pluginId: string; generation: number}>('plugin-invalidated', ({ payload }) => {
        if (plugins.value.some(p => p.manifest.id === payload.pluginId && p.enabled && p.generation === payload.generation)) invalidate();
      }));
      unlisteners.push(await listen<{pluginId:string;generation:number;error:string}>('plugin-runtime-error', ({payload}) => {
        if (plugins.value.some(p => p.manifest.id === payload.pluginId && p.enabled && p.generation === payload.generation)) errors.value = {...errors.value,[payload.pluginId]:payload.error};
      }));
      unlisteners.push(await listen<PluginEvent>('plugin-event', ({ payload }) => {
        const info = plugins.value.find(p => p.manifest.id === payload.pluginId && p.enabled && p.generation === payload.generation);
        if (!info) return;
        if (running.value.includes(info.manifest.id)) void deliverEvent(info, payload).catch(e => { errors.value = { ...errors.value, [info.manifest.id]: String(e) }; });
        else { const queued = backlog.get(info.manifest.id) || []; queued.push(payload); if (queued.length > 512) queued.shift(); backlog.set(info.manifest.id, queued); }
      }));
      await refresh();
      const warnings = await invoke<string[]>('plugins_diagnostics'); if (warnings.length) errors.value = { ...errors.value, host: warnings.join(' ') };
      await Promise.all(plugins.value.map(start));
    } catch (e) { errors.value = { ...errors.value, host: String(e) }; }
    finally { loading.value = false; }
  });
  onUnmounted(() => { disposed = true; unlisteners.forEach(stop => stop()); running.value = []; });
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
