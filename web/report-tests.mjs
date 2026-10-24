import test from 'node:test';
import assert from 'node:assert/strict';
import { InputRing, Reports, issueUrl, RECORDING_CAPACITY } from './reporting.js';

test('input ring retains every controller frame from power-on, including a failed attempt', () => {
  assert.equal(RECORDING_CAPACITY, 216000);
  const ring = new InputRing('rev02', 3);
  ring.begin(0, 0, false); ring.complete();
  ring.begin(0, 0, false); ring.complete();
  ring.begin(128, 16, true); ring.complete();
  assert.deepEqual(ring.snapshot().session.runs, [
    { frames: 2, pads: [256, 0] }, { frames: 1, pads: [384, 272] },
  ]);
  ring.begin(8, 0, false);
  const pending = ring.snapshot();
  assert.equal(pending.completed_frames, 3);
  assert.equal(pending.session.frames, 4);
  assert.deepEqual(pending.in_flight, [264, 0]);
  ring.complete();
  const tail = ring.snapshot();
  assert.equal(tail.first_frame, 1);
  assert.equal(tail.session, null, 'never mislabel a truncated tail as power-on');
  assert.equal(tail.input_tail.reduce((sum, run) => sum + run.frames, 0), 3);
  ring.reset();
  assert.equal(ring.snapshot().session.frames, 0);
  assert.equal(ring.snapshot().first_frame, 0);
});

test('panic/error deduplication cannot recursively invoke capture', async () => {
  const reporter = Object.create(Reports.prototype);
  reporter.capturing = reporter.crashed = false;
  let captures = 0;
  reporter.onCrash = () => { void reporter.crash('nested', new Error('nested')); };
  reporter.capture = async () => { captures++; await reporter.crash('recursive', new Error('recursive')); };
  await reporter.crash('forced', new Error('forced'));
  assert.equal(captures, 1);
});

test('GitHub issue is only a URL, encoded metadata and attach instructions, not a submission', () => {
  const bundle = {
    id: 'test', kind: 'crash', build: { frontend: 'web', version: 'test', commit: '123', target: 'wasm' },
    environment: { os: 'Linux', architecture: 'x64', browser: 'Chromium' }, rom: { revision: 'rev02', sha1: 'hash-only' },
    engine: { frame: 42 }, failure: { source: 'wasm_panic', message: 'forced ? & #\nsecond line' }, note: 'reproduction steps',
  };
  const url = new URL(issueUrl(bundle));
  assert.equal(url.origin, 'https://github.com');
  assert.equal(url.pathname, '/mschienbein/toejam-earl-port/issues/new');
  assert.match(url.searchParams.get('title'), /forced \? & #/);
  assert.match(url.searchParams.get('body'), /Never attach your ROM/);
  assert.match(url.searchParams.get('body'), /attach it here/);
  assert.match(url.searchParams.get('body'), /Last completed frame: 42/);
  assert.doesNotThrow(() => issueUrl({ ...bundle, note: 'x'.repeat(999) + '\u{20000}' }),
    'truncating a supplementary Unicode character must not break the issue action');
});
