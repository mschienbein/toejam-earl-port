import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import { defaultPlayer, gamepadMask, Inputs, normalizePlayer, selectGamepads, selectLocalGamepads, validKeyboardCode } from './input.js';
import { defaultSettings, readSettings, saveSettings, SETTINGS_KEY, SETTINGS_VERSION } from './storage.js';
import { AudioStream } from './audio.js';
import { checkDependencies } from './check-dependencies.mjs';
import { Conveniences, shortcutAvailable, stateSlot } from './convenience.js';
import { validateJamBindings } from './jam.js';

function standardPad() {
  return { id: 'Test pad', index: 0, connected: true, mapping: 'standard', axes: [0, 0], buttons: Array.from({ length: 17 }, () => ({ pressed: false, value: 0 })) };
}

test('standard gamepad buttons map independently to the Genesis wire order', () => {
  for (const [button, bit] of [[12, 1], [13, 2], [14, 4], [15, 8], [0, 16], [1, 32], [2, 64], [9, 128]]) {
    const pad = standardPad();
    pad.buttons[button].pressed = true;
    assert.equal(gamepadMask(pad), bit);
  }
  assert.equal(gamepadMask(null), 0);
  const unmapped = standardPad();
  unmapped.mapping = '';
  unmapped.buttons[0].pressed = true;
  assert.equal(gamepadMask(unmapped), 16, 'non-standard pads remain usable and remappable');
});

test('Jam modifier remains outside ordinary controller masks and defaults off', () => {
  const settings = defaultSettings();
  assert.equal(settings.jam.enabled, false);
  validateJamBindings(settings.players);
  const pad = standardPad();
  pad.buttons[4].pressed = true;
  assert.equal(gamepadMask(pad, settings.players[0].gamepad.bindings), 0);
  settings.players[0].keys.jam = settings.players[1].keys.a;
  assert.throws(() => validateJamBindings(settings.players), /conflicts/);
});

test('Jam settings roundtrip without changing faithful input bindings', () => {
  const values = new Map();
  const storage = { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key) };
  const settings = defaultSettings();
  settings.jam = { enabled: true, sounds: false, soundsWhileBusy: true, chord: true, backing: 6 };
  saveSettings(settings, storage);
  const restored = readSettings(storage).settings;
  assert.deepEqual(restored.jam, settings.jam);
  assert.deepEqual(restored.players, settings.players);
  values.set(SETTINGS_KEY, JSON.stringify({ version: SETTINGS_VERSION }));
  assert.equal(readSettings(storage).settings.jam.enabled, false);
});

test('gamepad deadzone, analog thresholds and simultaneous buttons', () => {
  const pad = standardPad();
  pad.axes = [0.34, -0.34];
  assert.equal(gamepadMask(pad), 0);
  pad.axes = [0.8, -0.8];
  pad.buttons[1].value = 0.75;
  assert.equal(gamepadMask(pad), 1 | 8 | 32);
});

test('per-player mappings support non-standard buttons and axes with nonzero neutral positions', () => {
  const p1 = defaultPlayer(0);
  const p2 = defaultPlayer(1);
  p1.gamepad.bindings.a = [{ kind: 'button', index: 5 }];
  p2.gamepad.bindings.b = [{ kind: 'axis', index: 2, center: -1, direction: 1, threshold: 0.5 }];
  const pad = standardPad();
  pad.mapping = '';
  pad.axes.push(-1);
  pad.buttons[5].pressed = true;
  assert.equal(gamepadMask(pad, p1.gamepad.bindings), 16);
  assert.equal(gamepadMask(pad, p2.gamepad.bindings), 0);
  pad.axes[2] = 1;
  assert.equal(gamepadMask(pad, p2.gamepad.bindings), 32);
  p2.gamepad.bindings.b = [];
  assert.equal(gamepadMask(pad, p2.gamepad.bindings), 0, 'a deliberately cleared binding stays clear');
});

test('explicit and automatic pad assignments do not share a device or replace an absent explicit choice', () => {
  const pads = [standardPad(), { ...standardPad(), index: 1 }, { ...standardPad(), id: 'Other pad', index: 2 }];
  const players = [defaultPlayer(0), defaultPlayer(1)];
  players[0].gamepad.device = { id: 'Test pad', index: 1 };
  assert.deepEqual(selectGamepads(pads, players), [pads[1], pads[0]]);
  assert.equal(selectGamepads(pads.slice(1), players)[0], pads[1], 'removing an earlier slot must not reassign the surviving pad');
  players[1].gamepad.device = { id: 'Test pad', index: 1 };
  assert.deepEqual(selectGamepads(pads, players), [pads[1], null]);
  players[0].gamepad.device = { id: 'Disconnected pad', index: 0 };
  players[1].gamepad.device = null;
  assert.deepEqual(selectGamepads(pads, players), [null, pads[0]]);
  players[1].gamepad.device = 'none';
  assert.deepEqual(selectGamepads(pads, players), [null, null]);
});

test('remapped keyboard inputs stay independent and release after focus changes or blur', () => {
  const previousWindow = globalThis.window;
  const previousElement = globalThis.Element;
  const listeners = new Map();
  class TestElement { constructor(editing = false) { this.editing = editing; } closest() { return this.editing ? this : null; } }
  globalThis.window = { addEventListener: (type, listener) => listeners.set(type, listener) };
  globalThis.Element = TestElement;
  try {
    const players = [defaultPlayer(0), defaultPlayer(1)];
    players[0].keys.up = 'KeyO';
    players[1].keys.up = 'KeyP';
    const inputs = new Inputs(players, () => []);
    const event = (code, editing = false) => ({ code, target: new TestElement(editing), preventDefault() {}, repeat: false });
    inputs.key(event('KeyO'), true);
    inputs.key(event('KeyP'), true);
    inputs.samplePads();
    assert.deepEqual(inputs.poll(true), [1, 1, true]);
    inputs.key(event('KeyO', true), false);
    assert.deepEqual(inputs.poll(true), [0, 1, true]);
    inputs.key(event('KeyO', true), true);
    assert.deepEqual(inputs.poll(true), [0, 1, true], 'editing controls does not move the player');
    listeners.get('blur')();
    assert.deepEqual(inputs.poll(true), [0, 0, true]);
    inputs.readPads = () => { throw new Error('Gamepad permission denied'); };
    inputs.samplePads();
    assert.match(inputs.gamepadError, /permission denied/);
    inputs.key(event('KeyO'), true);
    assert.equal(inputs.poll(false)[0], 1, 'keyboard survives an unavailable Gamepad API');
  } finally {
    globalThis.window = previousWindow;
    globalThis.Element = previousElement;
  }
});

test('reserved navigation/fullscreen keys and malformed persisted sources are normalized safely', () => {
  assert.equal(validKeyboardCode('KeyO'), true);
  for (const code of ['KeyF', 'Escape', 'Tab', 'ControlLeft', 'Unidentified', '<script>']) assert.equal(validKeyboardCode(code), false);
  const value = normalizePlayer({ keys: { up: 'KeyF' }, gamepad: { bindings: { a: [{ kind: 'axis', index: -1 }] } } }, 0);
  assert.equal(value.keys.up, 'ArrowUp');
  assert.deepEqual(value.gamepad.bindings.a, defaultPlayer(0).gamepad.bindings.a);
});

function memoryStorage(entries = []) {
  const values = new Map(entries);
  return { values, getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key) };
}

test('legacy settings migrate once without losing volume/scaling/P2 choices', () => {
  const storage = memoryStorage([['tje-settings-v1', JSON.stringify({ volume: 0.45, scale: 'fit', p2: true })]]);
  const result = readSettings(storage);
  assert.equal(result.persistable, true);
  assert.equal(result.settings.version, SETTINGS_VERSION);
  assert.equal(result.settings.volume, 0.45);
  assert.equal(result.settings.scale, 'fit');
  assert.equal(result.settings.pixelShape, 'square');
  assert.equal(result.settings.p2, true);
  assert.equal(result.settings.filter, 'sharp');
  assert.equal(result.settings.displayShader, 'off');
  assert.match(result.notice, /migrated/);
  assert.equal(storage.getItem('tje-settings-v1'), null);
  assert.equal(JSON.parse(storage.getItem(SETTINGS_KEY)).version, SETTINGS_VERSION);
  assert.equal(readSettings(storage).notice, '');
});

test('fresh and unset display settings use Native integer with sharp bilinear', () => {
  for (const value of [null, { version: 2, volume: 0.4 }, { version: SETTINGS_VERSION }]) {
    const storage = memoryStorage(value ? [[SETTINGS_KEY, JSON.stringify(value)]] : []);
    const settings = readSettings(storage).settings;
    assert.equal(settings.pixelShape, 'native');
    assert.equal(settings.scale, 'integer');
    assert.equal(settings.filter, 'sharp');
    assert.equal(settings.displayShader, 'off');
  }
});

test('each explicitly stored old display mode and filter retains its shape on migration and reload', () => {
  for (const version of [undefined, 2]) {
    for (const scale of ['integer', 'fit']) {
      for (const filter of ['nearest', 'linear']) {
        const key = version === undefined ? 'tje-settings-v1' : SETTINGS_KEY;
        const storage = memoryStorage([[key, JSON.stringify({ version, scale, filter })]]);
        const first = readSettings(storage).settings;
        assert.equal(first.pixelShape, 'square');
        assert.equal(first.scale, scale);
        assert.equal(first.filter, filter);
        assert.deepEqual(readSettings(storage).settings, first);
      }
    }
  }
});

test('all new pixel-shape and scale combinations round-trip independently', () => {
  for (const pixelShape of ['native', 'square']) {
    for (const scale of ['integer', 'fit']) {
      const storage = memoryStorage();
      saveSettings({ ...defaultSettings(), pixelShape, scale }, storage);
      const restored = readSettings(storage).settings;
      assert.equal(restored.pixelShape, pixelShape);
      assert.equal(restored.scale, scale);
      assert.equal(restored.filter, 'sharp');
    }
  }
});

test('display shaders are opt-in, round-trip independently and reject unknown presets', () => {
  for (const displayShader of ['off', 'smooth', 'crt', 'crt-curved']) {
    const storage = memoryStorage();
    saveSettings({ ...defaultSettings(), displayShader, pixelShape: 'square', scale: 'fit', filter: 'nearest' }, storage);
    const restored = readSettings(storage).settings;
    assert.equal(restored.displayShader, displayShader);
    assert.equal(restored.pixelShape, 'square');
    assert.equal(restored.scale, 'fit');
    assert.equal(restored.filter, 'nearest');
  }
  const storage = memoryStorage([[SETTINGS_KEY, JSON.stringify({ version: SETTINGS_VERSION, displayShader: 'unknown' })]]);
  assert.equal(readSettings(storage).settings.displayShader, 'off');
});

test('extended view is default off and only explicit experimental widths persist', () => {
  assert.equal(defaultSettings().extendedView, 0);
  for (const extendedView of [400, 408, 432, 504, 512]) {
    const storage = memoryStorage();
    saveSettings({ ...defaultSettings(), extendedView }, storage);
    assert.equal(readSettings(storage).settings.extendedView, extendedView);
  }
  for (const extendedView of [320, 399, 401, 409, 520, '400', true]) {
    const storage = memoryStorage([[SETTINGS_KEY, JSON.stringify({ version: SETTINGS_VERSION, extendedView })]]);
    assert.equal(readSettings(storage).settings.extendedView, 0);
  }
});

test('versioned mappings, filters and device choices persist; malformed/future versions recover without destructive migration', () => {
  const storage = memoryStorage();
  const settings = defaultSettings();
  settings.volume = 8;
  settings.filter = 'linear';
  settings.players[0].keys.up = 'KeyO';
  settings.players[1].gamepad.device = { id: 'Unmapped controller', index: 1 };
  settings.players[1].gamepad.bindings.b = [{ kind: 'axis', index: 2, direction: 1, center: -1, threshold: 0.5 }];
  saveSettings(settings, storage);
  const restored = readSettings(storage).settings;
  assert.equal(restored.volume, 1);
  assert.equal(restored.filter, 'linear');
  assert.equal(restored.players[0].keys.up, 'KeyO');
  assert.deepEqual(restored.players[1].gamepad, settings.players[1].gamepad);
  const future = JSON.stringify({ version: SETTINGS_VERSION + 1, futureOption: true });
  storage.setItem(SETTINGS_KEY, future);
  assert.equal(readSettings(storage).persistable, false);
  assert.equal(storage.getItem(SETTINGS_KEY), future);
  storage.setItem(SETTINGS_KEY, '{not json');
  assert.equal(readSettings(storage).persistable, true, 'new explicit choices may repair malformed settings');
  assert.deepEqual(readSettings(storage).settings, defaultSettings());
});

test('failed migration leaves the legacy copy and its in-memory choices intact', () => {
  const storage = memoryStorage([['tje-settings-v1', JSON.stringify({ volume: 0.25, scale: 'fit' })]]);
  storage.setItem = () => { throw new Error('Storage full'); };
  const result = readSettings(storage);
  assert.equal(result.settings.volume, 0.25);
  assert.equal(result.settings.scale, 'fit');
  assert.equal(result.persistable, false);
  assert.notEqual(storage.getItem('tje-settings-v1'), null);
  const denied = readSettings({ getItem() { throw new Error('Storage denied'); } });
  assert.equal(denied.persistable, false);
});

async function worklet() {
  const messages = [];
  let Processor;
  class AudioWorkletProcessor {
    constructor() { this.port = { postMessage: message => messages.push(message) }; }
  }
  const scope = {
    AudioWorkletProcessor, Float32Array, Array, Math, sampleRate: 48000, currentFrame: 0,
    registerProcessor: (name, constructor) => { assert.equal(name, 'tje-cartridge-audio'); Processor = constructor; },
  };
  vm.runInNewContext(await readFile(new URL('./audio-worklet.js', import.meta.url), 'utf8'), scope);
  const processor = new Processor();
  const send = data => processor.port.onmessage({ data });
  const quantum = () => {
    const output = [new Float32Array(128), new Float32Array(128)];
    assert.equal(processor.process([], [output]), true);
    scope.currentFrame += 128;
    return output;
  };
  send({ type: 'flush', epoch: 1, active: true });
  return { processor, send, quantum, messages };
}

function block(frames, epoch = 1) {
  const samples = new Float32Array(frames * 2);
  for (let i = 0; i < frames; i++) { samples[i * 2] = 0.25; samples[i * 2 + 1] = -0.5; }
  return { type: 'samples', epoch, length: samples.length, buffer: samples.buffer };
}

test('worklet outputs stereo samples, recycles blocks, and zero-fills underruns', async () => {
  const { processor, send, quantum, messages } = await worklet();
  send(block(2048));
  const [left, right] = quantum();
  assert(left.every(value => value === 0.25));
  assert(right.every(value => value === -0.5));
  for (let i = 1; i < 16; i++) quantum();
  assert.equal(processor.played, 2048);
  assert.equal(processor.nonzero, 2048);
  assert.equal(processor.queued, 0);
  assert.equal(messages.filter(message => message.type === 'recycle').length, 1);
  assert(quantum()[0].every(value => value === 0));
  assert.equal(processor.underruns, 128);
});

test('worklet queue is bounded and flush isolates epochs without retaining sample blocks', async () => {
  const { processor, send, quantum, messages } = await worklet();
  send(block(10000));
  send(block(4000));
  assert.equal(processor.dropped, 4000);
  assert.equal(processor.queued, 10000);
  send({ type: 'flush', epoch: 2, active: false });
  assert.equal(processor.queued, 0);
  assert.equal(processor.count, 0);
  assert.equal(processor.blocks.filter(Boolean).length, 0);
  send(block(2048, 1));
  assert.equal(processor.count, 0);
  assert(quantum()[0].every(value => value === 0));
  assert.equal(messages.filter(message => message.type === 'recycle').length, 3);
});

test('video phase lock smooths batched audio-clock updates while tracking device drift', () => {
  const stream = new AudioStream();
  stream.context = { currentTime: 0, sampleRate: 48000 };
  const targetFps = 59.92274340431231;
  let presentations = 0;
  for (let frame = 1; frame <= 3600; frame++) {
    const seconds = frame / 60;
    // Synthetic 1024-sample device batches and a 200-ppm clock difference.
    stream.context.currentTime = Math.floor(seconds * 48000 * 1.0002 / 1024) * 1024 / 48000;
    stream.syncClock(seconds * 1000);
    let steps = 0;
    while (stream.needed() > 0 && steps < 6) {
      stream.produced += 48000 / targetFps;
      steps++;
    }
    if (steps) presentations++;
  }
  assert(presentations >= 3600 * 0.95);
  const lead = stream.produced / 48000 - stream.context.currentTime;
  assert(lead > 0.035 && lead < 0.1, `bounded device lead: ${lead}`);
});

function audioFixture() {
  const stream = new AudioStream();
  const messages = [];
  stream.context = {
    state: 'suspended', sampleRate: 48000, currentTime: 1,
    async resume() { this.state = 'running'; },
    async suspend() { this.state = 'suspended'; },
  };
  stream.ready = Promise.resolve();
  stream.node = { port: { postMessage: message => messages.push(message) } };
  stream.gain = { gain: { setValueAtTime(value) { stream.testVolume = value; } } };
  return { stream, messages };
}

test('audio suspend/resume flushes old epochs and a denied resume remains retryable', async () => {
  const { stream, messages } = audioFixture();
  await stream.start(0.4);
  assert.equal(stream.context.state, 'running');
  assert.equal(stream.testVolume, 0.4);
  await stream.pause();
  assert.equal(stream.context.state, 'suspended');
  assert.equal(messages.at(-1).active, false);
  stream.context.resume = async () => { throw new Error('User gesture required'); };
  await assert.rejects(stream.start(0.5), /User gesture required/);
  assert.equal(stream.context.state, 'suspended');
  stream.context.resume = async function () { this.state = 'running'; };
  await stream.start(0.5);
  assert.equal(messages.at(-1).active, true);
  assert.equal(stream.context.state, 'running');
});

test('pausing during worklet initialization waits for readiness, and a stalled resume times out', async () => {
  const { stream } = audioFixture();
  let ready;
  stream.ready = new Promise(resolve => { ready = resolve; });
  stream.node = null;
  const paused = stream.pause();
  await Promise.resolve();
  stream.node = { port: { postMessage() {} } };
  ready();
  await paused;
  assert.equal(stream.context.state, 'suspended');
  stream.context.resume = () => new Promise(() => {});
  await assert.rejects(stream.start(0.5), /another audio gesture/);
});

test('shipping and observer wasm graphs exclude the 68000, host and browser oracles; release disables observers', () => {
  assert.deepEqual(checkDependencies(), [
    { graph: 'shipping', interpreterFree: true },
    { graph: 'observation-only', interpreterFree: true },
  ]);
});

test('convenience extras are opt-in, persisted strictly and never steal a custom key', async () => {
  const settings = defaultSettings();
  assert.deepEqual(settings.convenience, { saveStates: false, rewind: false, fastForward: false, slot: 0 });
  assert.equal(shortcutAvailable(settings, 'F2'), true);
  settings.players[0].keys.a = 'F2';
  assert.equal(shortcutAvailable(settings, 'F2'), false);
  const values = new Map();
  const storage = { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key) };
  settings.convenience = { saveStates: true, rewind: true, fastForward: true, slot: 4 };
  saveSettings(settings, storage);
  assert.deepEqual(readSettings(storage).settings.convenience, settings.convenience);
  await assert.rejects(stateSlot('put', 'not-a-ROM', 0, new Uint8Array()), /identity/);
  await assert.rejects(stateSlot('get', '0'.repeat(40), 5), /identity/);
});

test('fast-forward is clocked at four times game rate rather than monitor refresh', () => {
  const extra = Object.create(Conveniences.prototype);
  Object.assign(extra, { settings: defaultSettings(), held: new Set(), speedLatched: true, mode: 'normal', fastClock: 0 });
  assert.deepEqual(extra.tick(1000, 60), { rewind: false, fast: false, budget: 0, changed: false });
  extra.settings.convenience.fastForward = true;
  const first = extra.tick(1000, 60).budget;
  assert(first >= 3 && first <= 4, 'the fractional clock remainder is retained');
  const next = extra.tick(1000 + 1000 / 120, 60);
  assert.equal(next.fast, true);
  assert(next.budget >= 2 && next.budget <= 3, '120 Hz display gets about two game frames per presentation');
  assert(extra.tick(10000, 60).budget <= 24, 'a long tab stall cannot create unbounded work');
  extra.settings.convenience.rewind = true;
  extra.held.add('Backspace');
  assert.equal(extra.tick(10010, 60).rewind, true);
  assert.equal(extra.tick(10020, 60).fast, false);
  extra.release();
  assert.equal(extra.held.size, 0);
});

test('enhanced local seats reserve four sources through reordering, disconnect, reconnect and explicit swaps', () => {
  const players = Array.from({ length: 4 }, (_, seat) => defaultPlayer(seat));
  const pads = players.map((_, seat) => ({ ...standardPad(), id: `Owned pad ${seat}`, index: seat }));
  const reservations = [null, null, null, null];
  assert.deepEqual(selectLocalGamepads(pads.toReversed(), players, reservations), pads);
  assert.deepEqual(selectLocalGamepads([pads[3], pads[1], pads[2]], players, reservations), [null, pads[1], pads[2], pads[3]]);
  const newcomer = { ...standardPad(), id: 'Unassigned pad', index: 4 };
  assert.equal(selectLocalGamepads([newcomer, ...pads.slice(1)], players, reservations)[0], null);
  assert.deepEqual(selectLocalGamepads([newcomer, ...pads.toReversed()], players, reservations), pads);
});

test('four enhanced pad words, keyboard focus and bindings remain outside faithful records', () => {
  const oldWindow = globalThis.window, oldElement = globalThis.Element;
  class Target { constructor(editing = false) { this.editing = editing; } closest() { return this.editing; } }
  const listeners = new Map();
  globalThis.window = { addEventListener: (type, fn) => listeners.set(type, fn) };
  globalThis.Element = Target;
  try {
    const settings = defaultSettings();
    settings.enhancedPlayers.forEach(player => { player.gamepad.enabled = true; });
    const pads = Array.from({ length: 4 }, (_, seat) => ({ ...standardPad(), id: `Pad ${seat}`, index: seat }));
    for (let seat = 0; seat < 4; seat++) pads[seat].buttons[[12, 13, 14, 15][seat]].pressed = true;
    const inputs = new Inputs(settings.players, () => pads, settings.enhancedPlayers);
    inputs.samplePads();
    assert.deepEqual(inputs.pollLocal(), [0x101, 0x102, 0x104, 0x108]);
    assert.deepEqual(inputs.poll(true), [1, 2, true]);
    inputs.readPads = () => pads.slice(1);
    inputs.samplePads();
    assert.deepEqual(inputs.pollLocal(), [0x100, 0x102, 0x104, 0x108]);
    inputs.assignLocal(3, { id: 'Pad 1', index: 1 });
    inputs.samplePads();
    assert.equal(inputs.localSelected[3], pads[1]);
    assert.equal(inputs.localSelected[1], pads[3]);
    assert.equal(inputs.localAssignmentEpoch, 1);
    settings.enhancedPlayers[0].keys.a = 'KeyQ';
    const event = editing => ({ code: 'KeyQ', target: new Target(editing), preventDefault() {}, repeat: false });
    inputs.key(event(true), true);
    assert.equal(inputs.pollLocal()[2], 0x104);
    inputs.key(event(false), true);
    assert.equal(inputs.pollLocal()[2], 0x114);
    listeners.get('blur')();
    assert.equal(inputs.pollLocal()[2], 0x104);
    assert.equal(inputs.pollLocal(3)[2], 0);
    const store = memoryStorage(); saveSettings(settings, store);
    assert.deepEqual(readSettings(store).settings.enhancedPlayers, settings.enhancedPlayers);
    assert.equal(defaultSettings().enhancedPlayers[0].gamepad.enabled, false);
  } finally { globalThis.window = oldWindow; globalThis.Element = oldElement; }
});
