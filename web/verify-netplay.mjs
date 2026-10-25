// One Chromium instance, real WebRTC and the production Wasm engine. No public
// signaling/STUN/relay endpoint is used. Keep the receipt in private scratch.
// Usage: TJE_WEB_ARTIFACT_DIR=/private/scratch/netplay PLAYWRIGHT_MODULE=... node web/verify-netplay.mjs /private/rev02.bin
import assert from 'node:assert/strict';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { startServer } from './serve.mjs';

assert(process.argv[2] || process.env.TJE_REV02_ROM, 'supply a private verified REV 02 ROM');
const rom = resolve(process.argv[2] ?? process.env.TJE_REV02_ROM);
const artifacts = resolve(process.env.TJE_WEB_ARTIFACT_DIR ?? '../scratch/netplay');
assert(artifacts.includes('/scratch/'), 'receipt belongs in private scratch');
const frames = Number(process.env.TJE_NETPLAY_FRAMES ?? 3600);
assert(Number.isInteger(frames) && frames >= 120 && frames <= 12000);
const image = new Uint8Array(await readFile(rom));
const recording = JSON.parse(await readFile(new URL('../sessions/rev02-two-player.tje.json', import.meta.url)));
const inputs = [];
for (const run of recording.runs) {
  for (let index = 0; index < run.frames && inputs.length < frames; index++) inputs.push(run.pads.map(pad => pad & 255));
  if (inputs.length === frames) break;
}
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(resolve(process.env.PLAYWRIGHT_MODULE)).href : 'playwright');
const server = await startServer(0);
const origin = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch({ headless: true, args: ['--enable-unsafe-swiftshader', '--use-angle=swiftshader', '--autoplay-policy=no-user-gesture-required'] });
const errors = [];
const requests = [];
try {
  // The real page loads and compiles from the local picker; no test-only Wasm
  // loader, fake snapshot, input stub or emulated network transport is used.
  const context = await browser.newContext();
  const page = await context.newPage();
  page.on('pageerror', error => errors.push(String(error)));
  page.on('request', request => requests.push({ method: request.method(), url: request.url() }));
  await page.goto(`${origin}/?backend=webgl`);
  await page.waitForSelector('body[data-state="empty"]', { timeout: 90000 });
  assert.equal(await page.isChecked('#net-stun'), false);
  assert.match(await page.textContent('#net-status'), /^Off/);
  await page.locator('#rom').setInputFiles(rom);
  await page.waitForSelector('body[data-state="ready"]', { timeout: 90000 });
  const proof = await page.evaluate(async ({ bytes, inputs }) => {
    const { WebGame, CompiledCartridge } = await import('./pkg/tje_web.js');
    const { RtcTransport } = await import('./netplay.js');
    const make = async () => {
      const game = await WebGame.create(document.createElement('canvas'), 'webgl');
      game.install(await CompiledCartridge.load(new Uint8Array(bytes), 48000, () => {}));
      return game;
    };
    const games = [await make()];
    const delay = 2;
    let started = performance.now();
    for (let frame = 0; frame < inputs.length; frame++) {
      const pads = frame < delay ? [0, 0] : inputs[frame - delay];
      games[0].step(pads[0], pads[1], true);
    }
    const expected = Array.from(games[0].state_hash());
    const baselineMs = performance.now() - started;
    games[0].reset();
    games.push(await make());
    const token = Array.from(crypto.getRandomValues(new Uint8Array(16)));
    let tick = 0;
    let received = 0;
    const queue = [];
    const failures = [];
    const connections = [0, 1].map(player => new RtcTransport({
      onPacket: packet => {
        const serial = received++;
        if (serial % 7 === 0) return; // Loss after a real WebRTC datagram arrives.
        queue.push({ due: tick + 6 + serial % 4, player, packet });
      },
      onError: error => failures.push(String(error)),
    }));
    const offer = await connections[0].host({ delay, token });
    const answer = await connections[1].join(offer);
    await connections[0].accept(answer);
    const readyDeadline = performance.now() + 15000;
    while (!connections.every(connection => connection.channel?.readyState === 'open')) {
      if (performance.now() > readyDeadline) throw new Error('Local direct WebRTC did not open');
      await new Promise(resolve => setTimeout(resolve, 10));
    }
    for (let player = 0; player < 2; player++) games[player].netplay_start(player, new Uint8Array(token), delay, 12, 0);
    started = performance.now();
    let completed = false;
    for (tick = 0; tick < inputs.length * 8 + 1000; tick++) {
      for (let index = queue.length - 1; index >= 0; index--) {
        if (queue[index].due <= tick) {
          const delivery = queue.splice(index, 1)[0];
          games[delivery.player].netplay_receive(delivery.packet, tick * 17);
        }
      }
      for (let player = 0; player < 2; player++) {
        games[player].netplay_poll(tick * 17);
        const frame = games[player].frames();
        if (frame < inputs.length) games[player].netplay_step(inputs[frame][player]);
        connections[player].send(games[player].netplay_packet());
      }
      if (games.every(game => JSON.parse(game.netplay_status()).stats.confirmed === inputs.length)) { completed = true; break; }
      await new Promise(resolve => setTimeout(resolve, 0));
      if (failures.length) throw new Error(failures.join('; '));
    }
    if (!completed) throw new Error('Browser rollback proof did not finish');
    const hashes = games.map(game => Array.from(game.state_hash()));
    if (hashes.some(hash => hash.some((byte, index) => byte !== expected[index]))) throw new Error('Browser rollback final state differs from no-latency run');
    const candidateTypes = [];
    for (const connection of connections) {
      const stats = await connection.pc.getStats();
      for (const stat of stats.values()) {
        if (stat.type === 'candidate-pair' && stat.state === 'succeeded' && stat.nominated) {
          candidateTypes.push({ local: stats.get(stat.localCandidateId)?.candidateType, remote: stats.get(stat.remoteCandidateId)?.candidateType });
        }
      }
    }
    const receipt = { frames: inputs.length, delayFrames: delay, oneWayFrames: 6, jitterFrames: 3, dropEvery: 7,
      baselineMs, elapsedMs: performance.now() - started, simulatedTicks: tick, receivedDatagrams: received,
      peers: games.map(game => JSON.parse(game.netplay_status())), expectedHash: expected, actualHashes: hashes,
      offerBytes: offer.length, answerBytes: answer.length, candidateTypes };
    for (const connection of connections) connection.close();
    for (const game of games) game.free();
    return receipt;
  }, { bytes: Array.from(image), inputs });
  assert(proof.peers.every(peer => peer.stats.rollbacks > 0 && peer.stats.largest_rollback <= 12));
  assert(proof.candidateTypes.length > 0 && proof.candidateTypes.every(pair => pair.local !== 'relay' && pair.remote !== 'relay'));
  await context.close();

  // End-to-end host/join settings flow through two separate browser contexts.
  const contexts = await Promise.all([browser.newContext(), browser.newContext()]);
  const pages = await Promise.all(contexts.map(context => context.newPage()));
  for (const page of pages) {
    page.on('pageerror', error => errors.push(String(error)));
    page.on('request', request => requests.push({ method: request.method(), url: request.url() }));
    await page.goto(`${origin}/?backend=webgl`);
    await page.waitForSelector('body[data-state="empty"]', { timeout: 90000 });
    await page.locator('#rom').setInputFiles(rom);
    await page.waitForSelector('body[data-state="ready"]', { timeout: 90000 });
    await page.locator('#netplay-settings').evaluate(element => { element.open = true; });
  }
  await pages[0].click('#net-host');
  await pages[0].waitForFunction(() => document.getElementById('net-local').value.startsWith('tje-rtc1:'), null, { timeout: 30000 });
  const offer = await pages[0].inputValue('#net-local');
  await pages[1].fill('#net-remote', offer);
  await pages[1].click('#net-join');
  await pages[1].waitForFunction(() => document.getElementById('net-local').value.startsWith('tje-rtc1:'), null, { timeout: 30000 });
  await pages[0].fill('#net-remote', await pages[1].inputValue('#net-local'));
  await pages[0].click('#net-answer');
  for (const page of pages) {
    await page.waitForFunction(() => window.tje.metrics.netplay?.stats.confirmed >= 120, null, { timeout: 60000 });
    assert.equal(await page.isDisabled('#reset'), true);
    assert.equal(await page.isDisabled('#rom'), true);
  }
  const ui = await Promise.all(pages.map(page => page.evaluate(() => window.tje.metrics.netplay)));
  await pages[0].click('#net-disconnect');
  for (const page of pages) {
    await page.waitForFunction(() => window.tje.metrics.netplay === null);
    assert.equal(await page.isDisabled('#reset'), false);
  }
  assert.deepEqual(errors, []);
  assert(requests.every(request => request.method === 'GET' && request.url.startsWith(origin)), 'no network uploads or public signaling requests are allowed');
  await mkdir(artifacts, { recursive: true });
  const receipt = { browser: browser.version(), proof, ui, pageErrors: errors, publicRequests: 0 };
  await writeFile(resolve(artifacts, 'netplay-browser.json'), JSON.stringify(receipt, null, 2));
  console.log(JSON.stringify(receipt, null, 2));
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
