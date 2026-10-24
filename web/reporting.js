// Local-only playtest reports. No ROM, compiled module, memory/audio dump or
// automatic network transport is accepted by this module.
export const RECORDING_CAPACITY = 216000; // ~1 hour at 60 Hz; 864,000 input bytes.

export class InputRing {
  constructor(revision, capacity = RECORDING_CAPACITY) {
    this.revision = revision;
    this.capacity = capacity;
    this.pads = new Uint16Array(capacity * 2);
    this.reset();
  }
  reset() { this.completed = this.length = this.head = 0; this.pending = null; }
  begin(p1, p2, connected) { this.pending = [0x100 | p1, connected ? 0x100 | p2 : 0]; }
  complete() {
    if (!this.pending) return;
    const index = (this.head + this.length) % this.capacity;
    this.pads[index * 2] = this.pending[0];
    this.pads[index * 2 + 1] = this.pending[1];
    if (this.length === this.capacity) this.head = (this.head + 1) % this.capacity;
    else this.length++;
    this.completed++;
    this.pending = null;
  }
  snapshot() {
    const runs = [];
    const append = pads => {
      const previous = runs.at(-1);
      if (previous && previous.pads[0] === pads[0] && previous.pads[1] === pads[1]) previous.frames++;
      else runs.push({ frames: 1, pads });
    };
    for (let offset = 0; offset < this.length; offset++) {
      const index = ((this.head + offset) % this.capacity) * 2;
      append([this.pads[index], this.pads[index + 1]]);
    }
    if (this.pending) append(this.pending.slice());
    const first = this.completed - this.length;
    return {
      capacity_frames: this.capacity, first_frame: first, completed_frames: this.completed,
      in_flight: this.pending?.slice() ?? null,
      session: first === 0 ? {
        format: 1, revision: this.revision, start: { kind: 'power_on' },
        frames: this.completed + Number(this.pending !== null), tool: 'tje-session/0.2.0',
        description: 'Automatic playtest inputs since power-on; no machine snapshot or ROM data', runs,
      } : null,
      input_tail: first === 0 ? [] : runs,
    };
  }
}

function errorDetails(error) {
  // Never stringify arbitrary rejection objects: they could contain ROM buffers.
  const message = typeof error === 'string' ? error
    : typeof error?.message === 'string' ? error.message : 'Unhandled non-Error failure';
  return { message, backtrace: typeof error?.stack === 'string' ? error.stack : new Error(message).stack ?? null };
}
function openDatabase() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open('tje-playtest-reports', 1);
    request.onupgradeneeded = () => {
      request.result.createObjectStore('reports', { keyPath: 'id' });
      request.result.createObjectStore('meta');
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
    request.onblocked = () => reject(new Error('Report storage is blocked by another tab.'));
  });
}
async function transaction(mode, operation) {
  const database = await openDatabase();
  try {
    return await new Promise((resolve, reject) => {
      const tx = database.transaction(['reports', 'meta'], mode);
      let result;
      try { result = operation(tx.objectStore('reports'), tx.objectStore('meta')); }
      catch (error) { tx.abort(); reject(error); return; }
      tx.oncomplete = () => resolve(typeof result === 'function' ? result() : result);
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error ?? new Error('Report storage transaction aborted.'));
    });
  } finally { database.close(); }
}
export async function persistReport(bundle) {
  await transaction('readwrite', (reports, meta) => {
    reports.put(bundle);
    if (bundle.kind === 'crash') meta.put(bundle.id, 'pending-crash');
  });
}
export async function pendingReport() {
  return transaction('readonly', (reports, meta) => {
    let bundle = null;
    const pending = meta.get('pending-crash');
    pending.onsuccess = () => {
      if (pending.result) {
        const request = reports.get(pending.result);
        request.onsuccess = () => { bundle = request.result ?? null; };
      }
    };
    return () => bundle;
  });
}
export async function dismissPending(id) {
  await transaction('readwrite', (_reports, meta) => {
    const pending = meta.get('pending-crash');
    pending.onsuccess = () => {
      if (pending.result === id) meta.delete('pending-crash');
    };
  });
}

export function issueUrl(bundle) {
  const error = bundle.failure.message;
  const title = `[${bundle.build.frontend}] ${error.split('\n')[0].slice(0, 100)}`;
  const body = `Playtest ${bundle.kind} report \`${bundle.id}\`\n\n`
    + `Build: ${bundle.build.version} / commit ${bundle.build.commit}\nTarget: ${bundle.build.target}\n`
    + `OS: ${bundle.environment.os} / ${bundle.environment.architecture}\nBrowser: ${bundle.environment.browser ?? 'not applicable'}\n`
    + `ROM: ${bundle.rom ? `${bundle.rom.revision}, SHA-1 ${bundle.rom.sha1}` : 'not loaded'}\n`
    + `Last completed frame: ${bundle.engine?.frame ?? 0}\nSource: ${bundle.failure.source}\n\n\`\`\`text\n${error.slice(0, 2000)}\n\`\`\`\n\n${bundle.note.slice(0, 1000)}\n\n`
    + `Nothing has been uploaded. Download \`${bundle.id}.tje-report.json\`, review its note/settings and optional consented game screenshot, then attach it here using GitHub's attachment control (ZIP it first if GitHub does not accept JSON). Never attach your ROM or compiled-code cache. The bundle includes controller-only replay inputs; an overflow is explicitly marked.\n`;
  return `https://github.com/mschienbein/toejam-earl-port/issues/new?${new URLSearchParams({ title, body })}`;
}

export class Reports {
  constructor({ settings, onCrash, onConsent }) {
    this.settings = settings;
    this.onCrash = onCrash;
    this.onConsent = onConsent;
    this.ring = this.rom = this.engine = this.pixels = this.offer = null;
    this.crashed = this.capturing = false;
    this.build = { version: 'unavailable before Wasm initialization', commit: 'unavailable before Wasm initialization', target: 'wasm32-unknown-unknown', frontend: 'web', engine: 'not initialized' };
    this.frontend = { phase: 'loading', paused: true, focused: true, ticks: 0, backend: 'not initialized' };
    try { this.consent = JSON.parse(localStorage.getItem('tje-report-preferences-v1') ?? '{}').include_screenshot === true; }
    catch { this.consent = false; }
    this.bindUi();
    this.installHandlers();
  }
  installHandlers() {
    // The Rust panic hook calls only this cached-JS path. Calling WebGame after a
    // wasm panic would reenter a live wasm-bindgen borrow and lose the original error.
    globalThis.tjeCaptureWasmPanic = message => { void this.crash('wasm_panic', new Error(message)); };
    window.addEventListener('error', event => {
      const error = event.error ?? new Error(event.message ?? 'Page resource failed to load');
      void this.crash('window.onerror', error);
    });
    window.addEventListener('unhandledrejection', event => { void this.crash('unhandledrejection', event.reason); });
    // Workers do not bubble their ErrorEvents to window. Instrument construction
    // so every actual worker is observed, without changing its message protocol.
    if (window.Worker) {
      const NativeWorker = window.Worker;
      const reporter = this;
      window.Worker = class extends NativeWorker {
        constructor(...args) { super(...args); reporter.observeWorker(this); }
      };
    }
  }
  observeWorker(worker) {
    worker.addEventListener('error', event => { void this.crash('worker_error', event.error ?? new Error(event.message || 'Worker failed')); });
    worker.addEventListener('messageerror', () => { void this.crash('worker_messageerror', new Error('Worker message could not be deserialized')); });
  }
  async restore() {
    try { const bundle = await pendingReport(); if (bundle) this.show(bundle, 'Saved crash from the previous visit.'); }
    catch (error) { this.notice(`Saved reports could not be read: ${error.message ?? error}. New reports can still be downloaded.`); }
  }
  start(identity) {
    this.rom = identity;
    this.ring = new InputRing(identity.revision);
    this.engine = this.pixels = null;
    this.crashed = false;
  }
  reset() { this.ring?.reset(); this.engine = this.pixels = null; this.crashed = false; }
  unload() { this.ring = this.rom = this.engine = this.pixels = null; }
  begin(p1, p2, connected) { this.ring?.begin(p1, p2, connected); }
  complete(engine) { this.ring?.complete(); this.engine = engine; }
  rememberFrame(rgba, width, height, frame) {
    if (!this.consent) return;
    this.pixels = { rgba, width, height, frame };
  }
  screenshot() {
    if (!this.consent || !this.pixels) return null;
    const { rgba, width, height, frame } = this.pixels;
    const canvas = document.createElement('canvas');
    canvas.width = width; canvas.height = height;
    const pixels = new Uint8ClampedArray(rgba.buffer, rgba.byteOffset, rgba.byteLength);
    for (let offset = 3; offset < pixels.length; offset += 4) pixels[offset] = 255;
    canvas.getContext('2d').putImageData(new ImageData(pixels, width, height), 0, 0);
    const binary = atob(canvas.toDataURL('image/png').split(',')[1]);
    return { consent: true, frame, width, height, png: Array.from(binary, character => character.charCodeAt(0)) };
  }
  bundle(kind, source, error, note) {
    const now = new Date();
    return {
      format: 1, id: `report-${now.getTime()}-${crypto.randomUUID()}`, created_utc: now.toISOString(), kind,
      build: { ...this.build }, environment: {
        os: navigator.userAgentData?.platform ?? navigator.platform ?? 'unknown',
        architecture: navigator.userAgentData?.architecture ?? 'not exposed by browser', browser: navigator.userAgent,
      },
      rom: this.rom ? { ...this.rom } : null, settings: structuredClone(this.settings),
      failure: { source, ...errorDetails(error) }, note,
      engine: this.engine ? structuredClone(this.engine) : null, frontend: { ...this.frontend },
      recording: this.ring?.snapshot() ?? null, screenshot: this.screenshot(),
    };
  }
  async crash(source, error) {
    if (this.crashed || this.capturing) return;
    this.crashed = true;
    try { this.onCrash?.(); }
    catch (pauseError) { console.error('Could not pause after the primary failure', pauseError); }
    await this.capture('crash', source, error, '');
  }
  async capture(kind, source, error, note) {
    if (this.capturing) return;
    this.capturing = true;
    try {
      const bundle = this.bundle(kind, source, error, note);
      this.show(bundle, 'Saving the report locally…');
      try {
        await persistReport(bundle);
        this.notice('Report saved in this browser’s IndexedDB. Download it to keep a copy; nothing was sent.');
      } catch (storageError) {
        this.notice(`Browser report storage unavailable: ${storageError.message ?? storageError}. Download the bundle now; this in-memory copy will be lost on reload.`);
      }
      return bundle;
    } catch (captureError) {
      // Reporting must never produce an unhandled rejection / recurse.
      this.notice(`Report capture failed: ${captureError.message ?? captureError}. Nothing was sent.`);
    } finally { this.capturing = false; }
  }
  notice(text) { document.getElementById('report-status').textContent = text; }
  show(bundle, text) {
    this.offer = bundle;
    document.getElementById('report-offer').hidden = false;
    document.getElementById('report-summary').textContent = `${bundle.failure.message} — last completed frame ${bundle.engine?.frame ?? 0}. ${bundle.recording?.session ? 'Power-on replay is complete.' : bundle.recording ? 'Input ring overflowed; tail is not a deterministic replay.' : 'No cartridge inputs were captured.'}`;
    this.notice(text);
  }
  bindUi() {
    const $ = id => document.getElementById(id);
    $('report-screenshot').checked = this.consent;
    $('report-screenshot').addEventListener('change', () => {
      this.consent = $('report-screenshot').checked;
      if (!this.consent) this.pixels = null;
      try { localStorage.setItem('tje-report-preferences-v1', JSON.stringify({ include_screenshot: this.consent })); }
      catch (error) { this.notice(`Screenshot choice applies this visit only: ${error.message ?? error}`); }
      this.onConsent?.();
    });
    $('report-bug').addEventListener('click', () => {
      $('bug-form').hidden = !$('bug-form').hidden;
      if (!$('bug-form').hidden) $('report-note').focus();
    });
    $('report-save').addEventListener('click', () => {
      void this.capture('bug', 'report_a_bug', 'User-reported playtest bug', $('report-note').value);
    });
    $('report-download').addEventListener('click', () => {
      if (!this.offer) return;
      // This is the export/submission boundary. A future opt-in upload adapter
      // can consume the same bundle; capture itself has no transport API.
      const url = URL.createObjectURL(new Blob([JSON.stringify(this.offer) + '\n'], { type: 'application/json' }));
      const link = document.createElement('a');
      link.href = url; link.download = `${this.offer.id}.tje-report.json`;
      link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
    });
    $('report-issue').addEventListener('click', () => {
      if (this.offer) window.open(issueUrl(this.offer), '_blank', 'noopener,noreferrer');
    });
    $('report-dismiss').addEventListener('click', () => {
      const id = this.offer?.id;
      if (!id) return;
      void dismissPending(id).then(() => {
        // A late dismissal of an older offer must not hide a new crash.
        if (this.offer?.id === id) { $('report-offer').hidden = true; this.offer = null; }
      }).catch(error => this.notice(`Could not dismiss saved offer: ${error.message ?? error}`));
    });
  }
}
