// Private local state bytes only. This module never uploads or adds states to
// input recordings, generated-code caches, telemetry or playtest reports.
export const SLOT_COUNT = 5;
export function shortcutAvailable(settings, code) {
  return !settings.players.some(player => Object.values(player.keys).includes(code));
}
export async function stateSlot(action, sha1, slot, bytes) {
  if (!/^[a-f0-9]{40}$/.test(sha1) || !Number.isInteger(slot) || slot < 0 || slot >= SLOT_COUNT) throw new Error('Invalid local state slot identity.');
  const db = await new Promise((resolve, reject) => {
    const request = indexedDB.open('tje-private-save-states', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('states');
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
    request.onblocked = () => reject(new Error('Save-state storage is blocked by another tab.'));
  });
  try {
    return await new Promise((resolve, reject) => {
      const transaction = db.transaction('states', action === 'get' ? 'readonly' : 'readwrite');
      const store = transaction.objectStore('states');
      const key = `${sha1}:${slot}`;
      const request = action === 'get' ? store.get(key) : action === 'put' ? store.put(bytes, key) : store.delete(key);
      let result;
      request.onsuccess = () => { result = request.result; };
      transaction.oncomplete = () => resolve(result);
      transaction.onerror = () => reject(transaction.error);
      transaction.onabort = () => reject(transaction.error ?? new Error('Local state transaction aborted.'));
    });
  } finally { db.close(); }
}

export class Conveniences {
  constructor(settings, { command, persist, available, game }) {
    this.settings = settings;
    this.command = command;
    this.persist = persist;
    this.available = available;
    this.game = game;
    this.held = new Set();
    this.speedLatched = false;
    this.mode = 'normal';
    this.fastClock = 0;
    this.$ = id => document.getElementById(id);
    const controls = { 'enable-save-states': 'saveStates', 'enable-rewind': 'rewind', 'enable-fast-forward': 'fastForward' };
    for (const [id, key] of Object.entries(controls)) {
      this.$(id).checked = settings.convenience[key];
      this.$(id).addEventListener('change', () => {
        settings.convenience[key] = this.$(id).checked;
        this.configure();
        persist();
      });
    }
    this.$('state-slot').value = String(settings.convenience.slot);
    this.$('state-slot').addEventListener('change', () => {
      settings.convenience.slot = Number(this.$('state-slot').value);
      persist();
    });
    for (const [id, action] of Object.entries({ 'save-state': 'save', 'load-state': 'load', 'rewind-state': 'rewind', 'fast-forward': 'speed' })) {
      this.$(id).addEventListener('click', () => command(action));
    }
    window.addEventListener('keydown', event => this.key(event, true));
    window.addEventListener('keyup', event => this.key(event, false));
    window.addEventListener('blur', () => this.release());
    this.updateControls();
  }
  key(event, down) {
    if (!down) { this.held.delete(event.code); return; }
    if (!this.available() || document.activeElement?.id !== 'canvas' || event.ctrlKey || event.altKey || event.metaKey || !shortcutAvailable(this.settings, event.code)) return;
    const options = this.settings.convenience;
    const actions = { F2: 'save', F3: 'load', F4: 'slot' };
    if (options.saveStates && actions[event.code]) {
      event.preventDefault();
      if (!event.repeat) this.command(actions[event.code]);
    } else if ((options.rewind && event.code === 'Backspace') || (options.fastForward && event.code === 'Backquote')) {
      event.preventDefault();
      this.held.add(event.code);
    }
  }
  release() { this.held.clear(); this.fastClock = 0; }
  reset() { this.release(); this.speedLatched = false; this.mode = 'normal'; this.updateControls(); }
  configure() {
    this.game()?.set_rewind(this.settings.convenience.rewind);
    if (!this.settings.convenience.fastForward) this.speedLatched = false;
    this.updateControls();
  }
  updateControls() {
    const options = this.settings.convenience;
    const available = this.available();
    this.$('state-slot').disabled = !options.saveStates;
    this.$('save-state').disabled = this.$('load-state').disabled = !available || !options.saveStates;
    this.$('rewind-state').disabled = !available || !options.rewind;
    this.$('fast-forward').disabled = !available || !options.fastForward;
    this.$('fast-forward').setAttribute('aria-pressed', String(this.speedLatched));
  }
  notice(message) { this.$('convenience-notice').textContent = message; }
  // Called under the app's async busy guard, so an import cannot race a load.
  async perform(action) {
    const game = this.game();
    const options = this.settings.convenience;
    if (action === 'speed') {
      if (!options.fastForward) throw new Error('Enable fast-forward first.');
      this.speedLatched = !this.speedLatched;
      this.notice(this.speedLatched ? '4x fast-forward enabled (muted).' : 'Normal speed.');
      this.updateControls();
      return false;
    }
    if (action === 'rewind') {
      if (!options.rewind) throw new Error('Enable rewind first.');
      return game.rewind();
    }
    if (!options.saveStates) throw new Error('Enable local save states first.');
    if (action === 'slot') {
      options.slot = (options.slot + 1) % SLOT_COUNT;
      this.$('state-slot').value = String(options.slot);
      this.persist();
      this.notice(`Selected local slot ${options.slot + 1}.`);
      return false;
    }
    const { sha1 } = JSON.parse(game.report_identity());
    if (action === 'save') {
      await stateSlot('put', sha1, options.slot, game.snapshot());
      this.notice(`Saved local slot ${options.slot + 1}.`);
      return false;
    }
    const bytes = await stateSlot('get', sha1, options.slot);
    if (!bytes) throw new Error(`Local slot ${options.slot + 1} is empty.`);
    if (!(bytes instanceof Uint8Array) || bytes.byteLength > 16 * 1024 * 1024) throw new Error('Local save state is malformed.');
    game.restore(bytes);
    this.reset();
    this.notice(`Loaded local slot ${options.slot + 1}. Power-on input reports resume after Reset.`);
    return true;
  }
  tick(now, fps) {
    const options = this.settings.convenience;
    const rewind = options.rewind && this.held.has('Backspace');
    const fast = !rewind && options.fastForward && (this.speedLatched || this.held.has('Backquote'));
    const mode = rewind ? 'rewind' : fast ? 'fast' : 'normal';
    const changed = mode !== this.mode;
    this.mode = mode;
    let budget = 0;
    if (fast) {
      if (!this.fastClock || changed) this.fastClock = now - 1000 / fps;
      budget = Math.min(24, Math.max(0, Math.floor((now - this.fastClock) * fps * 4 / 1000)));
      this.fastClock = Math.max(now - 100, this.fastClock + budget * 1000 / (fps * 4));
    } else this.fastClock = 0;
    return { rewind, fast, budget, changed };
  }
}
