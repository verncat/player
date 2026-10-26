import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
import ts from 'typescript';

const names = ['releaseSeekPointer', 'cancelSeekGesture', 'seekPosFromEvent', 'mobileSeekHoldStart', 'mobileSeekHoldMove', 'mobileSeekHoldEnd', 'seek'];
const source = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8').split('<script setup lang="ts">')[1].split('</script>')[0];
const parsed = ts.createSourceFile('App.ts', source, ts.ScriptTarget.Latest, true);
const functions = parsed.statements.filter(node => ts.isFunctionDeclaration(node) && names.includes(node.name?.text)).map(node => node.getText(parsed)).join('\n');
const compiled = ts.transpileModule(functions, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;

function harness(playing = true) {
  const state = { activeSeekGesture: null, seekRequestSequence: 0, playbackTransitionSequence: 0,
    mobileSeekActive: { value: false }, seekPreviewPos: { value: null }, nowPlaying: { value: { id: 1 } },
    duration: { value: 200 }, currentTime: { value: 20 }, isPlaying: { value: playing },
    playbackRestoring: { value: false }, playbackRestorePromise: null, deviceMenuError: { value: '' },
    calls: [], ticks: 0, stops: 0, nativePosition: 20,
  };
  let captured = null;
  const element = {
    getBoundingClientRect: () => ({ left: 100, width: 200 }), focus() {},
    setPointerCapture: id => { captured = id; }, hasPointerCapture: id => captured === id,
    releasePointerCapture: () => { captured = null; },
  };
  const event = (type, clientX, pointerId = 1) => ({ type, clientX, pointerId, currentTarget: element, button: 0, preventDefault() {} });
  state.seekCurrentOutput = async position => { state.calls.push(position); state.nativePosition = position; };
  state.refreshPlaybackState = async () => ({ playing, finished: false, position: state.nativePosition });
  state.startTicker = () => state.ticks++;
  state.stopTicker = () => state.stops++;
  state.syncAndroid = () => {};
  state.handleFinishedPlayback = () => {};
  state.console = console;
  vm.createContext(state);
  vm.runInContext(compiled, state);
  return { state, event, captured: () => captured };
}

test('drag previews the position and seeks once on release, including outside the bar', async () => {
  const { state, event, captured } = harness();
  state.mobileSeekHoldStart(event('pointerdown', 150));
  state.mobileSeekHoldMove(event('pointermove', 250));
  assert.equal(state.currentTime.value, 150);
  assert.equal(state.seekPreviewPos.value, 150);
  assert.equal(state.calls.length, 0);
  assert.equal(captured(), 1);
  await state.mobileSeekHoldEnd(event('pointerup', 500));
  await state.mobileSeekHoldEnd(event('lostpointercapture', 0));
  assert.deepEqual(state.calls, [200]);
  assert.equal(captured(), null);
  assert.equal(state.mobileSeekActive.value, false);
  assert.equal(state.seekPreviewPos.value, null);
  assert.equal(state.ticks, 1);
});

test('cancelled pointer capture uses the last preview and seeking preserves pause', async () => {
  const { state, event } = harness(false);
  state.mobileSeekHoldStart(event('pointerdown', 150));
  state.mobileSeekHoldMove(event('pointermove', 180));
  await state.mobileSeekHoldEnd(event('pointercancel', 0));
  assert.deepEqual(state.calls, [80]);
  assert.equal(state.isPlaying.value, false);
  assert.equal(state.ticks, 0);
  assert.equal(state.stops, 1);
});

test('another pointer and a track change cannot apply the old drag to the new track', async () => {
  const { state, event } = harness();
  state.mobileSeekHoldStart(event('pointerdown', 150));
  state.mobileSeekHoldMove(event('pointermove', 250, 2));
  assert.equal(state.seekPreviewPos.value, 50);
  state.nowPlaying.value = { id: 2 };
  state.playbackTransitionSequence++;
  await state.mobileSeekHoldEnd(event('pointerup', 250));
  assert.equal(state.calls.length, 0);
  assert.equal(state.seekPreviewPos.value, null);
});

test('completion of an older seek cannot clear the preview of a new drag', async () => {
  const { state, event } = harness();
  let done;
  state.seekCurrentOutput = () => new Promise(resolve => { done = resolve; });
  state.mobileSeekHoldStart(event('pointerdown', 150));
  const first = state.mobileSeekHoldEnd(event('pointerup', 200));
  state.mobileSeekHoldStart(event('pointerdown', 250, 2));
  done();
  await first;
  assert.equal(state.seekPreviewPos.value, 150);
  assert.equal(state.mobileSeekActive.value, true);
  state.cancelSeekGesture();
});

test('keyboard seeking consumes arrow keys so they do not also skip tracks', () => {
  const component = readFileSync(new URL('../src/components/PlayerBar.vue', import.meta.url), 'utf8').split('<script setup lang="ts">')[1].split('</script>')[0];
  const file = ts.createSourceFile('PlayerBar.ts', component, ts.ScriptTarget.Latest, true);
  const keyboard = file.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === 'seekKeydown');
  const calls = [];
  const state = { props: { nowPlaying: { id: 1 }, duration: 200, currentTime: 40 }, emit: (...args) => calls.push(args) };
  vm.createContext(state);
  vm.runInContext(ts.transpileModule(keyboard.getText(file), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, state);
  let prevented = false, stopped = false;
  state.seekKeydown({ key: 'ArrowRight', repeat: false, preventDefault() { prevented = true; }, stopPropagation() { stopped = true; } });
  assert.deepEqual(calls, [['seek', 45]]);
  assert.equal(prevented, true);
  assert.equal(stopped, true);
});
