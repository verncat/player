import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import ts from 'typescript';
import { computed, reactive, ref } from 'vue';
const plain = (v) => JSON.parse(JSON.stringify(v));
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
  const template = ({ id: 'example.hello', name: 'Hello', description: 'Example', repository: 'https://github.com/verncat/player-plugins', addedAt: '2026-10-31', platforms: ['macos'], tags: [], image: 'https://example.org/logo.png' });
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
  const entry = ({ id: 'example.hello', name: 'Hello', description: 'Example', repository: 'https://github.com/verncat/player-plugins', addedAt: '2026-10-31', platforms: ['macos'], tags: [], image: 'https://example.org/logo.png' });
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


function nativeHostHarness({startupEvent} = {}) {
  const source = readFileSync(new URL('../src/plugins/host.ts', import.meta.url), 'utf8');
  const code = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText;
  const helpers = {}, mounted = [], unmounted = [], listeners = new Map(), calls = [], navigation = [];
  const plugin = { manifest: { id: 'example.native', permissions: ['ui', 'playback.read', 'playback.control'], contributions: [{id:'home',kind:'page',label:'Native',order:0}] }, generation: 1, enabled: true, supported: true };
  const invoke = async (command, args) => {
    calls.push([command, plain(args ?? {})]);
    if (command === 'plugins_list') return [plain(plugin)];
    if (command === 'plugins_diagnostics') return [];
    if (command === 'plugin_load') { if (startupEvent) listeners.get('plugin-event')({payload:startupEvent}); return {events:startupEvent ? [startupEvent] : []}; }
    if (command === 'plugins_set_enabled') { plugin.enabled = args.enabled; plugin.generation++; return plain(plugin); }
    if (command === 'plugin_native_call') return args.method === 'render' ? { type:'text',text:'Native UI' } : null;
    if (command === 'plugin_native_reply') return null;
    throw new Error(`Unexpected command ${command}`);
  };
  const require = name => name === 'vue' ? { computed,ref,watch:()=>{},onMounted:fn=>mounted.push(fn),onUnmounted:fn=>unmounted.push(fn),provide:()=>{},inject:()=>null }
    : name.endsWith('/core') ? {invoke} : {listen:async (topic,handler)=>{listeners.set(topic,handler);return ()=>listeners.delete(topic);}};
  new Function('exports','require',code)(helpers,require);
  const host = helpers.createPluginHost({ snapshot:()=>reactive({title:'A track',playing:true,resource:null}),transport:async()=>{},playSource:async()=>{},adopt:async()=>{},navigate:(...args)=>navigation.push(args) });
  return {host,calls,listeners,navigation,plugin,start:async()=>{for(const fn of mounted) await fn();},stop:()=>unmounted.forEach(fn=>fn())};
}

test('native host activates through IPC, passes plain data and renders native UI', async () => {
  const h = nativeHostHarness(); await h.start();
  assert.deepEqual(h.host.running.value,['example.native']);
  assert.equal(h.host.contributions('page').value[0].key,'example.native/home');
  assert.deepEqual(await h.host.call('example.native','render',reactive({viewId:'home',context:{track:{id:42}}})),{type:'text',text:'Native UI'});
  const requests=h.calls.filter(([command])=>command==='plugin_native_call').map(([,args])=>args);
  assert.deepEqual(requests.map(args=>args.method),['activate','playback','render']);
  assert.deepEqual(requests[2].args,{viewId:'home',context:{track:{id:42}}});
  await h.host.setEnabled('example.native',false);
  await assert.rejects(h.host.call('example.native','render'),/not running/);
  assert.deepEqual(h.host.running.value,[]);h.stop();
});

test('native host callbacks enforce activation identity and registered pages', async () => {
  const h=nativeHostHarness();await h.start();
  const handler=h.listeners.get('plugin-host-call');
  await handler({payload:{requestId:10,pluginId:'example.native',generation:1,method:'host.navigate',args:{pageId:'home',context:{id:2}}}});
  assert.deepEqual(h.navigation,[['plugins/example.native/home',{id:2}]]);
  await handler({payload:{requestId:11,pluginId:'example.native',generation:1,method:'host.navigate',args:{pageId:'missing'}}});
  assert.match(h.calls.at(-1)[1].error,/not registered/);
  await h.host.setEnabled('example.native',false);
  await handler({payload:{requestId:12,pluginId:'example.native',generation:1,method:'host.navigate',args:{pageId:'home'}}});
  assert.match(h.calls.at(-1)[1].error,/activation ended/);
  assert.equal(h.navigation.length,1);h.stop();
});

test('native invalidations and events ignore stale generations', async () => {
  const h=nativeHostHarness();await h.start();const revision=h.host.revision.value;
  h.listeners.get('plugin-invalidated')({payload:{pluginId:'example.native',generation:0}});
  assert.equal(h.host.revision.value,revision);
  h.listeners.get('plugin-invalidated')({payload:{pluginId:'example.native',generation:1}});
  assert.equal(h.host.revision.value,revision+1);
  h.listeners.get('plugin-event')({payload:{pluginId:'example.native',generation:1,topic:'transfer',payload:{state:'completed'}}});await settle();
  assert.equal(h.calls.filter(([command,args])=>command==='plugin_native_call'&&args.method==='event').length,1);h.stop();
});


test('native startup replays buffered events once and rejects later duplicates', async () => {
  const event={pluginId:'example.native',generation:1,revision:9,topic:'transfer',payload:{state:'completed'}};
  const h=nativeHostHarness({startupEvent:event});await h.start();
  h.listeners.get('plugin-event')({payload:event});await settle();
  assert.equal(h.calls.filter(([command,args])=>command==='plugin_native_call'&&args.method==='event').length,1);h.stop();
});
