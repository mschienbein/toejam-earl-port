// Usage: TJE_WEB_ARTIFACT_DIR=/absolute/scratch/evidence node web/verify.mjs /private/rev02.bin [auto|webgpu|webgl]
// npm install --prefix web, then npx --prefix web playwright install chromium.
// PLAYWRIGHT_MODULE may point to an existing user-local playwright/index.mjs.
import assert from 'node:assert/strict';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { startServer } from './serve.mjs';
import { installUxFixtures, verifyEmptyUx, verifyLoadedUx, verifyRestoredUx } from './verify-ux.mjs';

const rom = process.argv[2] ?? process.env.TJE_REV02_ROM;
assert(rom, 'Supply a private ROM path; no ROM is shipped with the frontend.');
const backend = process.argv[3] ?? 'auto';
assert(['auto', 'webgpu', 'webgl'].includes(backend));
const artifacts = resolve(process.env.TJE_WEB_ARTIFACT_DIR ?? fileURLToPath(new URL('../scratch/web-verification/', import.meta.url)));
assert(artifacts.includes('/scratch/'), 'Screenshots and measurements must stay under scratch/.');
await mkdir(artifacts, { recursive: true });
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE
  ? pathToFileURL(resolve(process.env.PLAYWRIGHT_MODULE)).href : 'playwright');
const server = await startServer(0);
const origin = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch({ headless: process.env.TJE_WEB_HEADED !== '1', args: [
  '--enable-unsafe-webgpu', '--enable-unsafe-swiftshader', '--use-angle=swiftshader',
  '--enable-features=Vulkan', '--use-vulkan=swiftshader', '--disable-vulkan-surface',
] });
const context = await browser.newContext({ viewport: { width: 1000, height: 1100 } });
const disableWebgpu = process.env.TJE_WEB_DISABLE_WEBGPU === '1';
if (disableWebgpu) {
  await context.addInitScript(() => Object.defineProperty(navigator, 'gpu', { value: undefined }));
}
await installUxFixtures(context);
const page = await context.newPage();
const errors = [];
const log = [];
const requests = [];
page.on('pageerror', error => errors.push(String(error)));
page.on('console', message => log.push(`${message.type()}: ${message.text()}`));
page.on('request', request => requests.push({ method: request.method(), url: request.url() }));
const metrics = () => page.evaluate(() => window.tje.metrics);
const frameAt = frame => page.waitForFunction(frame => window.tje?.metrics.frames >= frame, frame, { timeout: 30000 });
async function hold(key, start, duration = 8) {
  await frameAt(start);
  await page.keyboard.down(key);
  const initial = (await metrics()).frames;
  await frameAt(initial + duration);
  await page.keyboard.up(key);
}

let result;
try {
  const response = await page.goto(`${origin}/?backend=${backend}`);
  assert.equal(response.headers()['cross-origin-opener-policy'], undefined);
  assert.equal(response.headers()['cross-origin-embedder-policy'], undefined);
  await page.waitForSelector('body[data-state="empty"]', { timeout: 45000 });
  assert.equal((await metrics()).crossOriginIsolated, false);
  await verifyEmptyUx(page, metrics);
  // Exercise the real DOM file picker, not a direct Rust import/testing bypass.
  await page.locator('#rom').setInputFiles(resolve(rom));
  await page.waitForSelector('body[data-state="ready"]', { timeout: 15000 });
  assert.equal((await metrics()).source, 'file');
  await page.click('#play');
  await page.waitForSelector('body[data-state="running"]');
  await hold('Enter', 300);
  await frameAt(550);
  assert.notEqual((await metrics()).titleFrame, null, 'title routine must actually execute');
  await page.screenshot({ path: `${artifacts}/${backend}-title.png`, fullPage: true });
  await hold('Enter', 600);
  await hold('Enter', 900);
  await hold('ArrowDown', 1000);
  await hold('Enter', 1100);
  await hold('Enter', 1200);
  await page.waitForFunction(() => window.tje.metrics.gameFrame !== null, null, { timeout: 30000 });
  await hold('ArrowRight', 1450, 24);
  // Both keyboard wires are exercised, including the persisted P2 connection.
  await page.check('#p2');
  await page.locator('#canvas').focus();
  await page.keyboard.down('KeyW');
  await frameAt(1500);
  const p2 = await metrics();
  assert.equal(p2.lastInputs[1] & 1, 1);
  assert.equal(p2.lastInputs[2], true);
  await page.keyboard.up('KeyW');
  await page.locator('#volume').evaluate(element => { element.value = '45'; });
  await page.locator('#volume').dispatchEvent('input');
  await page.selectOption('#scale', 'fit');
  await frameAt(1800);
  result = await metrics();
  assert.equal(result.mode, 'replace');
  if (backend !== 'auto') assert.equal(result.backend, backend);
  if (disableWebgpu) assert.equal(result.backend, 'webgl', 'automatic WebGL2 fallback');
  assert.equal(result.audio.state, 'running');
  assert(result.audio.received > 48000, 'worklet must receive samples');
  assert(result.audio.played > 48000, 'worklet must consume samples');
  assert(result.audio.nonzero > 1000, 'real cartridge sound must be non-silent');
  assert(result.audio.recycledBlocks > 100, 'transferred sample buffers must be recycled');
  assert(result.presentationFps >= result.targetFps * 0.95, `video pacing is not full speed: ${result.presentationFps} fps`);
  assert(result.actualFps >= result.targetFps * 0.97, `not full speed: ${result.actualFps} fps`);
  assert(result.capacityFps > result.targetFps, 'emulation must have real-time headroom');
  assert(result.keyEvents[0] >= 7 && result.keyEvents[1] >= 1);
  await page.screenshot({ path: `${artifacts}/${backend}-gameplay.png`, fullPage: true });
  await page.click('#play');
  await page.waitForSelector('body[data-state="paused"]');
  const ux = await verifyLoadedUx({ page, context, metrics, rom: resolve(rom), artifacts, backend });
  await page.reload();
  await page.waitForSelector('body[data-state="ready"]', { timeout: 30000 });
  const restored = await metrics();
  assert.equal(restored.source, 'stored');
  assert.equal(restored.revision, result.revision);
  assert.equal(restored.frames, 0);
  assert.equal(restored.settings.volume, 0.45);
  assert.equal(restored.settings.scale, 'fit');
  assert.equal(restored.settings.p2, true);
  assert.equal(await page.locator('#p2').isChecked(), true);
  assert.equal(await page.locator('#scale').inputValue(), 'fit');
  await verifyRestoredUx(page, metrics);
  await page.click('#play');
  await frameAt(120);
  await page.waitForFunction(() => window.tje.metrics.audio.played > 1000);
  const reloaded = await metrics();
  assert.equal(reloaded.audio.state, 'running');
  assert.equal(reloaded.crossOriginIsolated, false);
  await page.click('#reset');
  await page.waitForSelector('body[data-state="ready"]');
  assert.equal((await metrics()).frames, 0, 'hard reset must reset the console frame count');
  await page.click('#forget');
  await page.waitForSelector('body[data-state="empty"]', { timeout: 30000 });
  assert.equal((await metrics()).source, '', 'forget must remove the stored and in-memory ROM');
  const transfer = await page.evaluateHandle(bytes => {
    const data = new DataTransfer();
    data.items.add(new File([new Uint8Array(bytes)], 'dropped-cartridge.bin'));
    return data;
  }, Array.from(await readFile(resolve(rom))));
  await page.dispatchEvent('#drop-zone', 'drop', { dataTransfer: transfer });
  await transfer.dispose();
  await page.waitForSelector('body[data-state="ready"]', { timeout: 15000 });
  assert.equal((await metrics()).source, 'file', 'drag-and-drop must use the real import listener');
  assert.deepEqual(errors, []);
  assert(requests.every(request => request.method === 'GET' && request.url.startsWith(origin)), 'ROM/network privacy: only local static GETs are allowed');
  result = { run: result, reload: reloaded, ux, reset: true, forget: true, dragDrop: true, browser: browser.version(), headed: process.env.TJE_WEB_HEADED === '1', disableWebgpu, errors, requests };
  await writeFile(`${artifacts}/${backend}-result.json`, JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));
} catch (error) {
  log.push(`FAIL ${error.stack}`);
  try {
    log.push(`STATE ${JSON.stringify(await metrics())}`);
    await page.screenshot({ path: `${artifacts}/${backend}-failed.png`, fullPage: true });
  } catch { /* Preserve the original test failure if the browser itself died. */ }
  throw error;
} finally {
  await writeFile(`${artifacts}/${backend}-browser.log`, [...log, ...errors].join('\n') + '\n');
  await context.close();
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
