// Build debug Wasm into web/pkg first. This uses the real page/engine, actual
// wasm panic, Worker ErrorEvent and AudioWorklet processor failure, not mocks.
// Usage: TJE_WEB_ARTIFACT_DIR=/private/scratch/reports PLAYWRIGHT_MODULE=... node web/verify-reports.mjs /private/rev02.bin
import assert from 'node:assert/strict';
import { mkdir, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { startServer } from './serve.mjs';

const rom = resolve(process.argv[2] ?? process.env.TJE_REV02_ROM ?? '');
assert(process.argv[2] || process.env.TJE_REV02_ROM, 'supply a private verified ROM');
const artifacts = resolve(process.env.TJE_WEB_ARTIFACT_DIR ?? '../scratch/reports');
assert(artifacts.includes('/scratch/'), 'keep all captures/reports private under scratch');
await mkdir(artifacts, { recursive: true });
const image = await readFile(rom);
const romHash = createHash('sha1').update(image).digest('hex');
const windows = new Set();
for (let offset = 0; offset <= image.length - 32; offset++) windows.add(image.subarray(offset, offset + 32).toString('hex'));
function noRomBytes(bytes) {
  for (let offset = 0; offset <= bytes.length - 32; offset++) {
    assert(!windows.has(bytes.subarray(offset, offset + 32).toString('hex')), `raw ROM fragment at report byte ${offset}`);
  }
}
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(resolve(process.env.PLAYWRIGHT_MODULE)).href : 'playwright');
const server = await startServer(0);
const origin = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch({ headless: true, args: [
  '--enable-unsafe-swiftshader', '--use-angle=swiftshader', '--enable-features=Vulkan', '--use-vulkan=swiftshader', '--disable-vulkan-surface',
] });
const receipts = [];
let page;
let workletServer;
const workletFixtureRoot = resolve(artifacts, 'worklet-fixture-site');
try {
  const context = await browser.newContext({ viewport: { width: 1100, height: 1300 }, acceptDownloads: true });
  page = await context.newPage();
  const requests = [];
  const errors = [];
  page.on('request', request => requests.push({ method: request.method(), url: request.url() }));
  page.on('pageerror', error => errors.push(String(error)));
  async function ready(query = '', base = origin) {
    await page.goto(`${base}/?backend=webgl${query}`);
    await page.waitForSelector('body[data-state="empty"],body[data-state="ready"]', { timeout: 90000 });
    if (await page.getAttribute('body', 'data-state') === 'empty') {
      await page.locator('#rom').setInputFiles(rom);
      await page.waitForSelector('body[data-state="ready"]', { timeout: 90000 });
    }
  }
  async function pending() {
    return page.evaluate(async () => (await import('./reporting.js')).pendingReport());
  }
  async function savedCrash(source, message) {
    // waitForFunction predicates must be synchronous: a Promise itself is
    // truthy and does not poll the eventual IndexedDB result.
    await page.waitForFunction(message =>
      !document.getElementById('report-offer').hidden
      && document.getElementById('report-summary').textContent.includes(message)
      && document.getElementById('report-status').textContent.startsWith('Report saved'), message);
    const bundle = await pending();
    assert.equal(bundle?.failure.source, source);
    return bundle;
  }
  async function download(label) {
    const [item] = await Promise.all([
      page.waitForEvent('download'), page.click('#report-download'),
    ]);
    const path = `${artifacts}/${label}.tje-report.json`;
    await item.saveAs(path);
    const bytes = await readFile(path);
    noRomBytes(bytes);
    const bundle = JSON.parse(bytes);
    assert.equal(bundle.rom.sha1, romHash);
    assert.match(bundle.build.commit, /^[0-9a-f]{40}$/);
    assert.equal(bundle.recording.first_frame, 0);
    assert.equal(bundle.recording.session.frames, bundle.recording.completed_frames + Number(bundle.recording.in_flight !== null));
    assert.equal(bundle.recording.session.runs.reduce((sum, run) => sum + run.frames, 0), bundle.recording.session.frames);
    if (bundle.screenshot) {
      assert.equal(bundle.screenshot.consent, true);
      const png = Buffer.from(bundle.screenshot.png);
      noRomBytes(png);
      assert.equal(png.subarray(0, 8).toString('hex'), '89504e470d0a1a0a');
      await writeFile(`${artifacts}/${label}-game.png`, png);
    }
    return bundle;
  }
  await ready('&debug-crash-frame=240');
  assert.equal(await page.evaluate(() => typeof window.tje.debugCrash), 'function', 'this lane requires the debug-only Wasm trigger');
  assert.equal(await page.isChecked('#report-screenshot'), false, 'screenshot collection defaults off');
  await page.check('#report-screenshot'); // Actual UI consent before any pixels are cached.
  await page.click('#play');
  await page.waitForSelector('#report-offer:not([hidden])', { timeout: 30000 });
  await savedCrash('wasm_panic', 'Forced browser playtest crash');
  const forced = await download('web-forced-crash');
  assert.equal(forced.failure.source, 'wasm_panic');
  assert.match(forced.failure.message, /Forced browser playtest crash/);
  assert.equal(forced.engine.frame, 240);
  assert.equal(forced.recording.session.frames, 240);
  assert(forced.screenshot && forced.screenshot.frame <= 240);
  await page.evaluate(async () => (await import('./reporting.js')).dismissPending('obsolete-offer-id'));
  assert.equal((await pending()).id, forced.id, 'dismissing an old report must not delete a newer pending crash');
  assert.equal((await page.evaluate(() => window.tje.metrics)).crashed, true, 'observability must not reenter a trapped WebGame');
  await page.locator('#report-offer').scrollIntoViewIfNeeded();
  await page.screenshot({ path: `${artifacts}/web-crash-offer.png`, fullPage: true });
  await ready();
  await page.locator('#report-offer:not([hidden])').waitFor();
  assert.equal((await pending()).id, forced.id, 'the next launch must retain and offer the saved crash');
  await page.screenshot({ path: `${artifacts}/web-next-launch-offer.png`, fullPage: true });
  assert(requests.every(request => request.method === 'GET' && request.url.startsWith(origin)), 'no report submission request may happen without a click');
  // Click a real issue action. Intercept the external navigation so this test
  // needs neither a GitHub account nor an external request; record its exact URL.
  let issue;
  await context.route('https://github.com/**', route => { issue = route.request().url(); return route.fulfill({ contentType: 'text/plain', body: 'User-clicked GitHub issue preview intercepted by acceptance test.' }); });
  const popup = page.waitForEvent('popup');
  await page.click('#report-issue');
  const opened = await popup;
  await opened.waitForLoadState();
  assert(issue?.startsWith('https://github.com/mschienbein/toejam-earl-port/issues/new?'));
  assert.match(new URL(issue).searchParams.get('body'), /Never attach your ROM/);
  await opened.close();
  await page.click('#report-dismiss');
  await page.uncheck('#report-screenshot');
  await page.click('#play');
  await page.waitForFunction(() => window.tje.metrics.frames >= 30);
  await page.click('#report-bug');
  await page.fill('#report-note', 'Actual Chromium playtest note: expected the character to move; UI report capture acceptance.');
  await page.click('#report-save');
  await page.waitForFunction(() => document.getElementById('report-status').textContent.startsWith('Report saved'));
  const manual = await download('web-manual-bug');
  assert.equal(manual.kind, 'bug');
  assert.match(manual.note, /Actual Chromium playtest note/);
  assert.equal(manual.screenshot, null, 'turning consent off must remove collected pixels');
  await page.screenshot({ path: `${artifacts}/web-manual-report.png`, fullPage: true });
  // Reset starts a fresh power-on recording, not a suffix of the prior play.
  await page.click('#report-dismiss');
  await page.click('#reset');
  await page.evaluate(() => { void Promise.reject(new Error('Forced actual unhandled rejection')); });
  const rejection = await savedCrash('unhandledrejection', 'Forced actual unhandled rejection');
  assert.equal(rejection.recording.completed_frames, 0);
  assert.equal(rejection.recording.session.frames, 0);
  await ready();
  await page.click('#report-dismiss');
  await page.evaluate(() => { setTimeout(() => { throw new Error('Forced actual window error'); }, 0); });
  await savedCrash('window.onerror', 'Forced actual window error');
  await ready();
  await page.click('#report-dismiss');
  await page.route('**/report-worker-fixture.js', route => route.fulfill({ contentType: 'text/javascript', body: 'throw new Error("Forced actual worker error");' }));
  await page.evaluate(() => { new Worker('./report-worker-fixture.js'); });
  await savedCrash('worker_error', 'Forced actual worker error');
  // Chromium does not expose AudioWorklet's addModule fetch to Playwright
  // routing. Serve an otherwise identical private tree with a throwing process
  // method, so the real processorerror path is exercised without a product shim.
  await rm(workletFixtureRoot, { recursive: true, force: true });
  await mkdir(workletFixtureRoot, { recursive: true });
  const webRoot = fileURLToPath(new URL('.', import.meta.url));
  for (const entry of await readdir(webRoot)) {
    if (entry !== 'audio-worklet.js') {
      await symlink(resolve(webRoot, entry), resolve(workletFixtureRoot, entry));
    }
  }
  const originalWorklet = await readFile(new URL('./audio-worklet.js', import.meta.url), 'utf8');
  const throwingWorklet = originalWorklet.replace('process(_inputs, outputs) {', 'process(_inputs, outputs) { throw new Error("Forced actual worklet processor error");');
  assert.notEqual(throwingWorklet, originalWorklet);
  await writeFile(resolve(workletFixtureRoot, 'audio-worklet.js'), throwingWorklet);
  workletServer = await startServer(0, workletFixtureRoot);
  await ready('', `http://127.0.0.1:${workletServer.address().port}`);
  await page.click('#play');
  await savedCrash('audio_worklet', 'AudioWorklet processor failed.');
  const worklet = await download('web-worklet-crash');
  receipts.push({ browser: browser.version(), forced: { id: forced.id, frame: forced.engine.frame, screenshotFrame: forced.screenshot.frame, recordingFrames: forced.recording.session.frames }, manual: { frame: manual.engine.frame, note: manual.note, screenshot: manual.screenshot }, resetRecordingFrames: rejection.recording.session.frames, sourcesExercised: ['wasm_panic', 'unhandledrejection', 'window.onerror', 'worker_error', 'audio_worklet'], worklet: worklet.failure, rawRomWindowScan: 'all exact 32-byte ROM windows absent from report JSON and PNGs', issueOpenedOnlyOnClick: true, issueNavigationIntercepted: true, requests, expectedForcedPageErrors: errors });
  await context.close();
} catch (error) {
  if (page && !page.isClosed()) {
    await page.screenshot({ path: `${artifacts}/web-report-failure.png`, fullPage: true }).catch(() => {});
    const diagnostic = await page.evaluate(async () => ({
      status: document.getElementById('report-status').textContent,
      offerHidden: document.getElementById('report-offer').hidden,
      summary: document.getElementById('report-summary').textContent,
      pending: await (await import('./reporting.js')).pendingReport(),
    })).catch(failure => ({ diagnosticError: String(failure) }));
    await writeFile(`${artifacts}/web-report-failure.json`, JSON.stringify(diagnostic, null, 2));
  }
  throw error;
} finally {
  await writeFile(`${artifacts}/web-report-evidence.json`, JSON.stringify(receipts, null, 2));
  await browser.close();
  if (workletServer) await new Promise(resolve => workletServer.close(resolve));
  await rm(workletFixtureRoot, { recursive: true, force: true });
  await new Promise(resolve => server.close(resolve));
}
console.log(JSON.stringify(receipts, null, 2));
