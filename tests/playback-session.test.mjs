import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
import ts from 'typescript';

const compile = source => ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText;
const helpers = {};
new Function('exports', compile(readFileSync(new URL('../src/utils/playbackSession.ts', import.meta.url), 'utf8')))(helpers);
const app = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8').split('<script setup lang="ts">')[1].split('</script>')[0];
const parsed = ts.createSourceFile('App.ts', app, ts.ScriptTarget.Latest, true);
const functions = ['restorePlaybackSession', 'refreshPlaybackState', 'applyRemotePlaybackStatus'];
const source = parsed.statements.filter(statement => ts.isFunctionDeclaration(statement) && functions.includes(statement.name?.text)).map(statement => statement.getText(parsed)).join('\n');
assert.equal(functions.filter(name => source.includes(`function ${name}(`)).length, functions.length);

const track = (id, liked = false) => ({ id, path: `${id}.flac`, title: `Track ${id}`, is_liked: liked, file_hash: String(id), duration_secs: 200 });
const saved = overrides => ({ version: 1, libraryDir: '/music', sourceId: 'session-1', trackId: 900,
  source: 'playlist', sourceIds: [900, 901, 902], queueIds: [901, 902], sourceIndex: 2,
  pagedSource: null, shuffled: true, repeat: 1, remotePeer: null, ...overrides });
function harness({ cache = saved(), playing = true, sourceId = 'session-1', active = track(900, true), sessionRequest, remoteStatus } = {}) {
  const calls = [];
  const values = ['libraryDataDir', 'nativePlaybackSourceId', 'volume', 'isPlaying', 'currentTime', 'duration', 'nowPlaying', 'queueSource', 'queueLibraryTracks', 'queuePlaylistTracks', 'queue', 'queueSourceIndex', 'queuedPagedSource', 'isShuffled', 'repeatMode', 'recentTracks', 'smartPlaylists', 'nativeSampleRate', 'remoteOutputPeer', 'deviceMenuError', 'seekPreviewPos'];
  const state = Object.fromEntries(values.map(name => [name, { value: null }]));
  Object.assign(state, { playbackRestoring: { value: true }, playbackTransitionSequence: 0, playbackRestoreDisposed: false, ticks: 0, notifications: 0, errors: [], ...helpers });
  state.queue.value = []; state.smartPlaylists.value = []; state.remoteOutputPeer.value = null;
  state.sessionStorage = { getItem: () => cache ? JSON.stringify(cache) : null };
  state.console = { error: (...args) => state.errors.push(args) };
  state.startTicker = () => { state.ticks++; };
  state.syncAndroid = () => { state.notifications++; };
  state.handleFinishedPlayback = () => {};
  state.remotePeerArgs = peer => ({ peerHost: peer.host, peerName: peer.name, peerPort: peer.port });
  state.peerDeviceNames = { value: {} };
  state.lastRemoteBeatSeq = 0;
  const status = () => ({ source_id: sourceId, playing, finished: false, position: 42, duration: 200, source_sample_rate: 96000 });
  const session = { library_dir: '/music', playback: status(), volume: 0.38,
    source: sourceId ? { path: `/music/${active?.path}`, follow_growing: false, preview: null } : null,
    track: active, tracks: [track(900, true), track(901), track(902)] };
  state.invoke = async (command, args) => {
    calls.push([command, args]);
    if (command === 'playback_session') return sessionRequest ? sessionRequest(session) : session;
    if (command === 'playback_status') return status();
    if (command === 'remote_playback_status' && remoteStatus) return remoteStatus;
    throw Error(`Restoration must not invoke ${command}`);
  };
  state.trackForHash = hash => session.tracks.find(track => track.file_hash === hash) ?? null;
  vm.createContext(state);
  vm.runInContext(compile(source), state);
  return { state, calls, session };
}

test('reload recovers a playing track outside the first library page, queue, likes, position and volume without replaying', async () => {
  const { state, calls } = harness();
  await state.restorePlaybackSession();
  assert.equal(state.nowPlaying.value.id, 900);
  assert.equal(state.nowPlaying.value.is_liked, true);
  assert.equal(state.isPlaying.value, true);
  assert.equal(state.currentTime.value, 42);
  assert.equal(state.volume.value, 38);
  assert.equal(state.nativeSampleRate.value, 96000);
  assert.equal(state.queue.value.map(track => track.id).join(','), '901,902');
  assert.equal(state.queuePlaylistTracks.value.length, 3);
  assert.equal(state.queueSourceIndex.value, 2);
  assert.equal(state.repeatMode.value, 1);
  assert.equal(state.ticks, 1);
  assert.equal(state.playbackRestoring.value, false);
  assert.deepEqual(calls.map(([command]) => command), ['playback_session', 'playback_status']);
  assert.equal(state.errors.length, 0);
});

test('reload preserves pause and a mismatched cached session cannot replace the native track or queue', async () => {
  const { state } = harness({ playing: false, sourceId: 'new-session', active: track(999) });
  await state.restorePlaybackSession();
  assert.equal(state.nowPlaying.value.id, 999);
  assert.equal(state.isPlaying.value, false);
  assert.equal(state.queue.value.length, 0);
  assert.equal(state.ticks, 0);
});

test('a user track change while native recovery is pending wins over the old snapshot', async () => {
  let resolve;
  const { state } = harness({ sessionRequest: session => new Promise(done => { resolve = () => done(session); }) });
  const restoring = state.restorePlaybackSession();
  state.playbackTransitionSequence++;
  state.nowPlaying.value = track(1000);
  resolve();
  await restoring;
  assert.equal(state.nowPlaying.value.id, 1000);
  assert.equal(state.playbackRestoring.value, false);
  assert.equal(state.ticks, 0);
});

test('stopped native playback does not resurrect the cached current track', async () => {
  const { state } = harness({ playing: false, sourceId: null });
  await state.restorePlaybackSession();
  assert.equal(state.nowPlaying.value, null);
  assert.equal(state.isPlaying.value, false);
  assert.equal(state.duration.value, 0);
  assert.equal(state.ticks, 0);
});

test('reload reconnects the selected remote output and reads its live state without restarting it', async () => {
  const peer = { name: 'desk', host: '192.0.2.1', port: 57322, addresses: ['192.0.2.1'] };
  const { state, calls } = harness({ playing: false, active: track(901),
    cache: saved({ sourceId: 'remote:desk:900', remotePeer: peer }),
    remoteStatus: { version: 'test', playback: { state: 'playing', hash: '900', position: 55, duration: 200, volume: 0.64 } },
  });
  await state.restorePlaybackSession();
  assert.equal(state.nowPlaying.value.id, 900);
  assert.equal(state.remoteOutputPeer.value.host, peer.host);
  assert.equal(state.isPlaying.value, true);
  assert.equal(state.currentTime.value, 55);
  assert.equal(state.volume.value, 64);
  assert.equal(state.queue.value.map(track => track.id).join(','), '901,902');
  assert.equal(state.ticks, 1);
  assert.equal(state.errors.length, 0);
  assert.deepEqual(calls.map(([command]) => command), ['playback_session', 'remote_playback_status', 'remote_playback_status']);
});

test('bad cache and a cache for a different library or engine session are ignored', () => {
  assert.equal(helpers.readPlaybackContext('{broken'), null);
  assert.equal(helpers.readPlaybackContext(JSON.stringify(saved({ queueIds: 'wrong' }))), null);
  const context = helpers.readPlaybackContext(JSON.stringify(saved()));
  assert.equal(helpers.matchesPlaybackContext(context, '/other', 'session-1', 900), false);
  assert.equal(helpers.matchesPlaybackContext(context, '/music', 'session-2', 900), false);
  assert.equal(helpers.matchesPlaybackContext(context, '/music', 'session-1', 901), false);
});
