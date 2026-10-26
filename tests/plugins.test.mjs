import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
import ts from 'typescript';
import { reactive } from 'vue';
const prelude = readFileSync(new URL('../src/plugins/worker.js', import.meta.url), 'utf8');
const source = (name) => readFileSync(new URL(`../plugins/${name}/main.js`, import.meta.url), 'utf8');
const plain = (v) => JSON.parse(JSON.stringify(v));
function runtime(script, handler = async () => null) {
  const calls = []; const replies = new Map(); let sequence = 0;
  const sandbox = { AbortController, setTimeout, clearTimeout, Date, Map, Set };
  sandbox.postMessage = (message) => {
    if (message.type === 'call') {
      calls.push(plain(message));
      Promise.resolve().then(() => handler(message.method, plain(message.args))).then(
        result => sandbox.onmessage({ data: { type: 'reply', id: message.id, result } }),
        error => sandbox.onmessage({ data: { type: 'reply', id: message.id, error: String(error) } }),
      );
    } else if (message.type === 'response') {
      const pending = replies.get(message.id); replies.delete(message.id);
      if (message.error) pending.reject(new Error(message.error)); else pending.resolve(plain(message.result));
    }
  };
  vm.createContext(sandbox); vm.runInContext(prelude + '\n' + script, sandbox);
  return {
    calls,
    request(method, args = {}) { return new Promise((resolve, reject) => { const id = ++sequence; replies.set(id, { resolve, reject }); void sandbox.onmessage({ data: { type: 'request', id, method, args } }); }); },
    send(data) { return sandbox.onmessage({ data }); },
  };
}
const settle = () => new Promise(resolve => setImmediate(resolve));

test('Worker transport serializes nested Vue proxies in view contexts, actions and snapshots', () => {
  const helpers = {};
  const code = ts.transpileModule(readFileSync(new URL('../src/plugins/messages.ts', import.meta.url), 'utf8'), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText;
  new Function('exports', code)(helpers);
  const context = reactive({ track: { id: 42, title: 'Привет' }, ids: [1, 2] });
  assert.throws(() => structuredClone(context), /cloned/);
  let received;
  const worker = { postMessage(message) { received = structuredClone(message); } };
  helpers.postPluginMessage(worker, { type: 'request', args: { context, payload: context.track } });
  assert.deepEqual(received.args.context, { track: { id: 42, title: 'Привет' }, ids: [1, 2] });
  assert.equal(received.args.payload.title, 'Привет');
  helpers.postPluginMessage(worker, { type: 'playback', snapshot: reactive({ resource: context.track }) });
  assert.equal(received.snapshot.resource.id, 42);
});

test('Android APK UI calls its native backend through the worker gateway', async () => {
  let callCount = 0;
  const r = runtime(source('examples/android-native'), async (method, args) => {
    assert.equal(method, 'android.call');
    assert.equal(args.method, 'echo');
    return { message: args.args.message, backend: 'Rust via JNI', arch: 'aarch64', callCount: ++callCount };
  });
  await r.request('activate', { pluginId: 'example.android-native', generation: 1 });
  assert.match((await r.request('render')).children[0].text, /Loaded from an APK/);
  await r.request('action', { action: 'ping', fields: { message: 'Привет 🎵' } });
  const view = await r.request('render');
  assert.match(view.children[0].text, /Привет 🎵 · Rust via JNI · aarch64/);
  assert.equal(view.children[1].text, 'Rust calls: 2');
  await r.request('deactivate');
  await assert.rejects(r.request('action', { action: 'ping' }), /activation has ended/);
  assert.equal(callCount, 2);
});

test('installable example uses the same SDK for settings, rendering and playback events', async () => {
  let config = { revision: 0, values: {} };
  const r = runtime(source('examples/hello'), async (method, args) => {
    if (method === 'settings.get') return config;
    if (method === 'settings.update') { assert.equal(args.revision, config.revision); config = { revision: config.revision + 1, values: args.values }; return config; }
    throw new Error(method);
  });
  await r.request('activate', { pluginId: 'example.hello' });
  await r.send({ type: 'playback', snapshot: { title: 'Test song', playing: true } });
  await r.request('action', { action: 'save', fields: { greeting: 'My player' } });
  const view = await r.request('render', { viewId: 'home' });
  assert.equal(view.title, 'My player'); assert.equal(view.children[0].text, 'Now playing: Test song');
  await r.request('deactivate');
});

test('deactivation aborts plugin lifetime and refuses further capability calls', async () => {
  const r = runtime(`registerPlugin(ctx => ({ action: async () => { if (!ctx.signal.aborted) await ctx.call('settings.get'); return ctx.signal.aborted; } }));`);
  await r.request('activate', { pluginId: 'example.test' });
  assert.equal(await r.request('action'), false);
  await r.request('deactivate');
  assert.equal(await r.request('action'), true);
  assert.equal(r.calls.length, 1);
});

test('worker reports invalid entrypoints and action errors without killing message routing', async () => {
  const bad = runtime('registerPlugin(() => null);');
  await assert.rejects(bad.request('activate', { pluginId: 'example.bad' }), /factory/);
  const r = runtime(`registerPlugin(() => ({ action() { throw new Error('Unavailable'); }, render() { return { type: 'text', text: 'Still alive' }; } }));`);
  await r.request('activate', { pluginId: 'example.test' });
  await assert.rejects(r.request('action'), /Unavailable/);
  assert.equal((await r.request('render')).text, 'Still alive');
  await r.request('deactivate');
});

test('Discord plugin sends meaningful changes and clears activity on pause', async () => {
  const r = runtime(source('discord-rpc'));
  await r.request('activate', { pluginId: 'player.discord-rpc' });
  const snapshot = { title: 'Song', artist: 'Artist', album: 'Album', position: 10, duration: 200, playing: true };
  await r.send({ type: 'playback', snapshot }); await settle();
  await r.send({ type: 'playback', snapshot }); await settle();
  await r.send({ type: 'playback', snapshot: { ...snapshot, playing: false } }); await settle();
  const updates = r.calls.filter(c => c.method === 'discord.update');
  assert.equal(updates.length, 2); assert.equal(updates[0].args.playback.title, 'Song'); assert.equal(updates[1].args.playback, null);
  await r.request('deactivate');
});

test('Soulseek previews buffer, play a generic source, then import and adopt the library track', async () => {
  const result = { username: 'peer', filename: 'Album/song.flac', basename: 'song.flac', size: 2000000, bitrate: 320, duration: 100, extension: 'flac' };
  const track = { id: 42, title: 'Song', path: 'Artist/Album/song.flac' };
  let config = { revision: 0, values: { username: 'me' } };
  const r = runtime(source('soulseek'), async (method, args) => {
    if (method === 'settings.get') return config;
    if (method === 'settings.update') { assert.equal(args.revision, config.revision); config = { revision: config.revision + 1, values: args.values }; return config; }
    if (method === 'secrets.has') return true;
    if (method === 'soulseek.status') return { configured: true };
    if (method === 'soulseek.search') return [result];
    if (method === 'soulseek.preview') return 'transfer-1';
    if (method === 'soulseek.prepare') { assert.equal(args.resource.pluginId, 'player.soulseek'); return 'source-1'; }
    if (method === 'library.import') return track;
    if (method === 'host.play' || method === 'host.adopt') return null;
    throw new Error(`Unexpected call ${method}`);
  });
  await r.request('activate', { pluginId: 'player.soulseek', generation: 1 });
  const found = await r.request('search', { query: 'song' }); const resourceId = found.items[0].id;
  await r.request('action', { action: 'play', payload: { resourceId } });
  await r.send({ type: 'event', topic: 'soulseek-preview', payload: { ...result, transferId: 'transfer-1', state: 'progress', localPath: '/cache/song.flac', bytesDownloaded: 1000 } });
  assert.equal(r.calls.filter(c => c.method === 'host.play').length, 0);
  await r.send({ type: 'event', topic: 'soulseek-preview', payload: { ...result, transferId: 'transfer-1', state: 'progress', localPath: '/cache/song.flac', bytesDownloaded: 1000000 } });
  assert.equal(r.calls.filter(c => c.method === 'host.play').length, 1);
  await r.request('action', { action: 'save', payload: { resourceId } });
  assert.equal(r.calls.filter(c => c.method === 'library.import').length, 0);
  await r.send({ type: 'event', topic: 'soulseek-preview', payload: { ...result, transferId: 'transfer-1', state: 'completed', localPath: '/cache/song.flac', bytesDownloaded: result.size } });
  assert.equal(r.calls.filter(c => c.method === 'library.import').length, 1);
  assert.equal(r.calls.find(c => c.method === 'host.adopt').args.track.id, 42);
  await r.request('deactivate');
});


test('Soulseek save intention survives Worker reload without restarting its native transfer', async () => {
  const result = { username: 'peer', filename: 'song.flac', basename: 'song.flac', size: 2000000, duration: 100 };
  let config = { revision: 0, values: {} }; const nativeCalls = [];
  const handler = async (method, args) => {
    nativeCalls.push(method);
    if (method === 'settings.get') return config;
    if (method === 'settings.update') { config = { revision: config.revision + 1, values: args.values }; return config; }
    if (method === 'secrets.has') return true;
    if (method === 'soulseek.status') return { configured: true };
    if (method === 'soulseek.search') return [result];
    if (method === 'soulseek.preview') return 'transfer';
    if (method === 'soulseek.prepare') return 'token';
    if (method === 'library.import') return { id: 77, path: 'song.flac' };
    if (method === 'host.adopt') return null;
    throw new Error(method);
  };
  const first = runtime(source('soulseek'), handler);
  await first.request('activate', { pluginId: 'player.soulseek', generation: 7 });
  const found = await first.request('search', { query: 'song' });
  await first.request('action', { action: 'save', payload: { resourceId: found.items[0].id } });
  assert.equal(config.values.transferIntents[0].save, true);
  await first.request('deactivate');
  const reloaded = runtime(source('soulseek'), handler);
  await reloaded.request('activate', { pluginId: 'player.soulseek', generation: 7 });
  await reloaded.send({ type: 'event', topic: 'soulseek-preview', payload: { ...result, transferId: 'transfer', state: 'completed', localPath: '/cache/song.flac', bytesDownloaded: result.size } });
  assert.equal(nativeCalls.filter(c => c === 'soulseek.preview').length, 1);
  assert.equal(nativeCalls.filter(c => c === 'library.import').length, 1);
  assert.deepEqual(config.values.transferIntents, []);
  await reloaded.request('deactivate');
});


test('Soulseek search pages preserve all matches and a stable next cursor', async () => {
  const results = Array.from({ length: 250 }, (_, index) => ({ username: 'peer', filename: `${index}.mp3`, basename: `${index}.mp3`, size: 1000 }));
  let searches = 0;
  const r = runtime(source('soulseek'), async (method) => {
    if (method === 'settings.get') return { revision: 0, values: {} };
    if (method === 'secrets.has') return true;
    if (method === 'soulseek.status') return { configured: true };
    if (method === 'soulseek.search') { searches++; return results; }
    throw new Error(method);
  });
  await r.request('activate', { pluginId: 'player.soulseek', generation: 1 });
  const first = await r.request('search', { query: 'music', limit: 120 });
  const second = await r.request('search', { query: 'music', cursor: first.nextCursor, limit: 120 });
  const last = await r.request('search', { query: 'music', cursor: second.nextCursor, limit: 120 });
  assert.equal(first.items.length, 120); assert.equal(second.items.length, 120); assert.equal(last.items.length, 10);
  assert.equal(last.nextCursor, null); assert.equal(searches, 1);
  assert.equal(new Set([...first.items, ...second.items, ...last.items].map(i => i.id)).size, 250);
  await r.request('deactivate');
});
