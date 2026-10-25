// Usage: TJE_WEB_ARTIFACT_DIR=/private/scratch/shaders node web/verify-shaders.mjs /private/rev02.bin [webgl|webgpu]
// Builds use web/build.sh. PLAYWRIGHT_MODULE can select an existing user-local installation.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { startServer } from './serve.mjs';

const rom = process.argv[2] ?? process.env.TJE_REV02_ROM;
assert(rom, 'Supply your private cartridge path.');
const backend = process.argv[3] ?? 'webgl';
assert(['webgl', 'webgpu'].includes(backend));
const artifacts = resolve(process.env.TJE_WEB_ARTIFACT_DIR ?? fileURLToPath(new URL('../scratch/shader-verification/', import.meta.url)));
assert(artifacts.includes('/scratch/'), 'Game screenshots must remain private under scratch/.');
await mkdir(artifacts, { recursive: true });
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(resolve(process.env.PLAYWRIGHT_MODULE)).href : 'playwright');
const server = await startServer(0);
const origin = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch({ headless: true, args: [
  '--enable-unsafe-webgpu', '--enable-unsafe-swiftshader', '--use-angle=swiftshader',
  '--enable-features=Vulkan', '--use-vulkan=swiftshader', '--disable-vulkan-surface',
] });
const context = await browser.newContext({ viewport: { width: 1057, height: 1000 } });
const page = await context.newPage();
const errors = [];
const log = [];
page.on('pageerror', error => errors.push(String(error)));
page.on('console', message => log.push(`${message.type()}: ${message.text()}`));
const metrics = () => page.evaluate(() => window.tje.metrics);
const settle = () => page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const shots = [];
const performance = [];
try {
  await page.goto(`${origin}/?backend=${backend}`);
  await page.waitForSelector('body[data-state="empty"]', { timeout: 45000 });
  assert.equal((await metrics()).settings.displayShader, 'off');
  await page.locator('#rom').setInputFiles(resolve(rom));
  await page.waitForSelector('body[data-state="ready"]', { timeout: 120000 });
  await page.click('#play');
  await page.waitForFunction(() => window.tje.metrics.frames >= 520, null, { timeout: 60000 });
  await page.click('#play');
  await page.waitForSelector('body[data-state="paused"]');
  const frozenFrame = (await metrics()).frames;
  for (const pixelShape of ['native', 'square']) {
    for (const scale of ['integer', 'fit']) {
      await page.selectOption('#pixel-shape', pixelShape);
      await page.selectOption('#scale', scale);
      await page.selectOption('#display-shader', 'off');
      await settle();
      const before = await page.locator('#canvas').screenshot();
      const prefix = `${backend}-${pixelShape}-${scale}`;
      await writeFile(`${artifacts}/${prefix}-before-off.png`, before);
      for (const preset of ['smooth', 'crt', 'crt-curved']) {
        await page.selectOption('#display-shader', preset);
        await settle();
        const image = await page.locator('#canvas').screenshot();
        assert(!image.equals(before), `${preset} must change rendered pixels`);
        const current = await metrics();
        assert.equal(current.frames, frozenFrame, 'shader changes must never step the engine');
        assert.equal(current.settings.pixelShape, pixelShape);
        assert.equal(current.settings.scale, scale);
        assert.equal(current.settings.filter, 'sharp', 'preset must not overwrite the saved filter');
        const name = `${prefix}-${preset}.png`;
        await writeFile(`${artifacts}/${name}`, image);
        shots.push({ name, preset, pixelShape, scale, sha256: digest(image) });
      }
      await page.selectOption('#display-shader', 'off');
      await settle();
      const after = await page.locator('#canvas').screenshot();
      assert(after.equals(before), 'turning effects Off must restore every display pixel');
      await writeFile(`${artifacts}/${prefix}-after-off.png`, after);
      shots.push({ name: `${prefix}-after-off.png`, preset: 'off', pixelShape, scale, beforeSha256: digest(before), afterSha256: digest(after), pixelIdentical: true });
    }
  }
  // Legacy filters remain live under CRT, and selecting Smooth does not destroy
  // the saved nearest choice when effects are subsequently disabled.
  await page.selectOption('#display-shader', 'crt');
  const filters = [];
  for (const filter of ['sharp', 'nearest', 'linear']) {
    await page.selectOption('#filter', filter);
    await settle();
    filters.push(digest(await page.locator('#canvas').screenshot()));
  }
  assert.equal(new Set(filters).size, 3, 'CRT retains all legacy filter choices');
  await page.selectOption('#filter', 'nearest');
  await page.selectOption('#display-shader', 'smooth');
  assert.equal((await metrics()).settings.filter, 'nearest');
  await page.selectOption('#display-shader', 'off');
  assert.equal((await metrics()).settings.filter, 'nearest');
  await page.selectOption('#filter', 'sharp');
  await page.selectOption('#pixel-shape', 'native');
  await page.selectOption('#scale', 'integer');
  for (const preset of ['off', 'smooth', 'crt', 'crt-curved']) {
    await page.selectOption('#display-shader', preset);
    await page.click('#play');
    await page.waitForSelector('body[data-state="running"]');
    const start = await metrics();
    await page.waitForFunction(frame => window.tje.metrics.frames >= frame + 180, start.frames, { timeout: 60000 });
    const end = await metrics();
    await page.click('#play');
    await page.waitForSelector('body[data-state="paused"]');
    const frames = end.measuredFrames - start.measuredFrames;
    const seconds = end.wallSeconds - start.wallSeconds;
    performance.push({ preset, frames, seconds, actualFps: frames / seconds,
      emulationCapacityFps: frames * 1000 / (end.emulationMs - start.emulationMs), canvas: end.canvas });
  }
  await page.selectOption('#display-shader', 'crt-curved');
  await page.reload();
  await page.waitForSelector('body[data-state="ready"]', { timeout: 120000 });
  assert.equal((await metrics()).settings.displayShader, 'crt-curved', 'opt-in preset must persist');
  await page.selectOption('#display-shader', 'off');
  await page.reload();
  await page.waitForSelector('body[data-state="ready"]', { timeout: 120000 });
  assert.equal((await metrics()).settings.displayShader, 'off', 'Off must persist');
  assert.deepEqual(errors, []);
  const result = { backend, browser: browser.version(), adapter: (await metrics()).adapter, shots, filters, performance, errors,
    performanceMethod: '180 real emulated frames per preset through the shipping frontend; includes engine, audio and presentation, vsync-limited; not isolated GPU timing' };
  await writeFile(`${artifacts}/${backend}-shaders.json`, JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));
} finally {
  await writeFile(`${artifacts}/${backend}-browser.log`, [...log, ...errors].join('\n'));
  await context.close();
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
