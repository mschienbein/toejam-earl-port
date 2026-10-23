#!/usr/bin/env node
// Build browser/build.sh first. Generated code/private evidence stay under scratch/.
// --rom PATH --session PATH --report PATH implements engine-verify's external protocol.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { basename, dirname, resolve, sep } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const args = new Map();
for (let index = 2; index < process.argv.length; index++) {
  const key = process.argv[index];
  assert(['--rom', '--session', '--report', '--primitives', '--cache-only'].includes(key), `unknown argument ${key}`);
  args.set(key, key === '--primitives' || key === '--cache-only' ? true : process.argv[++index]);
}
const romPath = args.get('--rom');
const sessionPath = args.get('--session');
assert.equal(Boolean(romPath), Boolean(sessionPath), 'retail proof requires both --rom and --session');
const reportPath = resolve(args.get('--report') ?? `${root}/scratch/wasm-proof/result.json`);
const verifierTemporaryReport = dirname(reportPath) === resolve(tmpdir()) &&
  /^tje-wasm-proof-\d+-\d+\.json$/.test(basename(reportPath));
assert(reportPath.includes(`${sep}scratch${sep}`) || verifierTemporaryReport,
  'evidence must stay under scratch/ or use the verifier temporary-report protocol');
await mkdir(dirname(reportPath), { recursive: true });
const packageRoot = resolve(process.env.TJE_WASM_PROOF_DIR ?? `${root}/scratch/wasm-proof/pkg`);
const loaderPath = resolve(root, 'crates/tje-engine-wasmgen/src/browser.js');
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE
  ? pathToFileURL(resolve(process.env.PLAYWRIGHT_MODULE)).href : 'playwright');
const server = createServer(async (request, response) => {
  try {
    const pathname = decodeURIComponent(new URL(request.url, 'http://127.0.0.1').pathname);
    if (pathname === '/') {
      response.writeHead(200, { 'Content-Type': 'text/html', 'Cache-Control': 'no-store' });
      response.end('<!doctype html><title>Private compiled-engine proof</title>');
      return;
    }
    let path;
    if (pathname === '/rom' && romPath) path = resolve(romPath);
    else if (pathname === '/session' && sessionPath) path = resolve(sessionPath);
    else if (pathname === '/loader.js') path = loaderPath;
    else if (pathname.startsWith('/pkg/')) {
      path = resolve(packageRoot, pathname.slice(5));
      assert(path.startsWith(`${packageRoot}${sep}`), 'package path traversal');
    } else { response.writeHead(404); response.end(); return; }
    const type = path.endsWith('.wasm') ? 'application/wasm' : path.endsWith('.js') ? 'text/javascript' : 'application/octet-stream';
    response.writeHead(200, { 'Content-Type': type, 'Cache-Control': 'no-store' });
    response.end(await readFile(path));
  } catch (error) {
    response.writeHead(500); response.end(String(error));
  }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch({ headless: true });
const context = await browser.newContext();
const page = await context.newPage();
const errors = [];
const logs = [];
const requests = [];
page.on('pageerror', error => errors.push(String(error)));
page.on('console', message => logs.push(`${message.type()}: ${message.text()}`));
page.on('request', request => requests.push(request.url()));
let exitCode = 0;
try {
  await page.goto(origin);
  const cache = await page.evaluate(async () => {
    const loader = await import('/loader.js');
    const rom = '0'.repeat(40);
    const version = 'authored-cache-v2';
    const bytes = new Uint8Array([
      0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 127,
      3, 2, 1, 0, 7, 5, 1, 1, 102, 0, 0, 10, 6, 1, 4, 0, 65, 42, 11,
    ]);
    // Exercise the real V1-to-V2 database cutover before any V2 access.
    const legacy = await new Promise((resolve, reject) => {
      const request = indexedDB.open('tje-private-engine', 1);
      request.onupgradeneeded = () => request.result.createObjectStore('modules');
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    await new Promise((resolve, reject) => {
      const tx = legacy.transaction('modules', 'readwrite');
      tx.objectStore('modules').put(bytes, 'legacy-unverified-bytes');
      tx.oncomplete = resolve;
      tx.onabort = () => reject(tx.error);
    });
    legacy.close();
    let generations = 0;
    const generate = () => { generations++; return bytes; };
    const forbidden = () => { throw new Error('verified warm cache invoked generator'); };
    const first = await loader.openEngine(rom, version, 7, {}, generate);
    const second = await loader.openEngine(rom, version, 7, {}, forbidden);
    if (first.instance.exports.f() !== 42 || second.instance.exports.f() !== 42 ||
        first.metrics.cacheHit || !second.metrics.cacheHit || generations !== 1) throw new Error('cold/warm cache failed');
    async function mutate(mode) {
      const db = await new Promise((resolve, reject) => {
        const request = indexedDB.open('tje-private-engine', 2);
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      try {
        await new Promise((resolve, reject) => {
          const tx = db.transaction('modules', 'readwrite');
          const store = tx.objectStore('modules');
          const key = JSON.stringify([rom, version, 7]);
          const request = store.get(key);
          let failure;
          request.onsuccess = () => {
            try {
              const record = request.result;
              if (mode === 'semantic') {
                const offset = record.bytes.lastIndexOf(42);
                if (offset < 0) throw new Error('authored constant missing');
                record.bytes[offset] = 43;
                if (!WebAssembly.validate(record.bytes)) throw new Error('semantic mutation is not valid Wasm');
              } else if (mode === 'magic') record.bytes[0] ^= 0xff;
              else if (mode === 'identity') record.m68kDivider = 8;
              else record.digest = '0'.repeat(64);
              store.put(record, key);
            } catch (error) { failure = error; tx.abort(); }
          };
          tx.oncomplete = resolve;
          tx.onabort = () => reject(failure ?? tx.error);
        });
      } finally { db.close(); }
    }
    for (const mode of ['semantic', 'magic', 'identity', 'digest']) {
      await mutate(mode);
      const before = generations;
      const restored = await loader.openEngine(rom, version, 7, {}, generate);
      if (restored.instance.exports.f() !== 42 || restored.metrics.cacheHit || generations !== before + 1) {
        throw new Error(`cache ${mode} corruption was not regenerated exactly once`);
      }
      await loader.openEngine(rom, version, 7, {}, forbidden);
    }
    const beforeInvalid = generations;
    await loader.storeBytes(rom, version, 7, new Uint8Array(8));
    const restored = await loader.openEngine(rom, version, 7, {}, generate);
    if (restored.instance.exports.f() !== 42 || generations !== beforeInvalid + 1) throw new Error('hashed invalid module did not recover');
    const other = bytes.slice();
    other[other.lastIndexOf(42)] = 43;
    const divider8 = await loader.openEngine(rom, version, 8, {}, () => { generations++; return other; });
    const divider7Warm = await loader.openEngine(rom, version, 7, {}, forbidden);
    const divider8Warm = await loader.openEngine(rom, version, 8, {}, forbidden);
    if (divider8.metrics.cacheHit || divider7Warm.instance.exports.f() !== 42 ||
        divider8Warm.instance.exports.f() !== 43) throw new Error('divider cache identity collision');
    const changedVersion = await loader.openEngine(rom, 'authored-cache-v3', 7, {}, generate);
    const changedRom = await loader.openEngine('1'.repeat(40), 'authored-cache-v3', 7, {}, generate);
    if (changedVersion.metrics.cacheHit || changedRom.metrics.cacheHit ||
        await loader.cachedBytes(rom, version, 7) !== null ||
        await loader.cachedBytes(rom, version, 8) !== null) throw new Error('ROM/version isolation or eviction failed');
    let badBuilds = 0;
    try {
      await loader.openEngine('3'.repeat(40), version, 7, {}, () => { badBuilds++; return new Uint8Array(8); });
      throw new Error('fresh invalid compilation was hidden');
    } catch (error) {
      if (!(error instanceof WebAssembly.CompileError) || badBuilds !== 1) throw error;
    }
    const upgraded = await new Promise((resolve, reject) => {
      const request = indexedDB.open('tje-private-engine', 2);
      request.onsuccess = () => {
        const db = request.result;
        const tx = db.transaction('modules');
        const get = tx.objectStore('modules').get('legacy-unverified-bytes');
        get.onsuccess = () => resolve(get.result === undefined);
        tx.oncomplete = () => db.close();
      };
      request.onerror = () => reject(request.error);
    });
    if (!upgraded) throw new Error('legacy unverified records survived cutover');
    return { first: first.metrics, warm: second.metrics, versionIsolation: true, romIsolation: true,
      dividerIsolation: true, corruptionsRegeneratedOnce: true, hashedInvalidModuleRegeneratedOnce: true,
      freshErrorPropagatedOnce: true, oldVersionEvicted: true, legacyRecordsRemoved: true };
  });
  let primitives;
  let instructions;
  let cacheProof;
  let audio;
  let result;
  if (!args.has('--cache-only')) {
    await page.evaluate(async () => {
      const module = await import('/pkg/engine_proof.js');
      globalThis.proofExports = await module.default();
      globalThis.proofModule = module;
    });
    if (args.has('--primitives') || !romPath) {
      primitives = await page.evaluate(async () => JSON.parse(await globalThis.proofModule.browser_verify_primitives()));
      assert(primitives.cases > 0);
      instructions = await page.evaluate(async () => JSON.parse(await globalThis.proofModule.browser_verify_instructions()));
      assert(instructions.cases > 0);
      cacheProof = await page.evaluate(async () => JSON.parse(await globalThis.proofModule.browser_verify_cache()));
      audio = await page.evaluate(async () => JSON.parse(await globalThis.proofModule.browser_verify_audio()));
      assert(Array.isArray(audio) && audio.length === 4);
    }
    if (romPath) {
      result = await page.evaluate(async () => {
        const [rom, session] = await Promise.all([fetch('/rom').then(value => value.arrayBuffer()), fetch('/session').then(value => value.text())]);
        return JSON.parse(await globalThis.proofModule.browser_verify_session(new Uint8Array(rom), session));
      });
      await writeFile(reportPath, JSON.stringify(result.replay, null, 2) + '\n');
      const proof = result.replay.instruction_proof;
      const completeInstructionProof = proof?.initial_state_compared &&
        proof.scheduler_steps_compared === result.replay.scheduler_steps &&
        proof.instructions_compared === result.replay.coverage.instructions_executed;
      if (result.replay.divergence || result.replay.frames_compared !== result.replay.requested_frames ||
          !completeInstructionProof) exitCode = 1;
      assert.equal(result.replay.candidate_backend, 'wasm');
      assert.equal(result.warm.cache_hit, true);
      assert.equal(result.warm.generation_ms, 0);
    }
  }
  assert.deepEqual(errors, []);
  assert(requests.every(url => url.startsWith(origin)), 'proof must not send ROM bytes to external hosts');
  const appMemoryBytes = args.has('--cache-only') ? null
    : await page.evaluate(() => globalThis.proofExports.memory.buffer.byteLength);
  const metrics = { browser: browser.version(), cache, primitives, instructions, cacheProof, audio, appMemoryBytes, ...(result ?? {}), errors };
  const metricsPath = romPath ? `${reportPath}.metrics.json` : reportPath;
  await writeFile(metricsPath, JSON.stringify(metrics, null, 2) + '\n');
  console.log(JSON.stringify({ report: reportPath, metrics: metricsPath, passed: exitCode === 0,
    frames: result?.replay.frames_compared, generation: result?.cold, capacity_fps: result?.capacity_fps,
    primitives, instructions, cacheProof, audio, appMemoryBytes }, null, 2));
} finally {
  await writeFile(`${reportPath}.browser.log`, [...logs, ...errors].join('\n') + '\n');
  await context.close();
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
process.exitCode = exitCode;
