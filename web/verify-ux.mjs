// Browser-only UX checks used by verify.mjs. Controller objects and denied audio
// gestures are explicitly synthetic; emulation, rendering and audio remain real.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { SETTINGS_VERSION } from './storage.js';

export async function installUxFixtures(context) {
  await context.addInitScript(() => {
    window.__testPads = [];
    Object.defineProperty(navigator, 'getGamepads', { value: () => window.__testPads });
    if (location.protocol === 'http:' && localStorage.getItem('tje-settings') === null && localStorage.getItem('tje-settings-v1') === null) {
      localStorage.setItem('tje-settings-v1', JSON.stringify({ volume: 0.63, scale: 'integer', p2: false }));
    }
  });
}

export async function verifyDisplayProfiles({ browser, url, artifacts, backend, disableWebgpu }) {
  // A separate, unseeded browser context proves the default is not supplied by
  // the legacy-settings fixture used by the rest of the acceptance run.
  const context = await browser.newContext({ viewport: { width: 1000, height: 1100 } });
  const errors = [];
  try {
    if (disableWebgpu) await context.addInitScript(() => Object.defineProperty(navigator, 'gpu', { value: undefined }));
    const page = await context.newPage();
    page.on('pageerror', error => errors.push(String(error)));
    await page.goto(url);
    await page.waitForSelector('body[data-state="empty"]', { timeout: 45000 });
    const fresh = await page.evaluate(() => window.tje.metrics);
    assert.equal(fresh.settings.pixelShape, 'native');
    assert.equal(fresh.settings.scale, 'integer');
    assert.equal(fresh.settings.filter, 'sharp');
    assert.equal(await page.locator('#pixel-shape').inputValue(), 'native');
    assert.equal(await page.locator('#scale').inputValue(), 'integer');
    assert.equal(await page.locator('#filter').inputValue(), 'sharp');
    assert.equal(await page.evaluate(() => localStorage.getItem('tje-settings')), null, 'fresh defaults need no saved setting');
    await page.screenshot({ path: `${artifacts}/${backend}-fresh-profile-defaults.png`, fullPage: true });
    await page.evaluate(() => localStorage.setItem('tje-settings', JSON.stringify({ version: 2, scale: 'fit', filter: 'nearest', volume: 0.37 })));
    await page.reload();
    await page.waitForSelector('body[data-state="empty"]', { timeout: 45000 });
    assert.match(await page.locator('#settings-notice').textContent(), /migrated/);
    await page.reload();
    await page.waitForSelector('body[data-state="empty"]', { timeout: 45000 });
    const restored = await page.evaluate(() => window.tje.metrics);
    assert.equal(restored.settings.version, SETTINGS_VERSION);
    assert.equal(restored.settings.pixelShape, 'square', 'old Fit was a square-pixel mode');
    assert.equal(restored.settings.scale, 'fit');
    assert.equal(restored.settings.filter, 'nearest');
    assert.equal(restored.settings.volume, 0.37);
    assert.equal(await page.locator('#pixel-shape').inputValue(), 'square');
    assert.equal(await page.locator('#scale').inputValue(), 'fit');
    assert.equal(await page.locator('#filter').inputValue(), 'nearest');
    assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('tje-settings')).version), SETTINGS_VERSION);
    await page.screenshot({ path: `${artifacts}/${backend}-legacy-fit-reloaded.png`, fullPage: true });
    assert.deepEqual(errors, []);
    return { fresh: { pixelShape: fresh.settings.pixelShape, scale: fresh.settings.scale, filter: fresh.settings.filter, canvas: fresh.canvas },
      legacyAfterTwoReloads: { version: restored.settings.version, pixelShape: restored.settings.pixelShape, scale: restored.settings.scale, filter: restored.settings.filter, canvas: restored.canvas }, errors };
  } finally { await context.close(); }
}

async function storedDigest(page) {
  return page.evaluate(async () => {
    const { cartridgeStore } = await import('./storage.js');
    const record = await cartridgeStore('get');
    if (!record) return null;
    const digest = await crypto.subtle.digest('SHA-1', record.bytes);
    return Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, '0')).join('');
  });
}

export async function verifyEmptyUx(page, metrics) {
  assert.equal((await metrics()).settings.version, SETTINGS_VERSION);
  assert.equal((await metrics()).settings.volume, 0.63, 'legacy volume must survive the real migration');
  assert.equal((await metrics()).settings.pixelShape, 'square', 'old Integer was a square-pixel mode');
  assert.equal((await metrics()).settings.scale, 'integer');
  assert.equal(await page.evaluate(() => localStorage.getItem('tje-settings-v1')), null);
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('tje-settings')).version), SETTINGS_VERSION);
  const csp = await page.locator('meta[http-equiv="Content-Security-Policy"]').getAttribute('content');
  assert(csp.includes("connect-src 'self'"));
  assert(!csp.includes("'unsafe-inline'") && !csp.includes("'unsafe-eval'"));
  await page.keyboard.press('Tab');
  assert.equal(await page.evaluate(() => document.activeElement.textContent), 'Skip to game controls');
  await page.locator('#rom').setInputFiles({ name: 'wrong-game.bin', mimeType: 'application/octet-stream', buffer: Buffer.from('Not a cartridge image') });
  await page.locator('#rom-error').waitFor({ state: 'visible' });
  assert.match(await page.locator('#rom-error').textContent(), /wrong game or damaged image/);
  assert.equal((await metrics()).revision, '');
  assert.equal(await storedDigest(page), null, 'a wrong ROM must not enter IndexedDB');
}

function mapping(page, player, action, kind) {
  return page.locator(`[data-player="${player}"][data-action="${action}"][data-kind="${kind}"]`);
}

async function settled(page) {
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
}

export async function verifyLoadedUx({ page, context, metrics, rom, artifacts, backend }) {
  const checks = { settingsMigration: true, invalidRomRejected: true, controllers: 'synthetic Gamepad API objects; no physical pad claim' };
  const originalDigest = createHash('sha1').update(await readFile(rom)).digest('hex');
  const beforeBadDump = (await metrics()).frames;
  const badDump = Buffer.from(await readFile(rom));
  badDump[badDump.length - 1] ^= 1;
  await page.locator('#rom').setInputFiles({ name: 'bad-dump.bin', mimeType: 'application/octet-stream', buffer: badDump });
  await page.locator('#rom-error').waitFor({ state: 'visible' });
  assert.match(await page.locator('#rom-error').textContent(), /previous cartridge and its saved copy were kept/);
  assert.equal((await metrics()).frames, beforeBadDump);
  assert.equal(await storedDigest(page), originalDigest, 'bad dump must not replace the accepted IndexedDB ROM');
  checks.badDumpPreservesCartridge = true;

  await page.locator('#control-settings > summary').click();
  await mapping(page, 0, 'up', 'keyboard').click();
  await page.keyboard.press('KeyF');
  assert.equal(await page.evaluate(() => !!document.fullscreenElement), false, 'reserved F must not escape a remap capture');
  await page.keyboard.press('Escape');
  assert.equal((await metrics()).settings.players[0].keys.up, 'ArrowUp');
  await mapping(page, 0, 'up', 'keyboard').click();
  await page.keyboard.press('Shift+Tab');
  assert.equal(await page.locator('#cancel-mapping').isVisible(), false, 'Shift+Tab cancels capture and navigates');
  assert.equal((await metrics()).settings.players[0].keys.up, 'ArrowUp');
  await mapping(page, 0, 'up', 'keyboard').click();
  await page.keyboard.press('KeyO');
  await mapping(page, 1, 'up', 'keyboard').click();
  await page.keyboard.press('KeyP');
  assert.equal((await metrics()).settings.players[0].keys.up, 'KeyO');
  assert.equal((await metrics()).settings.players[1].keys.up, 'KeyP');
  checks.keyboardRemapping = true;

  await page.evaluate(() => {
    const pad = (id, index) => ({ id, index, connected: true, mapping: '', axes: [0, 0, -1], buttons: Array.from({ length: 17 }, () => ({ pressed: false, value: 0 })) });
    window.__testPads = [pad('Synthetic non-standard player one', 0), pad('Synthetic non-standard player two', 2)];
  });
  await page.waitForFunction(() => window.tje.metrics.gamepads === 2);
  await page.selectOption('#pad-0', JSON.stringify({ id: 'Synthetic non-standard player one', index: 0 }));
  await page.selectOption('#pad-1', JSON.stringify({ id: 'Synthetic non-standard player two', index: 2 }));
  await mapping(page, 0, 'a', 'gamepad').click();
  await page.evaluate(() => { window.__testPads[0].buttons[5] = { pressed: true, value: 1 }; });
  await page.waitForFunction(() => window.tje.metrics.settings.players[0].gamepad.bindings.a[0].index === 5);
  await page.evaluate(() => { window.__testPads[0].buttons[5] = { pressed: false, value: 0 }; });
  await mapping(page, 1, 'b', 'gamepad').click();
  await page.evaluate(() => { window.__testPads[1].axes[2] = 1; });
  await page.waitForFunction(() => window.tje.metrics.settings.players[1].gamepad.bindings.b[0].kind === 'axis');
  const binding = (await metrics()).settings.players[1].gamepad.bindings.b[0];
  assert.deepEqual(binding, { kind: 'axis', index: 2, direction: 1, center: -1, threshold: 0.5 });
  await page.evaluate(() => { window.__testPads[1].axes[2] = -1; });
  checks.nonstandardButtonAndAxisRemapping = true;

  await page.click('#play');
  await page.waitForSelector('body[data-state="running"]');
  await page.keyboard.down('KeyO');
  await page.keyboard.down('KeyP');
  await page.waitForFunction(() => (window.tje.metrics.lastInputs[0] & 1) && (window.tje.metrics.lastInputs[1] & 1));
  await page.keyboard.up('KeyO');
  await page.keyboard.up('KeyP');
  await page.evaluate(() => { window.__testPads[0].buttons[5] = { pressed: true, value: 1 }; window.__testPads[1].axes[2] = 1; });
  await page.waitForFunction(() => (window.tje.metrics.lastInputs[0] & 16) && (window.tje.metrics.lastInputs[1] & 32));
  await page.evaluate(() => { window.__testPads[0].buttons[5] = { pressed: false, value: 0 }; window.__testPads[1].axes[2] = -1; });
  checks.remappedControllerWires = true;

  let visibilityMode = '';
  async function hide() {
    if (visibilityMode !== 'injected visibilitychange') {
      const other = await context.newPage();
      await other.bringToFront();
      const hidden = await page.waitForFunction(() => document.hidden, null, { timeout: 1500, polling: 50 }).then(() => true, () => false);
      if (hidden) { visibilityMode = 'real browser tab switch'; return other; }
      await other.close();
      visibilityMode = 'injected visibilitychange';
    }
    await page.evaluate(() => {
      Object.defineProperty(document, 'hidden', { configurable: true, get: () => true });
      document.dispatchEvent(new Event('visibilitychange'));
    });
    return null;
  }
  async function show(other) {
    if (other) { await other.close(); await page.bringToFront(); }
    else await page.evaluate(() => {
      Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
      document.dispatchEvent(new Event('visibilitychange'));
    });
  }
  let other = await hide();
  await page.waitForFunction(() => window.tje.metrics.state === 'paused' && window.tje.metrics.audio.state === 'suspended', null, { polling: 50 });
  await show(other);
  await page.waitForFunction(() => window.tje.metrics.state === 'running' && window.tje.metrics.audio.state === 'running');
  await page.waitForFunction(() => window.tje.metrics.audio.played > 1000);
  checks.audioAutomaticResume = true;

  other = await hide();
  await page.waitForFunction(() => window.tje.metrics.audio.state === 'suspended', null, { polling: 50 });
  await page.evaluate(() => {
    window.__originalResume = AudioContext.prototype.resume;
    AudioContext.prototype.resume = function () { return Promise.reject(new DOMException('Synthetic gesture required', 'NotAllowedError')); };
  });
  await show(other);
  await page.waitForFunction(() => window.tje.metrics.state === 'paused' && document.getElementById('status').textContent.includes('Click Play to retry'));
  await page.evaluate(() => { AudioContext.prototype.resume = window.__originalResume; delete window.__originalResume; });
  assert.equal(await page.locator('#play').isEnabled(), true);
  await page.click('#play');
  await page.waitForFunction(() => window.tje.metrics.audio.played > 1000 && window.tje.metrics.state === 'running');
  checks.audioGestureRetry = true;
  await page.click('#play');
  const pausedFrame = (await metrics()).frames;
  other = await hide();
  await show(other);
  await settled(page);
  assert.equal((await metrics()).state, 'paused');
  assert.equal((await metrics()).frames, pausedFrame, 'a manually paused game must not resume on tab return');
  checks.manualPausePreserved = true;
  checks.visibilityMode = visibilityMode;

  await page.locator('#canvas').focus();
  await page.keyboard.press('KeyF');
  await page.waitForFunction(() => document.fullscreenElement?.id === 'screen' && document.getElementById('fullscreen').getAttribute('aria-pressed') === 'true');
  await page.keyboard.press('KeyF');
  await page.waitForFunction(() => !document.fullscreenElement && document.getElementById('fullscreen').getAttribute('aria-pressed') === 'false');
  await settled(page);
  const screen = await page.locator('#screen').boundingBox();
  assert(Math.abs(screen.height - screen.width * 0.7) <= 1, 'exiting fullscreen must restore the normal screen aspect ratio');
  checks.fullscreenKeyboard = true;

  const modes = [];
  const modeDigests = new Set();
  await page.selectOption('#filter', 'sharp');
  for (const pixelShape of ['native', 'square']) {
    for (const scale of ['integer', 'fit']) {
      await page.selectOption('#pixel-shape', pixelShape);
      await page.selectOption('#scale', scale);
      await settled(page);
      const value = await metrics();
      const cssCanvas = await page.locator('#canvas').boundingBox();
      const dpr = await page.evaluate(() => Math.min(devicePixelRatio || 1, 3));
      const computedCanvas = { width: Math.max(1, Math.min(4096, Math.round(cssCanvas.width * dpr))),
        height: Math.max(1, Math.min(4096, Math.round(cssCanvas.height * dpr))) };
      assert.deepEqual(value.canvas, computedCanvas, 'canvas backing size must follow its physical-pixel display area');
      assert.equal(value.settings.pixelShape, pixelShape);
      assert.equal(value.settings.scale, scale);
      const capture = `${backend}-${pixelShape}-${scale}-sharp.png`;
      const image = await page.locator('#canvas').screenshot();
      await writeFile(`${artifacts}/${capture}`, image);
      modeDigests.add(createHash('sha256').update(image).digest('hex'));
      modes.push({ pixelShape, scale, filter: 'sharp', canvas: value.canvas,
        cssCanvas: { width: cssCanvas.width, height: cssCanvas.height }, dpr, capture });
    }
  }
  assert.equal(modeDigests.size, 4, 'each pixel-shape / scale combination must produce its own viewport');
  checks.displayModes = modes;
  await page.selectOption('#pixel-shape', 'native');
  await settled(page);
  const sharp = await page.locator('#canvas').screenshot();
  await writeFile(`${artifacts}/${backend}-sharp.png`, sharp);
  await page.selectOption('#filter', 'nearest');
  await settled(page);
  const nearest = await page.locator('#canvas').screenshot();
  await page.selectOption('#filter', 'linear');
  await settled(page);
  const linear = await page.locator('#canvas').screenshot();
  assert(!nearest.equals(linear), 'nearest and linear must produce different pixels on a fractional-fit game frame');
  assert(!sharp.equals(nearest) && !sharp.equals(linear), 'sharp-bilinear must retain its own crisp, edge-blended pixels');
  await writeFile(`${artifacts}/${backend}-nearest.png`, nearest);
  await writeFile(`${artifacts}/${backend}-linear.png`, linear);
  checks.filterChangesRenderedPixels = true;
  await page.selectOption('#pixel-shape', 'square');
  assert.equal(await page.locator('#volume').getAttribute('aria-valuetext'), '45 percent');
  const unnamed = await page.locator('button').evaluateAll(buttons => buttons.filter(button => !(button.getAttribute('aria-label') ?? button.textContent).trim()).length);
  const unlabelled = await page.locator('input,select').evaluateAll(fields => fields.filter(field => !field.labels?.length && !field.getAttribute('aria-label')).length);
  assert.equal(unnamed, 0);
  assert.equal(unlabelled, 0);
  await page.setViewportSize({ width: 360, height: 1000 });
  await settled(page);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, 'controls must reflow without horizontal overflow');
  await page.screenshot({ path: `${artifacts}/${backend}-mobile-settings.png`, fullPage: true });
  await page.setViewportSize({ width: 1000, height: 1100 });
  checks.accessibilityBasics = true;
  return checks;
}

export async function verifyRestoredUx(page, metrics) {
  const settings = (await metrics()).settings;
  assert.equal(settings.version, SETTINGS_VERSION);
  assert.equal(settings.filter, 'linear');
  assert.equal(settings.pixelShape, 'square');
  assert.equal(settings.players[0].keys.up, 'KeyO');
  assert.equal(settings.players[1].keys.up, 'KeyP');
  assert.equal(settings.players[0].gamepad.bindings.a[0].index, 5);
  assert.equal(settings.players[1].gamepad.bindings.b[0].center, -1);
  assert.equal(settings.players[1].gamepad.device.id, 'Synthetic non-standard player two');
  assert.equal(await page.locator('#filter').inputValue(), 'linear');
  assert.equal(await page.locator('#pixel-shape').inputValue(), 'square');
  assert.match(await page.locator('#pad-1').textContent(), /not connected/);
}
