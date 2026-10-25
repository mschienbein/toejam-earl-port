import { ACTIONS } from './input.js';

// Jam is an extra action channel. Neither ordinary wire masks nor the eight
// original bindings gain a ninth bit; the Wasm adapter supplies enhanced inputs.
export function validateJamBindings(players) {
  for (let seat = 0; seat < 2; seat++) {
    const key = players[seat].keys.jam;
    if (players.some(player => ACTIONS.some(action => player.keys[action] === key))
        || players[1 - seat].keys.jam === key) throw new Error(`Player ${seat + 1} Jam modifier conflicts with an existing key. Remap it in Controls first.`);
    for (const source of players[seat].gamepad.bindings.jam) {
      if (ACTIONS.some(action => players[seat].gamepad.bindings[action].some(other =>
        other.kind === source.kind && other.index === source.index && other.direction === source.direction))) {
        throw new Error(`Player ${seat + 1} Jam modifier conflicts with an ordinary gamepad action. Remap it in Controls first.`);
      }
    }
  }
}

export class JamControls {
  constructor(settings, { game, loaded, allowed, pause, persist, changed }) {
    this.settings = settings;
    this.game = game;
    this.loaded = loaded;
    this.allowed = allowed;
    this.pause = pause;
    this.persist = persist;
    this.changed = changed;
    this.notice = document.getElementById('jam-notice');
    this.controls = Object.fromEntries(['enabled', 'sounds', 'soundsWhileBusy', 'chord', 'backing'].map(key => [key, document.getElementById(`jam-${key}`)]));
    this.supported = game().has_enhanced_mode();
    for (const control of Object.values(this.controls)) control.addEventListener('change', () => { void this.change(); });
    this.refresh();
  }
  refresh() {
    for (const [key, control] of Object.entries(this.controls)) {
      if (key === 'backing') control.value = this.settings.jam[key];
      else control.checked = this.settings.jam[key];
      control.disabled = key === 'enabled' ? !this.supported && !this.settings.jam.enabled : !this.supported || !this.settings.jam.enabled;
    }
    if (!this.supported) this.notice.textContent = 'This build has no enhanced-mode support. Rebuild with TJE_ENHANCED=1 to use Jam Out.';
  }
  configure(target = this.game()) {
    const jam = this.settings.jam;
    if (jam.enabled) validateJamBindings(this.settings.players);
    const reset = target.set_jam_out(jam.enabled, jam.sounds, jam.soundsWhileBusy, jam.chord, jam.backing);
    if (jam.enabled) this.changed(reset);
  }
  async change() {
    const previous = { ...this.settings.jam };
    try {
      if (!this.allowed()) throw new Error('Disconnect netplay and finish the current operation before changing Jam Out.');
      await this.pause('Paused for Jam Out settings. Click Play to resume.');
      for (const [key, control] of Object.entries(this.controls)) this.settings.jam[key] = key === 'backing' ? Number(control.value) : control.checked;
      if (this.settings.jam.enabled) validateJamBindings(this.settings.players);
      if (this.loaded()) this.configure();
      this.persist();
      this.notice.textContent = 'Jam settings saved. Configuration changes apply at an engine phase boundary; existing gameplay sounds retain priority.';
    } catch (error) {
      Object.assign(this.settings.jam, previous);
      this.notice.textContent = `Jam settings rejected: ${error.message ?? error}`;
    }
    this.refresh();
  }
}
