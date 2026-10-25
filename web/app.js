import init, { CompiledCartridge, VideoFilter, WebGame, report_build } from './pkg/tje_web.js';
import { AudioStream } from './audio.js';
import { Inputs } from './input.js';
import { ControlSettings } from './controls.js';
import { cartridgeStore, readSettings, saveSettings } from './storage.js';
import { Reports } from './reporting.js';
import { Conveniences } from './convenience.js';
import { NetplayControls } from './netplay.js';
import { JamControls } from './jam.js';
import { fixedSettings } from './fixed.js';

const $ = id => document.getElementById(id);
const settingsState = readSettings();
const settings = settingsState.settings;
const videoFilters = { sharp: VideoFilter.Sharp, nearest: VideoFilter.Nearest, linear: VideoFilter.Linear };
const displayShaders = { off: 0, smooth: 1, crt: 2, 'crt-curved': 3 };
const audio = new AudioStream();
let canvas = $('canvas');
let game;
let wasm;
let inputs;
let controls;
let conveniences;
let netplay;
let jam;
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
let engineWarning = '';
let compilationStage = '';
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
const debugCrashFrame = Number(new URLSearchParams(location.search).get('debug-crash-frame'));
const reports = new Reports({
  settings,
  onCrash: () => {
    void pause('A playtest failure was captured locally. Download the report below; reload before playing again.');
    $('play').disabled = $('reset').disabled = true;
  },
  onConsent: () => rememberReportFrame(),
});
audio.onFailure = error => { void reports.crash('audio_worklet', error); };

function rememberReportFrame() {
  if (reports.consent && loaded && !reports.crashed) {
    const width = game.report_width();
    const height = game.report_height();
    // Exactly one stable pixel copy; exporting a Rust Vec would allocate/copy
    // once in Wasm and then again at the JS boundary on every consented frame.
    const pixels = new Uint8Array(wasm.memory.buffer, game.report_pixels_ptr(), width * height * 4).slice();
    reports.rememberFrame(pixels, width, height, game.frames());
  }
}

function status(message, nextState = state) {
  state = nextState;
  reports.frontend.phase = state;
  reports.frontend.paused = !running;
  reports.frontend.focused = !document.hidden;
  document.body.dataset.state = state;
  $('status').textContent = message;
}

function pause(message = 'Paused. Click Play to resume.', resumeWhenVisible = false) {
  resumeOnVisible = resumeWhenVisible;
  playbackVersion++;
  if (running) activeSeconds += (performance.now() - activeSince) / 1000;
  running = false;
  inputs?.clear();
  conveniences?.release();
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
  void reports.crash('frontend_error', error);
  status(String(error?.message ?? error), 'error');
}

function importFailure(error, name = 'This file') {
  const detail = String(error?.message ?? error);
  const advice = detail.startsWith('ROM identification failed:') ? 'Use an unmodified, verified REV 02 or REV 00 dump; a wrong game or damaged image will not pass identification.'
    : 'Choose the file again to retry. If browser storage was evicted or corrupted, re-import your verified cartridge.';
  $('rom-error').textContent = `${name} was not loaded. ${detail} ${advice} ${loaded ? 'The previous cartridge and its saved copy were kept.' : 'No cartridge was replaced.'}`;
  $('rom-error').hidden = false;
  status(loaded ? `Import rejected. The previous cartridge ${running ? 'continues playing' : 'is paused and can still play'}.` : 'Import rejected. Choose a verified cartridge.', loaded ? (running ? 'running' : 'paused') : 'empty');
}

function resetMeasurements() {
  activeSeconds = measuredFrames = emulationMs = maxFrameMs = presented = stalls = 0;
  histogram.fill(0);
}

function snapshot() {
  if (reports.crashed) {
    return { state, frames: reports.engine?.frame ?? 0, revision, crashed: true,
      reportId: reports.offer?.id ?? null, settings: structuredClone(settings) };
  }
  const wallSeconds = activeSeconds + (running ? (performance.now() - activeSince) / 1000 : 0);
  let cumulative = 0;
  let p95 = 0;
  for (; p95 < histogram.length; p95++) {
    cumulative += histogram[p95];
    if (cumulative >= measuredFrames * 0.95) break;
  }
  return {
    state, engine: game?.execution_engine(), revision, source, backend: game?.backend(), adapter: game?.adapter(),
    frames: game?.frames() ?? 0, playerLevel: game?.player_level() ?? null,
    compilation: game?.compilation() ?? null, compilationStage,
    targetFps: game?.target_fps() ?? 0, measuredFrames, wallSeconds,
    actualFps: wallSeconds > 0 ? measuredFrames / wallSeconds : 0,
    emulationMs, capacityFps: emulationMs > 0 ? measuredFrames * 1000 / emulationMs : 0,
    maxFrameMs, p95FrameMs: p95, presented, stalls, audio: audio.snapshot(),
    presentationFps: wallSeconds > 0 ? presented / wallSeconds : 0,
    gamepads: inputs?.pads ?? 0, keyEvents: inputs?.keyEvents.slice() ?? [0, 0],
    lastInputs: inputs?.last.slice() ?? [0, 0, false], settings: structuredClone(settings), storageWarning,
    netplay: netplay?.started ? JSON.parse(game.netplay_status()) : null,
    canvas: { width: canvas.width, height: canvas.height },
    secure: isSecureContext, crossOriginIsolated,
  };
}

// Read-only observability for the local Playwright acceptance harness. There is
// no test-only stepping path: tests operate the actual controls and device clock.
Object.defineProperty(window, 'tje', { value: Object.freeze({
  get metrics() { return snapshot(); },
  // The Wasm method is absent from release builds; there is no release trigger.
  get debugCrash() { return typeof game?.debug_crash === 'function' ? () => game.debug_crash() : undefined; },
}) });

function renderVideo() {
  return game.render(canvas.width, canvas.height, settings.pixelShape === 'native', settings.scale === 'integer', videoFilters[settings.filter], displayShaders[settings.displayShader]);
}

function resize() {
  const rect = canvas.getBoundingClientRect();
  const dpr = Math.min(devicePixelRatio || 1, 3);
  const width = Math.max(1, Math.min(4096, Math.round(rect.width * dpr)));
  const height = Math.max(1, Math.min(4096, Math.round(rect.height * dpr)));
  if (canvas.width !== width) canvas.width = width;
  if (canvas.height !== height) canvas.height = height;
  videoDirty = true;
  if (loaded && !running && !reports.crashed) {
    renderVideo();
    rememberReportFrame();
  }
}

function frame(now) {
  requestAnimationFrame(frame);
  try {
    inputs?.samplePads();
    reports.frontend.ticks++;
    controls?.update();
    conveniences?.updateControls();
    if (!running) return;
    if (audio.context.state !== 'running') {
      void pause('Audio was interrupted; trying to resume. Click Play if another gesture is required.', true).then(tryAutomaticResume);
      return;
    }
    const speed = netplay?.active ? { fast: false, rewind: false, changed: false } : conveniences.tick(now, game.target_fps());
    if (speed.changed) audio.begin(!speed.fast && !speed.rewind);
    if (speed.rewind) {
      if (game.rewind()) {
        reports.invalidateTimeline();
        reports.engine = JSON.parse(game.report_state());
        videoDirty = true;
      }
      if (videoDirty) { renderVideo(); videoDirty = false; rememberReportFrame(); }
      return;
    }
    if (!speed.fast) audio.syncClock(now);
    // A long tab/OS stall pauses real time, never skips emulated game logic.
    if (!speed.fast && audio.needed() > audio.context.sampleRate * 0.25) {
      audio.begin(true);
      stalls++;
    }
    let steps = 0;
    while (speed.fast ? steps < speed.budget : audio.needed() > 0 && steps < 6) {
      steps++;
      const [p1, p2, p2Connected] = inputs.poll(settings.p2);
      const started = performance.now();
      if (netplay?.active) {
        if (!game.netplay_step(p1)) break;
      } else {
        reports.begin(p1, p2, p2Connected);
        game.set_jam_input(settings.jam.enabled ? inputs.jamMask() : 0);
        game.step(p1, p2, p2Connected);
        reports.complete(JSON.parse(game.report_state()));
      }
      if (debugCrashFrame > 0 && typeof game.debug_crash === 'function' && game.frames() >= debugCrashFrame) game.debug_crash();
      const elapsed = performance.now() - started;
      emulationMs += elapsed;
      maxFrameMs = Math.max(maxFrameMs, elapsed);
      histogram[Math.min(100, Math.ceil(elapsed))]++;
      measuredFrames++;
      // memory.buffer is looked up after every step: wasm memory can grow.
      if (!speed.fast) audio.push(new Float32Array(wasm.memory.buffer, game.audio_ptr(), game.audio_len()));
    }
    if ((steps > 0 || videoDirty) && renderVideo()) {
      presented++;
      videoDirty = false;
      rememberReportFrame();
    }
    if (now - lastStatus > 500) {
      lastStatus = now;
      const value = snapshot();
      $('metrics').textContent = `${value.backend} · ${value.actualFps.toFixed(1)} / ${value.targetFps.toFixed(2)} fps · `
        + `${value.capacityFps.toFixed(0)} engine fps capacity · ${value.audio.sampleRate} Hz · `
        + `${value.audio.played ?? 0} audio frames played · ${value.gamepads} gamepad(s)`;
    }
  } catch (error) {
    void pause();
    errorMessage(error);
  }
}

function compiling(stage) {
  compilationStage = stage;
  const labels = {
    identify: 'Identifying your cartridge and preparing its private engine. First-time compilation may take a few seconds…',
    cache: 'Verifying the browser’s compiled-code cache. On a first visit the engine is generated from your ROM…',
    generate: 'Generating cycle-exact WebAssembly from your cartridge. All code stays on this machine…',
    ready: 'Compiled engine is ready. Finishing local cartridge setup…',
  };
  $('compile-label').textContent = labels[stage] ?? stage;
  $('compile-state').hidden = false;
  $('drop-zone').setAttribute('aria-busy', 'true');
  status(labels[stage] ?? stage, 'loading');
}

function finishCompilation() {
  $('compile-state').hidden = true;
  $('drop-zone').setAttribute('aria-busy', 'false');
  compilationStage = '';
}

async function loadCartridge(bytes, name, origin) {
  $('play').disabled = $('reset').disabled = $('forget').disabled = true;
  compiling('identify');
  try {
    // Paint a truthful indeterminate status before potentially synchronous IR
    // emission. Cache hits never enter that lazy generation path.
    await new Promise(resolve => setTimeout(resolve, 0));
    const cartridge = await CompiledCartridge.load(new Uint8Array(bytes), audio.context?.sampleRate ?? 48000, compiling);
    cartridge.set_enhanced_debug(new URLSearchParams(location.search).get('enhanced-debug') === '1');
    cartridge.set_extended_view(settings.extendedView);
    jam.configure(cartridge);
    cartridge.set_fixed(settings.fixed.enabled, settings.fixed.fixes);
    revision = game.install(cartridge);
    source = origin;
    loaded = true;
    conveniences.reset();
    conveniences.configure();
    reports.start(JSON.parse(game.report_identity()));
    if (settings.fixed.enabled || settings.jam.enabled || settings.extendedView !== 0) reports.invalidateTimeline();
    reports.engine = JSON.parse(game.report_state());
    resetMeasurements();
    inputs.clear();
    const compilation = game.compilation();
    engineWarning = compilation.storageError ? ` Compiled-code cache unavailable: ${compilation.storageError}. This visit can still play.` : '';
    const code = compilation.cacheHit ? 'verified engine cache loaded'
      : compilation.storageError ? 'engine generated for this visit; code cache unavailable' : 'engine generated and cached locally';
    $('cartridge').textContent = `${revision} · ${name} · ${origin === 'stored' ? 'remembered cartridge' : 'imported locally'} · ${code} · ${game.profile_id()}`;
    resize();
    if (origin === 'stored') {
      status(`${revision} ready from this browser’s storage. Click Play; no file selection is needed.${storageWarning}${engineWarning}`, 'ready');
    } else {
      status(`${revision} compiled. Saving your cartridge privately in this browser…`, 'loading');
    }
  } finally {
    finishCompilation();
    $('play').disabled = $('reset').disabled = !loaded;
    $('forget').disabled = !loaded && origin !== 'stored';
  }
}

async function importFile(file) {
  if (!file || busy) return;
  if (netplay?.active) { status('Disconnect netplay before replacing the cartridge.'); return; }
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
      status(`${revision} ready. Cartridge and engine remembered on this machine. Click Play to start.${engineWarning}`, 'ready');
    } catch (error) {
      storageWarning = ` Storage unavailable: ${error.message ?? error}. The current cartridge can still play.`;
      status(`${revision} ready.${storageWarning}${engineWarning}`, 'ready');
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
  if (!loaded || busy || reports.crashed) return;
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
    status(`${revision} running cycle-exact generated WebAssembly — no 68000 interpreter.${storageWarning}${engineWarning}`, 'running');
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

async function convenienceAction(action) {
  if (!loaded || busy || reports.crashed) return;
  busy = true;
  try {
    if (action === 'load' || action === 'rewind') await pause('Paused for local state restore. Click Play to resume.');
    if (await conveniences.perform(action)) {
      reports.invalidateTimeline();
      reports.engine = JSON.parse(game.report_state());
      videoDirty = true;
      resize();
    }
  } catch (error) {
    conveniences.notice(`Local state action rejected: ${error.message ?? error}. The current machine was kept if restore failed.`);
  } finally { busy = false; conveniences.updateControls(); }
}

$('play').addEventListener('click', () => {
  if (busy || !loaded) return;
  if (running) void pause();
  else void startPlayback();
});

$('reset').addEventListener('click', async () => {
  if (!loaded || busy) return;
  if (netplay?.active) return;
  busy = true;
  try {
    await pause();
    reports.reset();
    game.reset();
    conveniences.reset();
    reports.engine = JSON.parse(game.report_state());
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
  if (netplay?.active) return;
  busy = true;
  try {
    await pause();
    const identity = game.unload();
    loaded = false;
    reports.unload();
    revision = source = '';
    $('play').disabled = $('reset').disabled = true;
    await CompiledCartridge.forget_cached(identity);
    await cartridgeStore('delete');
    // The live compiled instance was dropped before cache deletion. Reload is
    // now an empty session; neither cartridge bytes nor generated code remain.
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
  settings.pixelShape = $('pixel-shape').value;
  settings.scale = $('scale').value;
  settings.filter = $('filter').value;
  settings.displayShader = $('display-shader').value;
  settings.p2 = $('p2').checked;
  $('volume-value').value = `${$('volume').value}%`;
  $('volume').setAttribute('aria-valuetext', `${$('volume').value} percent`);
  canvas.classList.toggle('smooth-filter', settings.filter !== 'nearest' || settings.displayShader !== 'off');
  audio.setVolume(settings.volume);
  persistSettings();
  resize();
}
$('settings-notice').textContent = settingsState.notice;
$('volume').value = String(settings.volume * 100);
$('volume-value').value = `${$('volume').value}%`;
$('volume').setAttribute('aria-valuetext', `${$('volume').value} percent`);
$('pixel-shape').value = settings.pixelShape;
$('scale').value = settings.scale;
$('filter').value = settings.filter;
$('display-shader').value = settings.displayShader;
$('p2').checked = settings.p2;
$('extended-view').value = String(settings.extendedView);
$('extended-view').addEventListener('input', () => {
  settings.extendedView = Number($('extended-view').value);
  persistSettings();
  $('extended-view-notice').textContent = 'World width saved. Re-import your ROM or reload this page to apply; the current session is unchanged.';
});
fixedSettings(settings, persistSettings);
canvas.classList.toggle('smooth-filter', settings.filter !== 'nearest' || settings.displayShader !== 'off');
for (const id of ['volume', 'pixel-shape', 'scale', 'filter', 'display-shader', 'p2']) $(id).addEventListener('input', updateSettings);

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
  reports.build = JSON.parse(report_build());
  await reports.restore();
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
  for (const option of $('extended-view').options) {
    if (Number(option.value) !== 0) option.disabled = !game.has_enhanced_mode();
  }
  if (!game.has_enhanced_mode()) $('extended-view-notice').textContent = 'This build is faithful-only. Extended view requires TJE_ENHANCED=1.';
  reports.frontend.backend = game.backend();
  inputs = new Inputs(settings.players, undefined, game.has_enhanced_mode() ? settings.enhancedPlayers : null);
  conveniences = new Conveniences(settings, {
    command: action => { void convenienceAction(action); },
    persist: persistSettings,
    available: () => loaded && !busy && !reports.crashed && !netplay?.active,
    game: () => game,
  });
  controls = new ControlSettings(settings, inputs, persistSettings, () => { void pause('Paused for control assignment. Click Play when finished.'); });
  jam = new JamControls(settings, {
    game: () => game, loaded: () => loaded,
    allowed: () => !busy && !reports.crashed && !netplay?.active,
    pause, persist: persistSettings,
    changed: reset => {
      if (reset) { reports.reset(); conveniences.reset(); resetMeasurements(); }
      reports.invalidateTimeline();
    },
  });
  netplay = new NetplayControls({
    game,
    available: () => loaded && !busy && !reports.crashed,
    begin: async ({ player, token, delay, window: rollbackWindow }) => {
      await pause('Preparing direct peer connection…');
      const sampleRate = await audio.start(settings.volume);
      game.set_sample_rate(sampleRate);
      conveniences.reset();
      game.set_rewind(false);
      game.netplay_start(player, new Uint8Array(token), delay, rollbackWindow, performance.now());
      // Predicted inputs can be revised; do not label them a replayable recording.
      reports.unload();
      resetMeasurements();
      activeSince = performance.now();
      running = true;
      $('play').textContent = 'Pause';
      $('play').setAttribute('aria-pressed', 'true');
      status('Netplay connecting — exchange the connection codes below.', 'running');
    },
    stop: () => {
      void pause('Netplay disconnected; cartridge reset. Click Play for local play.');
      game.netplay_stop();
      conveniences.configure();
      reports.start(JSON.parse(game.report_identity()));
      reports.engine = JSON.parse(game.report_state());
      resetMeasurements();
    },
    fail: error => { status(`Netplay stopped: ${error.message ?? error}. Cartridge reset; local play remains available.`, 'paused'); },
  });
  new ResizeObserver(resize).observe($('screen'));
  status(`Checking private cartridge storage. Graphics: ${game.backend()}.`, 'loading');
  busy = true; // Block a second drop while the remembered cartridge compiles.
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
  busy = false;
  $('rom').disabled = false;
  requestAnimationFrame(frame);
}
$('report-bug').addEventListener('click', () => { void pause('Paused to write a bug report.'); });

boot().catch(errorMessage);
