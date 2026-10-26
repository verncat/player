/* This file is the portable SDK injected into every plugin Worker. No Tauri/DOM APIs. */
let definition;
let instance;
let nextId = 0;
const pending = new Map();
const topics = new Map();
const playbackHandlers = new Set();
let playbackSnapshot;
let active = true;
const lifetime = new AbortController();
globalThis.registerPlugin = (factory) => {
  if (definition) throw new Error('registerPlugin must be called exactly once');
  if (typeof factory !== 'function') throw new Error('registerPlugin expects a factory');
  definition = factory;
};
function call(method, args = {}) {
  if (!active) return Promise.reject(new Error('Plugin activation has ended'));
  return new Promise((resolve, reject) => {
    const id = ++nextId;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`TIMEOUT: ${method}`)); }, 45000);
    pending.set(id, { resolve, reject, timer });
    postMessage({ type: 'call', id, method, args });
  });
}
globalThis.onmessage = async ({ data }) => {
  if (data.type === 'reply') {
    const p = pending.get(data.id); if (!p) return;
    pending.delete(data.id); clearTimeout(p.timer);
    if (data.error) p.reject(new Error(data.error)); else p.resolve(data.result);
    return;
  }
  if (data.type === 'event') {
    for (const listener of topics.get(data.topic) || []) {
      try { await listener(data.payload); } catch (e) { postMessage({ type: 'log', message: String(e.message || e) }); }
    }
    return;
  }
  if (data.type === 'playback') {
    playbackSnapshot = data.snapshot;
    for (const listener of playbackHandlers) { try { listener(playbackSnapshot); } catch { /* isolate subscriber */ } }
    return;
  }
  if (data.type !== 'request') return;
  try {
    let result;
    if (data.method === 'activate') {
      if (!definition || instance) throw new Error('Invalid plugin entrypoint or repeated activation');
      const ctx = {
        pluginId: data.args.pluginId, generation: data.args.generation, signal: lifetime.signal, call,
        android: { call: (method, args = {}) => call('android.call', { method, args }) },
        on(topic, listener) {
          if (!topics.has(topic)) topics.set(topic, new Set());
          topics.get(topic).add(listener); return () => topics.get(topic)?.delete(listener);
        },
        playback: {
          subscribe(listener) {
            playbackHandlers.add(listener); if (playbackSnapshot) listener(playbackSnapshot);
            return () => playbackHandlers.delete(listener);
          },
          play: (source) => call('host.play', source),
          pause: () => call('host.transport', { action: 'pause' }),
          resume: () => call('host.transport', { action: 'resume' }),
          stop: () => call('host.transport', { action: 'stop' }),
          seek: (position) => call('host.transport', { action: 'seek', position }),
          adopt: (args) => call('host.adopt', args),
        },
        ui: {
          invalidate: () => postMessage({ type: 'invalidate' }),
          navigate: (pageId, context) => call('host.navigate', { pageId, context }),
        },
        log: (message) => postMessage({ type: 'log', message: String(message).slice(0, 1000) }),
      };
      instance = definition(ctx);
      if (!instance || typeof instance !== 'object' || typeof instance.then === 'function') throw new Error('Plugin factory must return an object');
      await instance.activate?.(); result = null;
    } else if (data.method === 'deactivate') {
      lifetime.abort(); await instance?.deactivate?.(); active = false;
      for (const p of pending.values()) { clearTimeout(p.timer); p.reject(new Error('CANCELLED')); }
      pending.clear(); topics.clear(); playbackHandlers.clear(); result = null;
    } else if (data.method === 'renderSearch') result = await instance?.renderSearch?.(data.args) || null;
    else if (data.method === 'render') result = await instance?.render?.(data.args) || { type: 'group', children: [] };
    else if (data.method === 'search') {
      if (!instance?.search) throw new Error('Search provider is not implemented');
      result = await instance.search(data.args);
    } else if (data.method === 'action') {
      if (!instance?.action) throw new Error('Action handler is not implemented');
      result = await instance.action({ ...data.args, payload: data.args?.payload || {}, fields: data.args?.fields || {}, context: data.args?.context || {} });
    } else throw new Error('METHOD_NOT_FOUND');
    postMessage({ type: 'response', id: data.id, result: result === undefined ? null : result });
  } catch (e) { postMessage({ type: 'response', id: data.id, error: String(e.message || e).slice(0, 2000) }); }
};
