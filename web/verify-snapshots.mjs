// Private localhost proof. Build web/build-replay.sh and run snapshot_sessions
// first. TJE_WEB_REPLAY_DIR=/private/scratch/pkg TJE_WEB_ARTIFACT_DIR=/private/scratch/evidence
// node web/verify-snapshots.mjs PRIVATE_ROM NATIVE_SNAPSHOT_RECEIPT
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, extname, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';

const [romPath, nativePath] = process.argv.slice(2).map(path => resolve(path));
assert(romPath && nativePath, 'supply private ROM and native snapshot_sessions receipt');
const native = JSON.parse(await readFile(nativePath, 'utf8'));
const sessions = await Promise.all(native.sessions.map(async report => ({ ...report, recording: JSON.parse(await readFile(report.session, 'utf8')) })));
const pkg = resolve(process.env.TJE_WEB_REPLAY_DIR);
const reportPath = resolve(process.env.TJE_WEB_ARTIFACT_DIR, `${sessions[0].recording.revision}-snapshots-browser.json`);
assert(pkg.includes(`${sep}scratch${sep}`) && reportPath.includes(`${sep}scratch${sep}`), 'private generated code/evidence must stay in scratch');
await mkdir(dirname(reportPath), { recursive: true });
const root = resolve(dirname(new URL(import.meta.url).pathname));
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(resolve(process.env.PLAYWRIGHT_MODULE)).href : 'playwright');
const server = createServer(async (request, response) => {
  try {
    const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
    if (pathname === '/') {
      response.writeHead(200, { 'Content-Type': 'text/html', 'Content-Security-Policy': "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'; object-src 'none'; base-uri 'none'" });
      response.end('<!doctype html><title>Private snapshot proof</title><input id="rom" type="file"><canvas id="canvas"></canvas>');
      return;
    }
    const relative = pathname.startsWith('/app/') ? pathname.slice(5) || 'index.html' : pathname.slice(1);
    const path = relative.startsWith('pkg/') ? resolve(pkg, relative.slice(4)) : resolve(root, relative);
    assert(path.startsWith(`${root}${sep}`) || path.startsWith(`${pkg}${sep}`));
    const type = { '.js': 'text/javascript', '.mjs': 'text/javascript', '.wasm': 'application/wasm', '.html': 'text/html', '.css': 'text/css' }[extname(path)];
    assert(type);
    response.writeHead(200, { 'Content-Type': type, 'Cache-Control': 'no-store' });
    response.end(await readFile(path));
  } catch (error) { response.writeHead(500).end(String(error)); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const browser = await chromium.launch({ headless: true, args: ['--enable-unsafe-webgpu', '--enable-unsafe-swiftshader', '--use-angle=swiftshader'] });
const page = await browser.newPage();
const errors = [];
page.on('pageerror', error => errors.push(String(error)));
try {
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  await page.locator('#rom').setInputFiles(romPath);
  const proof = await page.evaluate(async sessions => {
    const { default: init, CompiledCartridge, WebGame } = await import('/pkg/tje_web.js');
    const { stateSlot } = await import('/convenience.js');
    await init();
    const rom = new Uint8Array(await document.getElementById('rom').files[0].arrayBuffer());
    const cartridge = await CompiledCartridge.load(rom, 48000, () => {});
    const game = await WebGame.create(document.getElementById('canvas'), 'webgl');
    game.install(cartridge);
    const identity = JSON.parse(game.report_identity());
    const hex = bytes => Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
    const hash = () => hex(game.state_hash());
    const equal = (actual, expected, message) => { if (actual !== expected) throw new Error(`${message}: ${actual} != ${expected}`); };
    const check = (value, message) => { if (!value) throw new Error(message); };
    const reports = [];
    for (const session of sessions) {
      const recording = session.recording;
      check(recording.start.kind === 'power_on', 'browser snapshot proof uses full power-on sessions');
      const pads = recording.runs.flatMap(run => Array.from({ length: run.frames }, () => run.pads));
      const step = frame => { const [p1, p2] = pads[frame]; game.step(p1 & 255, p2 & 255, !!(p2 & 256)); };
      game.set_rewind(false);
      game.reset();
      const saved = [];
      for (let frame = 0; frame < pads.length; frame++) {
        step(frame);
        equal(hash(), session.frame_hashes[frame][0], `native/browser complete-state hash ${recording.revision} frame ${frame + 1}`);
        if (session.checkpoints.includes(frame + 1)) saved.push({ frame: frame + 1, bytes: game.snapshot() });
      }
      let compared = 0;
      for (const checkpoint of saved) {
        game.restore(checkpoint.bytes);
        equal(hash(), session.frame_hashes[checkpoint.frame - 1][0], 'immediate restored state');
        for (let frame = checkpoint.frame; frame < Math.min(pads.length, checkpoint.frame + 120); frame++) {
          step(frame);
          equal(hash(), session.frame_hashes[frame][0], `restored continuation frame ${frame + 1}`);
          compared++;
        }
      }
      const image = saved.at(-1).bytes;
      const capture = [], restore = [];
      for (let index = 0; index < 100; index++) {
        let start = performance.now();
        const bytes = game.snapshot();
        capture.push((performance.now() - start) * 1000);
        start = performance.now();
        game.restore(bytes);
        restore.push((performance.now() - start) * 1000);
      }
      const distribution = values => { values.sort((a, b) => a - b); return { iterations: values.length, min_us: values[0], median_us: values[50], p95_us: values[95], max_us: values.at(-1) }; };
      reports.push({ session: session.session, frames: pads.length, restored_frames_compared: compared, checkpoints: saved.map(item => item.frame), snapshot_bytes: image.byteLength,
        capture_api_including_js_copy: distribution(capture), restore_api_including_js_copy: distribution(restore) });
    }
    const original = game.snapshot();
    const originalHash = hash();
    for (const kind of ['format', 'rom', 'corruption', 'truncation']) {
      let bad = original.slice();
      if (kind === 'format') new DataView(bad.buffer).setUint32(8, 99, true);
      if (kind === 'corruption') bad[256] ^= 1;
      if (kind === 'truncation') bad = bad.slice(0, 100);
      if (kind === 'rom') {
        bad[12] ^= 1;
        bad.set(new Uint8Array(await crypto.subtle.digest('SHA-1', bad.subarray(0, bad.length - 20))), bad.length - 20);
      }
      let rejected = false;
      try { game.restore(bad); } catch { rejected = true; }
      check(rejected, `${kind} was not rejected`);
      equal(hash(), originalHash, `${kind} rejection mutated live machine`);
    }
    for (let slot = 0; slot < 5; slot++) {
      await stateSlot('put', identity.sha1, slot, original);
      const bytes = await stateSlot('get', identity.sha1, slot);
      equal(hex(bytes), hex(original), `persisted slot ${slot + 1}`);
    }
    const first = sessions[0];
    const pads = first.recording.runs.flatMap(run => Array.from({ length: run.frames }, () => run.pads));
    game.reset();
    game.set_rewind(true);
    for (let frame = 0; frame < 12; frame++) { const [p1, p2] = pads[frame]; game.step(p1 & 255, p2 & 255, !!(p2 & 256)); }
    check(game.rewind(), 'rewind checkpoint missing');
    equal(game.frames(), 9, 'rewind frame');
    equal(hash(), first.frame_hashes[8][0], 'rewind exact image');
    check(game.rewind(), 'second rewind checkpoint missing');
    equal(game.frames(), 6, 'second rewind frame');
    for (let frame = 6; frame < 11; frame++) { const [p1, p2] = pads[frame]; game.step(p1 & 255, p2 & 255, !!(p2 & 256)); equal(hash(), first.frame_hashes[frame][0], 'rewind branch continuation'); }
    game.set_rewind(false);
    check(!game.rewind(), 'disabling rewind retained history');
    game.free();
    return { identity, sessions: reports, negative_cases: ['format', 'ROM', 'corruption', 'truncation'], slots: 5, rewind_branch: true, snapshot_bytes: original.length };
  }, sessions);
  await page.reload();
  const persisted = await page.evaluate(async ({ identity, snapshot_bytes }) => {
    const { stateSlot } = await import('/convenience.js');
    for (let slot = 0; slot < 5; slot++) {
      const bytes = await stateSlot('get', identity.sha1, slot);
      if (bytes?.length !== snapshot_bytes) throw new Error('slot did not survive page reload');
    }
    return true;
  }, proof);
  // Exercise real release frontend controls, not a test-only stepping path.
  await page.goto(`http://127.0.0.1:${server.address().port}/app/?backend=webgl`);
  await page.waitForFunction(() => !document.getElementById('rom').disabled);
  await page.locator('#rom').setInputFiles(romPath);
  await page.waitForFunction(() => !document.getElementById('play').disabled && window.tje.metrics.frames === 0, null, { timeout: 120000 });
  await page.locator('#convenience-settings').evaluate(element => { element.open = true; });
  await page.locator('#enable-save-states').check();
  await page.locator('#enable-rewind').check();
  await page.locator('#enable-fast-forward').check();
  await page.locator('#state-slot').selectOption('4');
  await page.locator('#save-state').click();
  await page.waitForFunction(() => document.getElementById('convenience-notice').textContent.includes('Saved local slot 5'));
  await page.locator('#play').click();
  await page.waitForFunction(() => window.tje.metrics.frames > 20);
  await page.locator('#load-state').click();
  await page.waitForFunction(() => window.tje.metrics.frames === 0 && document.getElementById('convenience-notice').textContent.includes('Loaded local slot 5'));
  await page.locator('#play').click();
  await page.waitForFunction(() => window.tje.metrics.frames > 12);
  await page.locator('#canvas').focus();
  const rewindStart = await page.evaluate(() => window.tje.metrics.frames);
  await page.keyboard.down('Backspace');
  await page.waitForFunction(frame => window.tje.metrics.frames < frame, rewindStart);
  await page.keyboard.up('Backspace');
  await page.locator('#fast-forward').click();
  assert.equal(await page.locator('#fast-forward').getAttribute('aria-pressed'), 'true');
  const fastStart = await page.evaluate(() => window.tje.metrics.frames);
  await page.waitForFunction(frame => window.tje.metrics.frames > frame + 12, fastStart);
  assert.equal(await page.evaluate(() => window.tje.metrics.audio.produced), 0, 'fast-forward advances the machine without queuing sound');
  await page.locator('#fast-forward').click();
  assert.equal(await page.locator('#fast-forward').getAttribute('aria-pressed'), 'false');
  await page.locator('#play').click();
  await page.reload();
  await page.waitForFunction(() => window.tje?.metrics.source === 'stored' && !document.getElementById('play').disabled, null, { timeout: 120000 });
  assert.equal(await page.locator('#enable-save-states').isChecked(), true);
  await page.locator('#convenience-settings').evaluate(element => { element.open = true; });
  await page.locator('#load-state').click();
  await page.waitForFunction(() => document.getElementById('convenience-notice').textContent.includes('Loaded local slot 5'));
  await page.locator('#canvas').focus();
  await page.keyboard.press('F4');
  await page.waitForFunction(() => document.getElementById('convenience-notice').textContent.includes('Selected local slot 1'));
  await page.keyboard.press('F2');
  await page.waitForFunction(() => document.getElementById('convenience-notice').textContent.includes('Saved local slot 1'));
  await page.keyboard.press('F3');
  await page.waitForFunction(() => document.getElementById('convenience-notice').textContent.includes('Loaded local slot 1'));
  await page.locator('#play').click();
  await page.locator('#canvas').focus();
  const heldStart = await page.evaluate(() => window.tje.metrics.frames);
  await page.keyboard.down('Backquote');
  await page.waitForFunction(frame => window.tje.metrics.frames > frame + 12, heldStart);
  assert.equal(await page.evaluate(() => window.tje.metrics.audio.produced), 0, 'held fast-forward mutes audio');
  await page.keyboard.up('Backquote');
  assert.deepEqual(errors, []);
  const receipt = { ...proof, persisted_across_reload: persisted, real_frontend_controls: true, browser: browser.version(), page_errors: errors };
  await writeFile(reportPath, JSON.stringify(receipt, null, 2));
  console.log(JSON.stringify(receipt, null, 2));
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
