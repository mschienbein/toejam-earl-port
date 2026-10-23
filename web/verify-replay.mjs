// Build web/build-replay.sh and the native replay-reference example first.
// TJE_WEB_REPLAY_DIR=/absolute/scratch/pkg TJE_WEB_REFERENCE=/absolute/target/release/examples/replay-reference \
// TJE_WEB_ARTIFACT_DIR=/absolute/scratch/evidence node web/verify-replay.mjs PRIVATE_ROM CONTROLLER_SESSION
// This private localhost tool streams reference bytes into the browser. It is
// never packaged or imported by the static game, and contains no browser oracle.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, extname, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';

const [rom, sessionPath] = process.argv.slice(2).map(path => resolve(path));
assert(rom && sessionPath, 'supply private ROM and a full power-on controller recording');
const session = JSON.parse(await readFile(sessionPath, 'utf8'));
assert.equal(session.format, 1);
assert.equal(session.start.kind, 'power_on', 'RAM recipes are not controller-only frontend recordings');
assert.equal(session.runs.reduce((sum, run) => sum + run.frames, 0), session.frames);
assert(session.runs.every(run => run.frames > 0 && run.pads[0] & 256));
const pkg = resolve(process.env.TJE_WEB_REPLAY_DIR);
const referenceExecutable = resolve(process.env.TJE_WEB_REFERENCE);
const report = resolve(process.env.TJE_WEB_ARTIFACT_DIR, `${session.revision}-replay.json`);
assert(pkg.includes(`${sep}scratch${sep}`) && report.includes(`${sep}scratch${sep}`), 'private code/evidence must stay in scratch');
await mkdir(dirname(report), { recursive: true });
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(resolve(process.env.PLAYWRIGHT_MODULE)).href : 'playwright');
let reference;
let referenceFinished;
let referenceLog = '';
const csp = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'; object-src 'none'; base-uri 'none'";
const server = createServer(async (request, response) => {
  try {
    const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
    if (pathname === '/') {
      response.writeHead(200, { 'Content-Type': 'text/html', 'Content-Security-Policy': csp });
      response.end('<!doctype html><title>Private frontend byte replay</title><input id="rom" type="file"><canvas id="canvas" width="640" height="448"></canvas>');
    } else if (pathname === '/reference') {
      assert(!reference, 'one exact reference stream per recording');
      reference = spawn(referenceExecutable, [rom, sessionPath], { stdio: ['ignore', 'pipe', 'pipe'] });
      referenceFinished = new Promise((resolve, reject) => {
        reference.once('error', reject);
        reference.once('close', code => code === 0 ? resolve() : reject(new Error(`reference exporter exited ${code}: ${referenceLog}`)));
      });
      // Observe rejection now, while the streaming reader supplies context later.
      referenceFinished.catch(() => {});
      reference.stderr.on('data', data => { referenceLog += data; });
      response.writeHead(200, { 'Content-Type': 'application/octet-stream', 'Cache-Control': 'no-store' });
      reference.stdout.pipe(response);
    } else if (pathname.startsWith('/pkg/')) {
      const path = resolve(pkg, pathname.slice(5));
      assert(path.startsWith(`${pkg}${sep}`));
      assert(['.js', '.wasm'].includes(extname(path)));
      response.writeHead(200, { 'Content-Type': path.endsWith('.wasm') ? 'application/wasm' : 'text/javascript', 'Cache-Control': 'no-store' });
      response.end(await readFile(path));
    } else response.writeHead(404).end();
  } catch (error) { response.writeHead(500).end(String(error)); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const browser = await chromium.launch({ headless: true, args: [
  '--enable-unsafe-webgpu', '--enable-unsafe-swiftshader', '--use-angle=swiftshader',
  '--enable-features=Vulkan', '--use-vulkan=swiftshader', '--disable-vulkan-surface',
] });
const page = await browser.newPage();
const errors = [];
page.on('pageerror', error => errors.push(String(error)));
try {
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  await page.locator('#rom').setInputFiles(rom);
  const result = await page.evaluate(async recording => {
    const { default: init, CompiledCartridge, WebGame } = await import('/pkg/tje_web.js');
    await init();
    const bytes = new Uint8Array(await document.getElementById('rom').files[0].arrayBuffer());
    const coldStages = [];
    let cartridge = await CompiledCartridge.load(bytes, 48000, stage => coldStages.push(stage));
    const game = await WebGame.create(document.getElementById('canvas'), 'webgl');
    const revision = game.install(cartridge);
    if (typeof game.observation !== 'function') throw new Error('build the observation-only package, not the release package');
    const cold = game.compilation();
    const stream = await fetch('/reference');
    if (!stream.ok) throw new Error(`reference stream: HTTP ${stream.status}`);
    const reader = stream.body.getReader();
    let chunk = new Uint8Array();
    let offset = 0;
    async function take(length) {
      const bytes = new Uint8Array(length);
      let written = 0;
      while (written < length) {
        if (offset === chunk.length) {
          const next = await reader.read();
          if (next.done) throw new Error('reference ended before every requested frame');
          chunk = next.value;
          offset = 0;
        }
        const count = Math.min(length - written, chunk.length - offset);
        bytes.set(chunk.subarray(offset, offset + count), written);
        written += count;
        offset += count;
      }
      return bytes;
    }
    let comparedBytes = 0;
    async function compare(frame) {
      const header = await take(4);
      const length = new DataView(header.buffer).getUint32(0, true);
      if (length > 8 * 1024 * 1024) throw new Error('invalid reference frame length');
      const expected = await take(length);
      const actual = game.observation();
      if (actual.length !== expected.length) throw new Error(`frame ${frame}: byte count ${actual.length} != ${expected.length}`);
      for (let index = 0; index < length; index++) {
        if (actual[index] !== expected[index]) throw new Error(`frame ${frame}: first unequal canonical observation byte ${index}`);
      }
      comparedBytes += length;
    }
    const started = performance.now();
    await compare(0);
    let frames = 0;
    for (const run of recording.runs) {
      for (let tick = 0; tick < run.frames; tick++) {
        game.step(run.pads[0] & 255, run.pads[1] & 255, !!(run.pads[1] & 256));
        await compare(++frames);
      }
    }
    if (offset !== chunk.length || !(await reader.read()).done) throw new Error('reference contained trailing frames');
    const elapsedMs = performance.now() - started;
    const warmStages = [];
    cartridge = await CompiledCartridge.load(bytes, 48000, stage => warmStages.push(stage));
    game.install(cartridge);
    const warm = game.compilation();
    if (!warm.cacheHit || warm.generationMs !== 0 || warmStages.includes('generate')) throw new Error('warm cache invoked IR generation');
    game.free();
    return { revision, framesCompared: frames, requestedFrames: recording.frames, initialStateCompared: true,
      comparedBytes, comparison: 'actual serialized canonical FrameState bytes, not hashes', elapsedMs,
      domains: ['position', 'CPU', 'work RAM', 'VRAM', 'CRAM', 'VSRAM', 'VDP registers', 'complete VDP state', 'Z80 RAM', 'dimensions', 'full RGBA backing buffer', 'exact f64 audio bits/count', 'ordered timestamped YM2612/PSG writes with origin/PC'],
      cold, warm, coldStages, warmStages, crossOriginIsolated,
    };
  }, session);
  await referenceFinished;
  assert.equal(result.framesCompared, session.frames);
  assert.deepEqual(errors, []);
  const receipt = { ...result, session: sessionPath, reference: JSON.parse(referenceLog.trim()), browser: browser.version(), pageErrors: errors };
  await writeFile(report, JSON.stringify(receipt, null, 2));
  console.log(JSON.stringify(receipt, null, 2));
} catch (error) {
  await writeFile(`${report}.failure.log`, `${error.stack}\n${referenceLog}\n${errors.join('\n')}`);
  throw error;
} finally {
  if (reference && reference.exitCode === null) reference.kill();
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
