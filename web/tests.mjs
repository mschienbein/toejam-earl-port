import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import { gamepadMask } from './input.js';
import { readSettings, saveSettings } from './storage.js';
import { AudioStream } from './audio.js';

function standardPad() {
  return { mapping: 'standard', axes: [0, 0], buttons: Array.from({ length: 17 }, () => ({ pressed: false, value: 0 })) };
}

test('standard gamepad buttons map independently to the Genesis wire order', () => {
  for (const [button, bit] of [[12, 1], [13, 2], [14, 4], [15, 8], [0, 16], [1, 32], [2, 64], [9, 128]]) {
    const pad = standardPad();
    pad.buttons[button].pressed = true;
    assert.equal(gamepadMask(pad), bit);
  }
  assert.equal(gamepadMask(null), 0);
  const unsupported = standardPad();
  unsupported.mapping = '';
  unsupported.buttons[0].pressed = true;
  assert.equal(gamepadMask(unsupported), 0);
});

test('gamepad deadzone, analog thresholds and simultaneous buttons', () => {
  const pad = standardPad();
  pad.axes = [0.34, -0.34];
  assert.equal(gamepadMask(pad), 0);
  pad.axes = [0.8, -0.8];
  pad.buttons[1].value = 0.75;
  assert.equal(gamepadMask(pad), 1 | 8 | 32);
});

test('local settings reject malformed data and clamp numeric volume', () => {
  const store = new Map();
  globalThis.localStorage = { getItem: key => store.get(key), setItem: (key, value) => store.set(key, value) };
  assert.deepEqual(readSettings(), { volume: 0.7, scale: 'integer', p2: false });
  saveSettings({ volume: 0.45, scale: 'fit', p2: true });
  assert.deepEqual(readSettings(), { volume: 0.45, scale: 'fit', p2: true });
  saveSettings({ volume: 8, scale: 'bad', p2: 'true' });
  assert.deepEqual(readSettings(), { volume: 1, scale: 'integer', p2: false });
  store.set('tje-settings-v1', '{not json');
  assert.deepEqual(readSettings(), { volume: 0.7, scale: 'integer', p2: false });
  delete globalThis.localStorage;
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
