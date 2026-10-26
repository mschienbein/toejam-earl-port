// ROM-free service parity. This does not substitute for private S6 core/audio runs.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';

const [native, wasm] = process.argv.slice(2);
assert.ok(native && wasm, 'usage: node wasm-parity.mjs NATIVE_PROOF WASM_PROOF');
const expected = execFileSync(native, [], { encoding: 'utf8' }).trim().split('\n');
const module = await WebAssembly.compile(await readFile(wasm));
assert.deepEqual(WebAssembly.Module.imports(module), [], 'proof must not import host time, RNG, I/O, or services');
const instance = await WebAssembly.instantiate(module, {});
const actual = [0, 2].map((revision) => {
  const bytes = Buffer.alloc(32);
  for (let index = 0; index < 8; index += 1) {
    bytes.writeUInt32LE(instance.exports.overlord_proof_word(revision, index) >>> 0, index * 4);
  }
  return `authored-rev${String(revision).padStart(2, '0')} ${bytes.toString('hex')}`;
});
assert.deepEqual(actual, expected);
console.log(actual.join('\n'));
console.log('Overlord authored authoritative-state native/wasm parity passed; core/audio/campaign not exercised.');
