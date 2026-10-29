import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
import ts from 'typescript';

const source = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8').split('<script setup lang="ts">')[1].split('</script>')[0];
const parsed = ts.createSourceFile('App.ts', source, ts.ScriptTarget.Latest, true);
const functions = parsed.statements.filter(node => ts.isFunctionDeclaration(node)
  && ['applyIdentifiedTrack', 'handleIdentifyProgress'].includes(node.name?.text)).map(node => node.getText(parsed)).join('\n');
const compiled = ts.transpileModule(functions, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
const progressHelpers = {};
new Function('exports', ts.transpileModule(readFileSync(new URL('../src/utils/progressSegments.ts', import.meta.url), 'utf8'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText)(progressHelpers);
const { progressSegments } = progressHelpers;
const plain = value => JSON.parse(JSON.stringify(value));
const listNames = ['libraryTracks', 'recentTracks', 'localSearchTracks', 'queue', 'queueLibraryTracks', 'queuePlaylistTracks', 'queueSoulseekTracks', 'smartViewTracks'];
const saved = { id: 1, title: 'Круги на воде', artist: 'Слот', album: 'Septima', manually_edited: true, album_artist_or_artist: 'Слот', cover_hash: 'recognized-cover' };

function harness() {
  const old = { id: 1, title: 'Old title', artist: null, album: null };
  const other = { id: 2, title: 'Unrelated track', artist: null, album: null };
  const state = { identifyCurrent: { value: 0 }, identifyTotal: { value: 1 }, identifyDone: { value: false }, identifyPhase: { value: 'Preparing…' }, identifyStage: { value: 'preparing' },
    identifyResults: { value: [] }, identifyLogRef: { value: null }, nowPlaying: { value: old }, editingTrack: { value: old },
    editForm: { value: { title: old.title, artist: '', album: '' } }, playlistView: { value: { id: 10, tracks: [old, other] } },
    homePinnedRegularTracks: { value: { 10: [old, other] } }, historyEntries: { value: [{ played_at: 100, track: old }] },
    trackContextMenu: { value: { track: old, x: 10, y: 20 } }, notifications: 0,
    nextTick: callback => { callback(); return Promise.resolve(); },
  };
  for (const name of listNames) state[name] = { value: [old, other] };
  state.syncAndroid = () => { state.notifications++; };
  vm.createContext(state); vm.runInContext(compiled, state);
  const send = (status, extra = {}) => state.handleIdentifyProgress({ current: 0, total: 1, track_id: 1, track_name: old.title, status, message: status, ...extra });
  return { state, send, old, other };
}

test('identification stages update the right-hand status without creating result rows or completing the track', () => {
  const { state, send } = harness();
  for (const [status, label] of [['fingerprinting', 'Analyzing audio…'], ['looking_up', 'Searching AcoustID…'], ['fetching_cover', 'Fetching artwork…'], ['saving', 'Applying metadata…']]) {
    send(status);
    assert.equal(state.identifyPhase.value, label);
    assert.equal(state.identifyStage.value, status);
    assert.equal(state.identifyCurrent.value, 0);
    assert.equal(state.identifyResults.value.length, 0);
    assert.equal(state.nowPlaying.value.title, 'Old title');
  }
  send('found', { current: 1, track: saved, message: 'Слот – Круги на воде' });
  assert.equal(state.identifyCurrent.value, 1);
  assert.equal(state.identifyPhase.value, 'Metadata applied');
  assert.equal(state.identifyResults.value.length, 1);
  send('done', { current: 1, track_id: 0 });
  assert.equal(state.identifyDone.value, true);
  assert.equal(state.identifyPhase.value, 'Completed');
  assert.equal(state.identifyResults.value.length, 1);
});

test('a saved identification replaces stale metadata in now playing, all loaded lists, queues and an untouched editor', () => {
  const { state, send, old, other } = harness();
  send('found', { current: 1, track: saved });
  assert.deepEqual(plain(state.nowPlaying.value), saved);
  for (const name of listNames) {
    assert.deepEqual(plain(state[name].value[0]), saved, name);
    assert.equal(state[name].value[1], other, name);
  }
  assert.deepEqual(plain(state.playlistView.value.tracks[0]), saved);
  assert.deepEqual(plain(state.homePinnedRegularTracks.value[10][0]), saved);
  assert.deepEqual(plain(state.historyEntries.value[0]), { played_at: 100, track: saved });
  assert.deepEqual(plain(state.trackContextMenu.value.track), saved);
  assert.deepEqual(plain(state.editingTrack.value), saved);
  assert.deepEqual(plain(state.editForm.value), { title: saved.title, artist: saved.artist, album: saved.album });
  assert.equal(state.notifications, 1);
  assert.equal(old.title, 'Old title');
});

test('applying metadata preserves a user-edited draft while refreshing other editor fields', () => {
  const { state, send } = harness();
  state.editForm.value.title = 'My custom title';
  send('found', { current: 1, track: saved });
  assert.equal(state.editForm.value.title, 'My custom title');
  assert.equal(state.editForm.value.artist, saved.artist);
  assert.equal(state.editForm.value.album, saved.album);
  assert.equal(state.nowPlaying.value.title, saved.title);
});

test('the log contains only terminal results, with no duplicate row for the same track', () => {
  const { state, send } = harness();
  send('not_found', { current: 1, total: 2 });
  send('fingerprinting', { current: 1, total: 2, track_id: 2 });
  assert.equal(state.identifyResults.value.length, 1);
  send('error', { current: 2, total: 2, track_id: 2, message: 'Could not save' });
  send('error', { current: 2, total: 2, track_id: 2, message: 'Could not save metadata' });
  send('done', { current: 2, total: 2, track_id: 0 });
  assert.equal(state.identifyCurrent.value, 2);
  assert.deepEqual(plain(state.identifyResults.value.map(item => item.status)), ['not_found', 'error']);
  assert.equal(state.identifyResults.value[1].message, 'Could not save metadata');
  assert.equal(state.identifyResults.value[1].segmentIndex, 1);
  assert.equal(state.nowPlaying.value.title, 'Old title');
});

test('the active file occupies exactly one share while finished progress stays static', () => {
  const progress = progressSegments(4, 1, [], true);
  assert.equal(progress.completedPercent, 25);
  assert.deepEqual(progress.active, { left: 25, width: 25 });
  const finished = progressSegments(4, 2, [], false);
  assert.equal(finished.completedPercent, 50);
  assert.equal(finished.active, null);
  assert.deepEqual(progressSegments(4, 2, [], true).active, { left: 50, width: 25 });
  assert.deepEqual(progressSegments(1, 0, [], true).active, { left: 0, width: 100 });
  assert.equal(progressSegments(1, 1, [], false).active, null);
});

test('failed portions stay in their original positions through later processing and completion', () => {
  const running = progressSegments(4, 3, [0, 2, 0], true);
  assert.deepEqual(running.errors, [{ start: 0, count: 1, left: 0, width: 25 }, { start: 2, count: 1, left: 50, width: 25 }]);
  assert.equal(running.failedCount, 2);
  assert.deepEqual(running.active, { left: 75, width: 25 });
  const done = progressSegments(4, 4, [0, 2], false);
  assert.deepEqual(done.errors, running.errors);
  assert.equal(done.completedPercent, 100);
  assert.equal(done.active, null);
  const allFailed = progressSegments(4, 4, [0, 1, 2, 3], false);
  assert.deepEqual(allFailed.errors, [{ start: 0, count: 4, left: 0, width: 100 }]);
});

test('only the active portion has a 5% minimum, including at the right edge', () => {
  const running = progressSegments(100, 30, [10], true);
  assert.deepEqual(running.active, { left: 30, width: 5 });
  assert.equal(running.completedPercent, 30);
  assert.deepEqual(running.errors, [{ start: 10, count: 1, left: 10, width: 1 }]);
  const finished = progressSegments(100, 31, [10, 30], false);
  assert.equal(finished.active, null);
  assert.equal(finished.completedPercent, 31);
  assert.deepEqual(finished.errors[1], { start: 30, count: 1, left: 30, width: 1 });
  assert.deepEqual(progressSegments(100, 99, [], true).active, { left: 95, width: 5 });
  assert.deepEqual(progressSegments(20, 19, [], true).active, { left: 95, width: 5 });
  assert.equal(progressSegments(100, 100, [], true).active, null);
});

test('only completed error results make gray portions; unmatched files finish normally', () => {
  const { state, send } = harness();
  send('error', { current: 1, total: 3 });
  send('not_found', { current: 2, total: 3, track_id: 2 });
  send('looking_up', { current: 2, total: 3, track_id: 3 });
  const failed = state.identifyResults.value.filter(item => item.status === 'error').map(item => item.segmentIndex);
  const segments = progressSegments(state.identifyTotal.value, state.identifyCurrent.value, failed, state.identifyStage.value === 'looking_up');
  assert.equal(segments.failedCount, 1);
  assert.equal(segments.errors[0].start, 0);
  assert.ok(Math.abs(segments.active.left - 100 / 3 * 2) < 1e-10);
  assert.equal(progressSegments(0, 0, [], true).active, null);
  assert.deepEqual(progressSegments(2, 1, [-1, 1, 100, NaN], true).errors, []);
});
