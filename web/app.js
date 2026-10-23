import init, { WebGame } from './pkg/tje_web.js';
import { AudioStream } from './audio.js';
import { Inputs } from './input.js';
import { ControlSettings } from './controls.js';
import { cartridgeStore, readSettings, saveSettings } from './storage.js';

const $ = id => document.getElementById(id);
const settingsState = readSettings();
const settings = settingsState.settings;
const audio = new AudioStream();
let canvas = $('canvas');
let game;
let wasm;
let inputs;
let controls;
let starting = false;
let resumeOnVisible = false;
let playbackVersion = 0;
let pauseTask = Promise.resolve();
let loaded = false;
let running = false;
let busy = false;
let state = 'loading';
let revision = '';
let source = '';
let storageWarning = '';
let lastStatus = 0;
let activeSince = 0;
let activeSeconds = 0;
let measuredFrames = 0;
let emulationMs = 0;
let maxFrameMs = 0;
let presented = 0;
let stalls = 0;
let videoDirty = true;
const histogram = new Uint32Array(101);

function status(message, nextState = state) {
  state = nextState;
  document.body.dataset.state = state;
  $('status').textContent = message;
}

function pause(message = 'Paused. Click Play to resume.', resumeWhenVisible = false) {
  resumeOnVisible = resumeWhenVisible;
  playbackVersion++;
  if (running) activeSeconds += (performance.now() - activeSince) / 1000;
  running = false;
  inputs?.clear();
  $('play').textContent = 'Play / enable audio';
  $('play').setAttribute('aria-pressed', 'false');
  pauseTask = audio.pause().catch(error => {
    resumeOnVisible = false;
    status(`Audio could not pause cleanly: ${error.message ?? error}. Click Play to retry.`, 'paused');
  });
  if (loaded) status(message, 'paused');
  return pauseTask;
}

function errorMessage(error) {
  console.error(error);
  status(String(error?.message ?? error), 'error');
}

function importFailure(error, name = 'This file') {
  const detail = String(error?.message ?? error);
  $('rom-error').textContent = `${name} was not loaded. ${detail} Use an unmodified, verified REV 02 or REV 00 dump; a wrong game or damaged image will not pass identification. ${loaded ? 'The previous cartridge and its saved copy were kept.' : 'No cartridge was replaced.'}`;
  $('rom-error').hidden = false;
  status(loaded ? `Import rejected. The previous cartridge ${running ? 'continues playing' : 'is paused and can still play'}.` : 'Import rejected. Choose a verified cartridge.', loaded ? (running ? 'running' : 'paused') : 'empty');
}

function resetMeasurements() {
  activeSeconds = measuredFrames = emulationMs = maxFrameMs = presented = stalls = 0;
  histogram.fill(0);
}

function snapshot() {
  const wallSeconds = activeSeconds + (running ? (performance.now() - activeSince) / 1000 : 0);
  let cumulative = 0;
  let p95 = 0;
  for (; p95 < histogram.length; p95++) {
    cumulative += histogram[p95];
    if (cumulative >= measuredFrames * 0.95) break;
  }
  return {
    state, mode: 'replace', registry: 'translated', revision, source, backend: game?.backend(), adapter: game?.adapter(),
    frames: game?.frames() ?? 0, titleFrame: game?.title_frame() ?? null,
    gameFrame: game?.game_frame() ?? null, logicTicks: game?.logic_ticks() ?? 0,
    targetFps: game?.target_fps() ?? 0, measuredFrames, wallSeconds,
    actualFps: wallSeconds > 0 ? measuredFrames / wallSeconds : 0,
    emulationMs, capacityFps: emulationMs > 0 ? measuredFrames * 1000 / emulationMs : 0,
    maxFrameMs, p95FrameMs: p95, presented, stalls, audio: audio.snapshot(),
    presentationFps: wallSeconds > 0 ? presented / wallSeconds : 0,
    gamepads: inputs?.pads ?? 0, keyEvents: inputs?.keyEvents.slice() ?? [0, 0],
    lastInputs: inputs?.last.slice() ?? [0, 0, false], settings: structuredClone(settings), storageWarning,
    canvas: { width: canvas.width, height: canvas.height },
    secure: isSecureContext, crossOriginIsolated,
  };
}

// Read-only observability for the local Playwright acceptance harness. There is
// no test-only stepping path: tests operate the actual controls and device clock.
Object.defineProperty(window, 'tje', { value: Object.freeze({ get metrics() { return snapshot(); } }) });

function resize() {
  const rect = canvas.getBoundingClientRect();
  const dpr = Math.min(devicePixelRatio || 1, 3);
  const width = Math.max(1, Math.min(4096, Math.round(rect.width * dpr)));
  const height = Math.max(1, Math.min(4096, Math.round(rect.height * dpr)));
  if (canvas.width !== width) canvas.width = width;
  if (canvas.height !== height) canvas.height = height;
  videoDirty = true;
  if (loaded && !running) game.render(width, height, settings.scale === 'integer', settings.filter === 'linear');
}

function frame(now) {
  requestAnimationFrame(frame);
  try {
    inputs?.samplePads();
    controls?.update();
    if (!running) return;
    if (audio.context.state !== 'running') {
      void pause('Audio was interrupted; trying to resume. Click Play if another gesture is required.', true).then(tryAutomaticResume);
      return;
    }
    audio.syncClock(now);
    // A long tab/OS stall pauses real time, never skips emulated game logic.
    if (audio.needed() > audio.context.sampleRate * 0.25) {
      audio.begin(true);
      stalls++;
    }
    let steps = 0;
    while (audio.needed() > 0 && steps < 6) {
      steps++;
      const [p1, p2, p2Connected] = inputs.poll(settings.p2);
      const started = performance.now();
      game.step(p1, p2, p2Connected);
      const elapsed = performance.now() - started;
      emulationMs += elapsed;
      maxFrameMs = Math.max(maxFrameMs, elapsed);
      histogram[Math.min(100, Math.ceil(elapsed))]++;
      measuredFrames++;
      // memory.buffer is looked up after every step: wasm memory can grow.
      audio.push(new Float32Array(wasm.memory.buffer, game.audio_ptr(), game.audio_len()));
    }
    if ((steps > 0 || videoDirty) && game.render(canvas.width, canvas.height, settings.scale === 'integer', settings.filter === 'linear')) {
      presented++;
      videoDirty = false;
    }
    if (now - lastStatus > 500) {
      lastStatus = now;
      const value = snapshot();
      document.body.dataset.title = String(value.titleFrame !== null);
      document.body.dataset.game = String(value.gameFrame !== null);
      $('metrics').textContent = `${value.backend} · ${value.actualFps.toFixed(1)} / ${value.targetFps.toFixed(2)} fps · `
        + `${value.capacityFps.toFixed(0)} emulation fps capacity · ${value.audio.sampleRate} Hz · `
        + `${value.audio.played ?? 0} audio frames played · ${value.gamepads} gamepad(s)`;
    }
  } catch (error) {
    void pause();
    errorMessage(error);
  }
}

async function loadCartridge(bytes, name, origin) {
  const acceptedRevision = game.load_rom(new Uint8Array(bytes));
  revision = acceptedRevision;
  source = origin;
  loaded = true;
  resetMeasurements();
  inputs.clear();
  $('cartridge').textContent = `${revision} · ${name} · ${origin === 'stored' ? 'restored from this browser' : 'imported locally'}`;
  $('play').disabled = $('reset').disabled = $('forget').disabled = origin !== 'stored';
  document.body.dataset.title = document.body.dataset.game = 'false';
  resize();
  if (origin === 'stored') {
    status(`${revision} ready. Click Play to enable sound and start.${storageWarning}`, 'ready');
  } else {
    status(`${revision} accepted. Saving privately in this browser…`, 'loading');
  }
}

async function importFile(file) {
  if (!file || busy) return;
  busy = true;
  $('rom').disabled = true;
  $('rom-error').hidden = true;
  try {
    await pause();
    // A bounded picker read also avoids loading enormous accidental archives.
    if (file.size > 16 * 1024 * 1024) throw new Error('Choose a ROM or ZIP smaller than 16 MiB.');
    const bytes = await file.arrayBuffer();
    await loadCartridge(bytes, file.name, 'file');
    try {
      await cartridgeStore('put', { bytes, name: file.name });
      storageWarning = '';
      status(`${revision} ready. Saved privately in this browser. Click Play to start.`, 'ready');
    } catch (error) {
      storageWarning = ` Storage unavailable: ${error.message ?? error}. The current cartridge can still play.`;
      status(`${revision} ready.${storageWarning}`, 'ready');
    }
  } catch (error) {
    importFailure(error, file.name);
  } finally {
    busy = false;
    $('rom').disabled = false;
    $('rom').value = '';
    $('play').disabled = $('reset').disabled = $('forget').disabled = !loaded;
  }
}

async function tryAutomaticResume() {
  await pauseTask;
  if (resumeOnVisible && !document.hidden && !busy && loaded) await startPlayback(true);
}

async function startPlayback(automatic = false) {
  if (!loaded || busy) return;
  busy = starting = true;
  const request = ++playbackVersion;
  $('play').setAttribute('aria-busy', 'true');
  try {
    await pauseTask;
    if (request !== playbackVersion || document.hidden) return;
    const sampleRate = await audio.start(settings.volume);
    if (document.hidden || request !== playbackVersion) {
      await audio.pause();
      return;
    }
    game.set_sample_rate(sampleRate);
    activeSince = performance.now();
    running = true;
    resumeOnVisible = false;
    $('play').textContent = 'Pause';
    $('play').setAttribute('aria-pressed', 'true');
    if (!automatic) canvas.focus();
    status(`${revision} running: registered Rust translations and original cartridge code on the hybrid host.${storageWarning}`, 'running');
  } catch (error) {
    if (request !== playbackVersion && resumeOnVisible) {
      status('Audio was paused while starting; returning to this tab will try to resume.', 'paused');
    } else {
      resumeOnVisible = false;
      status(`${automatic ? 'Audio could not resume automatically' : 'Playback/audio could not start'}: ${error.message ?? error}. Click Play to retry.`, 'paused');
    }
  } finally {
    busy = starting = false;
    $('play').setAttribute('aria-busy', 'false');
    if (resumeOnVisible && !document.hidden) queueMicrotask(tryAutomaticResume);
  }
}

$('play').addEventListener('click', () => {
  if (busy || !loaded) return;
  if (running) void pause();
  else void startPlayback();
});

$('reset').addEventListener('click', async () => {
  if (!loaded || busy) return;
  busy = true;
  try {
    await pause();
    game.reset();
    resetMeasurements();
    resize();
    status('Cartridge reset. Click Play to start.', 'ready');
  } catch (error) { errorMessage(error); }
  finally { busy = false; }
});

$('rom').addEventListener('change', event => { void importFile(event.target.files[0]); });
for (const type of ['dragenter', 'dragover']) {
  window.addEventListener(type, event => {
    if (!event.dataTransfer?.types.includes('Files')) return;
    event.preventDefault();
    $('drop-zone').classList.add('dragging');
  });
}
window.addEventListener('dragleave', event => {
  if (!event.relatedTarget) $('drop-zone').classList.remove('dragging');
});
window.addEventListener('drop', event => {
  event.preventDefault();
  $('drop-zone').classList.remove('dragging');
  if (!game || !event.dataTransfer?.files.length) return;
  if (event.dataTransfer.files.length !== 1) {
    importFailure('Drop exactly one ROM or ZIP at a time.', 'The drop');
    return;
  }
  void importFile(event.dataTransfer.files[0]);
});

$('forget').addEventListener('click', async () => {
  if (busy) return;
  busy = true;
  try {
    await pause();
    await cartridgeStore('delete');
    // Releasing the wasm object also releases its in-memory cartridge. Reload
    // starts an empty session and leaves the user's settings untouched.
    game.free();
    location.reload();
  } catch (error) {
    errorMessage(error);
    busy = false;
  }
});

function persistSettings() {
  if (!settingsState.persistable) return;
  try {
    saveSettings(settings);
    $('settings-notice').textContent = '';
  } catch (error) {
    $('settings-notice').textContent = `Settings could not be saved: ${error.message ?? error}. Current choices still work for this visit.`;
  }
}

function updateSettings() {
  settings.volume = Number($('volume').value) / 100;
  settings.scale = $('scale').value;
  settings.filter = $('filter').value;
  settings.p2 = $('p2').checked;
  $('volume-value').value = `${$('volume').value}%`;
  $('volume').setAttribute('aria-valuetext', `${$('volume').value} percent`);
  canvas.classList.toggle('linear-filter', settings.filter === 'linear');
  audio.setVolume(settings.volume);
  persistSettings();
  resize();
}
$('settings-notice').textContent = settingsState.notice;
$('volume').value = String(settings.volume * 100);
$('volume-value').value = `${$('volume').value}%`;
$('volume').setAttribute('aria-valuetext', `${$('volume').value} percent`);
$('scale').value = settings.scale;
$('filter').value = settings.filter;
$('p2').checked = settings.p2;
canvas.classList.toggle('linear-filter', settings.filter === 'linear');
for (const id of ['volume', 'scale', 'filter', 'p2']) $(id).addEventListener('input', updateSettings);

let fullscreenReturnFocus;
function toggleFullscreen() {
  if (!document.fullscreenElement) fullscreenReturnFocus = document.activeElement;
  const operation = document.fullscreenElement ? document.exitFullscreen() : $('screen').requestFullscreen?.();
  if (!operation) { status('Fullscreen is unavailable in this browser.'); return; }
  operation.catch(error => status(`Fullscreen could not change: ${error.message ?? error}`));
}
$('fullscreen').addEventListener('click', toggleFullscreen);
window.addEventListener('keydown', event => {
  if (event.code !== 'KeyF' || event.repeat || event.ctrlKey || event.altKey || event.metaKey) return;
  if (event.target instanceof Element && event.target.closest('input,select,textarea,[contenteditable]')) return;
  event.preventDefault();
  toggleFullscreen();
});
document.addEventListener('fullscreenchange', () => {
  const active = !!document.fullscreenElement;
  $('fullscreen').textContent = active ? 'Exit fullscreen (F)' : 'Fullscreen (F)';
  $('fullscreen').setAttribute('aria-pressed', String(active));
  if (active) canvas.focus();
  else fullscreenReturnFocus?.focus({ preventScroll: true });
  resize();
});
document.addEventListener('visibilitychange', () => {
  if (document.hidden && (running || starting || resumeOnVisible)) {
    void pause('Tab hidden; game and audio paused. Returning will try to resume.', true);
  } else if (!document.hidden) {
    void tryAutomaticResume();
  }
});

async function boot() {
  if (!isSecureContext) throw new Error('Use HTTPS or localhost; WebGPU and AudioWorklet require a secure context.');
  wasm = await init();
  const requested = new URLSearchParams(location.search).get('backend') ?? 'auto';
  if (!['auto', 'webgpu', 'webgl'].includes(requested)) throw new Error(`Unknown backend: ${requested}`);
  const candidates = requested === 'auto' ? ['webgpu', 'webgl'] : [requested];
  const failures = [];
  for (const backend of candidates) {
    try {
      game = await WebGame.create(canvas, backend);
      break;
    } catch (error) {
      failures.push(`${backend}: ${error.message ?? error}`);
      // Browsers forbid changing a canvas's context type after initialization.
      const fresh = canvas.cloneNode(false);
      canvas.replaceWith(fresh);
      canvas = fresh;
    }
  }
  if (!game) throw new Error(`No graphics backend is available. ${failures.join('; ')}`);
  inputs = new Inputs(settings.players);
  controls = new ControlSettings(settings, inputs, persistSettings, () => { void pause('Paused for control assignment. Click Play when finished.'); });
  new ResizeObserver(resize).observe($('screen'));
  status(`Checking private cartridge storage. Graphics: ${game.backend()}.`, 'loading');
  let stored;
  try { stored = await cartridgeStore('get'); }
  catch (error) { storageWarning = ` Browser cartridge storage could not be read: ${error.message ?? error}.`; }
  if (stored) {
    $('forget').disabled = false;
    try { await loadCartridge(stored.bytes, stored.name, 'stored'); }
    catch (error) { importFailure(error, stored.name ?? 'The saved cartridge'); }
  } else {
    status(`Choose your own cartridge. Graphics: ${game.backend()}.${storageWarning}`, 'empty');
  }
  $('rom').disabled = false;
  requestAnimationFrame(frame);
}

boot().catch(errorMessage);
