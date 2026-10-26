import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { hashObservationStream, validateRecording } from './browser_file_replay.mjs';

function framed(payload) {
  const header = Buffer.alloc(4); header.writeUInt32LE(payload.length);
  return Buffer.concat([header, payload]);
}
async function* chunks(bytes, width) {
  for (let offset = 0; offset < bytes.length; offset += width) yield bytes.subarray(offset, offset + width);
}
test('canonical hashes preserve all bytes across every header/payload chunk boundary', async () => {
  const payloads = [Buffer.from('authored observation one'), Buffer.from([1, 2, 3, 4, 5]), Buffer.from('authored observation three')];
  const stream = Buffer.concat(payloads.map(framed));
  const expected = payloads.map(payload => createHash('sha1').update(payload).digest('hex'));
  for (let width = 1; width <= stream.length; width++) {
    const result = await hashObservationStream(chunks(stream, width));
    assert.deepEqual(result.hashes, expected);
    assert.equal(result.totalBytes, payloads.reduce((sum, payload) => sum + payload.length, 0));
  }
});
test('malformed, truncated and oversized observation streams reject without receipts', async () => {
  for (const bytes of [Buffer.from([1]), framed(Buffer.alloc(0)), Buffer.from([2, 0, 0, 0, 1]), Buffer.from([1, 0, 0, 1])]) {
    await assert.rejects(hashObservationStream(chunks(bytes, 1)));
  }
});
test('file proof only accepts complete two-pad power-on recordings', () => {
  const recording = { format: 1, start: { kind: 'power_on' }, frames: 2, runs: [{ frames: 2, pads: [0x100, 0] }] };
  validateRecording(recording);
  for (const changed of [
    { ...recording, frames: 3 }, { ...recording, start: { kind: 'recipe' } },
    { ...recording, runs: [{ frames: 2, pads: [0x200, 0] }] },
    { ...recording, runs: [{ frames: 2, pads: [0x100, 1] }] },
    { ...recording, runs: [{ frames: 2, pads: [0x100, 0, 0, 0] }] },
  ]) assert.throws(() => validateRecording(changed));
});
test('slow browser compilation settles through a binding, never a pending CDP evaluation', async () => {
  const page = await readFile(new URL('./browser_file_replay.html', import.meta.url), 'utf8');
  const driver = await readFile(new URL('./browser_file_replay.mjs', import.meta.url), 'utf8');
  assert(page.includes('this.result = run(recording, expected).then'));
  assert(page.includes('globalThis.tjeReplayComplete(JSON.stringify'));
  assert(page.includes("globalThis.tjeReplayReady('ready')"));
  assert(!driver.includes('awaitPromise: true'));
  assert(!driver.includes('createServer'));
});
