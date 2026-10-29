import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
import ts from 'typescript';
import { computed, reactive, ref } from 'vue';
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

test('internal plugin pages stay routable while only visible pages reach the sidebar', () => {
  const script = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8').split('<script setup lang="ts">')[1].split('</script>')[0];
  const parsed = ts.createSourceFile('App.ts', script, ts.ScriptTarget.Latest, true);
  const declarations = parsed.statements.filter(s => ts.isVariableStatement(s) && s.declarationList.declarations.some(d => ['pluginMenuPages', 'activePluginPage'].includes(d.name.getText(parsed))));
  assert.equal(declarations.length, 2);
  const code = ts.transpileModule(declarations.map(d => d.getText(parsed)).join('\n'), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText;
  const pluginPages = ref([
    { id: 'home', pluginId: 'example.hello' },
    { id: 'public', pluginId: 'example.other', showInMenu: true },
    { id: 'replacement', pluginId: 'player.soulseek', showInMenu: false },
  ]);
  const activeNav = ref('plugins/player.soulseek/replacement');
  const { pluginMenuPages, activePluginPage } = new Function('computed', 'pluginPages', 'activeNav', `${code}\nreturn { pluginMenuPages, activePluginPage };`)(computed, pluginPages, activeNav);
  assert.deepEqual(pluginMenuPages.value.map(p => p.id), ['home', 'public']);
  assert.equal(activePluginPage.value.id, 'replacement');
  activeNav.value = 'plugins/example.hello/home';
  assert.equal(activePluginPage.value.id, 'home');
  activeNav.value = 'plugins/player.soulseek/replacement';
  assert.equal(activePluginPage.value.id, 'replacement');
  pluginPages.value = [];
  assert.equal(activePluginPage.value, undefined);
  assert.deepEqual(pluginMenuPages.value, []);
});

test('Soulseek opens its internal replacement page from a track action and keeps its search provider', async () => {
  const manifest = JSON.parse(readFileSync(new URL('../plugins/soulseek/plugin.json', import.meta.url), 'utf8'));
  assert.equal(manifest.contributions.find(c => c.id === 'replacement').showInMenu, false);
  assert.ok(manifest.contributions.some(c => c.kind === 'search'));
  const track = { id: 42, artist: 'Artist', title: 'Song' };
  const r = runtime(source('soulseek'), async (method) => {
    if (method === 'settings.get') return { revision: 0, values: {} };
    if (method === 'secrets.has') return false;
    if (method === 'soulseek.status') return { configured: false };
    if (method === 'host.navigate') return null;
    throw new Error(`Unexpected call ${method}`);
  });
  await r.request('activate', { pluginId: manifest.id });
  await r.request('action', { action: 'find-replacement', payload: { track } });
  assert.deepEqual(r.calls.find(c => c.method === 'host.navigate').args, { pageId: 'replacement', context: { track } });
  const view = await r.request('render', { viewId: 'replacement', context: { track } });
  assert.equal(view.title, 'Replace Song');
  assert.equal(view.children.find(n => n.key === 'query').value, 'Artist Song');
  await r.request('deactivate');
});

for (const component of ['PluginPanel', 'PluginSearch']) {
  test(`${component} keeps the clicked button busy, blocks duplicate actions and resets after errors`, async () => {
    const script = readFileSync(new URL(`../src/plugins/components/${component}.vue`, import.meta.url), 'utf8').split('<script setup lang="ts">')[1].split('</script>')[0];
    const parsed = ts.createSourceFile(`${component}.ts`, script, ts.ScriptTarget.Latest, true);
    const fn = parsed.statements.find(s => ts.isFunctionDeclaration(s) && s.name?.text === 'action');
    const code = ts.transpileModule(`export ${fn.getText(parsed)}`, {
      compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
    }).outputText;
    const helpers = {}, busy = { value: false }, pendingButton = { value: undefined }, error = { value: '' };
    let calls = 0, finish, fail;
    const host = { call: () => { calls++; return new Promise((resolve, reject) => { finish = resolve; fail = reject; }); } };
    new Function('exports', 'host', 'props', 'busy', 'actionBusy', 'loading', 'pendingButton', 'error', 'fields', 'render', code)(
      helpers, host, { pluginId: 'example.test', contribution: { pluginId: 'example.test' } }, busy, busy, { value: false }, pendingButton, error, { value: {} }, async () => {},
    );
    const first = helpers.action('save', {}, '/0/button');
    assert.equal(busy.value, true); assert.equal(pendingButton.value, '/0/button');
    await helpers.action('save', {}, '/1/button');
    assert.equal(calls, 1); assert.equal(pendingButton.value, '/0/button');
    finish(); await first;
    assert.equal(busy.value, false); assert.equal(pendingButton.value, undefined);
    const second = helpers.action('save', {}, '/1/button');
    fail(new Error('Request failed')); await second;
    assert.equal(busy.value, false); assert.equal(pendingButton.value, undefined);
    assert.match(error.value, /Request failed/);
  });
}

const catalogHelpers = {};
new Function('exports', ts.transpileModule(readFileSync(new URL('../src/plugins/catalog.ts', import.meta.url), 'utf8'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText)(catalogHelpers);

test('catalog requests the remote list once, clears old entries and only toasts on failure', async () => {
  const script = readFileSync(new URL('../src/plugins/components/PluginCatalog.vue', import.meta.url), 'utf8').split('<script setup lang="ts">')[1].split('</script>')[0];
  const parsed = ts.createSourceFile('PluginCatalog.ts', script, ts.ScriptTarget.Latest, true);
  const fn = parsed.statements.find(s => ts.isFunctionDeclaration(s) && s.name?.text === 'load');
  const code = ts.transpileModule(`export ${fn.getText(parsed)}`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText;
  const helpers = {}, loading = { value: false }, listing = { value: { catalog: { plugins: ['old'] } } };
  const calls = [], notices = [];
  let finish, fail;
  new Function('exports', 'invoke', 'loading', 'listing', 'emit', 'disposed', code)(helpers,
    (...args) => { calls.push(args); return new Promise((resolve, reject) => { finish = resolve; fail = reject; }); },
    loading, listing, (...args) => notices.push(args), false);
  const first = helpers.load();
  assert.equal(listing.value, undefined); assert.equal(loading.value, true);
  await helpers.load();
  assert.deepEqual(calls, [['plugins_catalog']]);
  const fresh = { catalog: { plugins: ['remote'] }, platform: 'linux' };
  finish(fresh); await first;
  assert.equal(listing.value, fresh); assert.equal(loading.value, false);
  const second = helpers.load();
  assert.equal(listing.value, undefined);
  fail(new Error('Plugin catalog returned HTTP 404')); await second;
  assert.equal(listing.value, undefined); assert.equal(loading.value, false);
  assert.deepEqual(notices, [['notify', 'Error: Plugin catalog returned HTTP 404']]);
  const retry = helpers.load();
  const empty = { catalog: { plugins: [] }, platform: 'linux' };
  finish(empty); await retry;
  assert.equal(listing.value, empty); assert.equal(calls.length, 3);
});

test('catalog searches author and tags, sorts newest first and leaves source order intact', () => {
  const template = JSON.parse(readFileSync(new URL('../plugins/catalog.json', import.meta.url), 'utf8')).plugins[0];
  const entries = [
    { ...template, id: 'example.old', name: 'Old plugin', addedAt: '2026-01-01', author: { name: 'Alice' }, tags: ['radio'] },
    { ...template, id: 'example.new', name: 'New plugin', addedAt: '2026-10-27', author: 'Alice', tags: ['radio'] },
    { ...template, id: 'example.other', name: 'Another plugin', addedAt: '2026-10-27', author: 'Bob', tags: ['lyrics'] },
  ];
  assert.deepEqual(catalogHelpers.filterCatalog(entries, '  ALICE radio ').map(p => p.id), ['example.new', 'example.old']);
  assert.deepEqual(catalogHelpers.filterCatalog(entries, '').map(p => p.id), ['example.other', 'example.new', 'example.old']);
  assert.deepEqual(catalogHelpers.filterCatalog(entries, 'missing'), []);
  assert.equal(entries[0].id, 'example.old');
});

test('catalog display metadata does not grant permissions or define executable UI contributions', () => {
  const entry = JSON.parse(readFileSync(new URL('../plugins/catalog.json', import.meta.url), 'utf8')).plugins[0];
  const displayed = catalogHelpers.catalogManifest({ ...entry, permissions: ['native.android'], contributions: [{ id: 'injected' }], script: 'malicious' });
  assert.deepEqual(displayed.permissions, []); assert.deepEqual(displayed.contributions, []);
  assert.equal(displayed.script, undefined);
  assert.equal(displayed.image, entry.image);
  assert.equal(displayed.repository, entry.repository);
});

test('catalog selection checks the requested ID and previews the actual package before installation', async () => {
  const script = readFileSync(new URL('../src/plugins/components/PluginManager.vue', import.meta.url), 'utf8').split('<script setup lang="ts">')[1].split('</script>')[0];
  const parsed = ts.createSourceFile('PluginManager.ts', script, ts.ScriptTarget.Latest, true);
  const fn = parsed.statements.find(s => ts.isFunctionDeclaration(s) && s.name?.text === 'loadCatalogPlugin');
  const code = ts.transpileModule(`export ${fn.getText(parsed)}`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText;
  const helpers = {}, busy = { value: '' }, error = { value: '' }, calls = [];
  const downloaded = { content: 'actual package', manifest: { id: 'example.hello', name: 'Actual name', permissions: ['network.http'] }, fingerprint: 'actual fingerprint' };
  let preview, fail = false;
  new Function('exports', 'invoke', 'busy', 'error', 'resetPreview', 'showPreview', code)(helpers,
    async (command, args) => { calls.push([command, args]); if (fail) throw new Error('ID mismatch'); return downloaded; },
    busy, error, () => { preview = undefined; }, (content, checked) => { preview = { content, checked }; });
  await helpers.loadCatalogPlugin({ id: 'example.hello', repository: 'https://github.com/author/plugin', installUrl: 'https://example.org/plugin.player-plugin', name: 'Catalog name' });
  assert.deepEqual(calls, [['plugins_download', { url: 'https://example.org/plugin.player-plugin', expectedId: 'example.hello' }]]);
  assert.equal(preview.checked.manifest.name, 'Actual name');
  assert.deepEqual(preview.checked.manifest.permissions, ['network.http']);
  assert.equal(busy.value, '');
  fail = true;
  await helpers.loadCatalogPlugin({ id: 'example.hello', repository: 'https://github.com/author/plugin' });
  assert.equal(calls[1][1].url, 'https://github.com/author/plugin');
  assert.equal(preview, undefined); assert.match(error.value, /ID mismatch/); assert.equal(busy.value, '');
});

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
