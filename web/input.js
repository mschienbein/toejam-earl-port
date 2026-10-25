// Wire order matches crates/tje-web/src/input.rs.
export const ACTIONS = ['up', 'down', 'left', 'right', 'a', 'b', 'c', 'start'];
export const ACTION_LABELS = ['Up', 'Down', 'Left', 'Right', 'A', 'B', 'C', 'Start'];
export const CONTROL_ACTIONS = [...ACTIONS, 'jam'];
export const CONTROL_LABELS = [...ACTION_LABELS, 'Jam modifier'];
const DEFAULT_KEYS = [
  ['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'KeyZ', 'KeyX', 'KeyC', 'Enter'],
  ['KeyW', 'KeyS', 'KeyA', 'KeyD', 'KeyJ', 'KeyK', 'KeyL', 'ShiftRight'],
];
const RESERVED_KEYS = new Set(['Escape', 'Tab', 'KeyF', 'Unidentified', 'ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'MetaLeft', 'MetaRight']);
const button = index => ({ kind: 'button', index });
const axis = (index, direction) => ({ kind: 'axis', index, direction, center: 0, threshold: 0.35 });

export function validKeyboardCode(code) {
  return typeof code === 'string' && /^[A-Za-z][A-Za-z0-9]{1,31}$/.test(code) && !RESERVED_KEYS.has(code);
}

export function defaultPlayer(player) {
  return {
    keys: { ...Object.fromEntries(ACTIONS.map((action, index) => [action, DEFAULT_KEYS[player][index]])), jam: player === 0 ? 'KeyV' : 'KeyN' },
    gamepad: {
      device: null, // Automatic assignment; explicit choices pin browser id + slot.
      bindings: {
        up: [button(12), axis(1, -1)], down: [button(13), axis(1, 1)],
        left: [button(14), axis(0, -1)], right: [button(15), axis(0, 1)],
        a: [button(0)], b: [button(1)], c: [button(2)], start: [button(9)],
        jam: [button(4)],
      },
    },
  };
}

function normalizeSource(source) {
  if (!Number.isInteger(source?.index) || source.index < 0 || source.index > 255) return null;
  if (source.kind === 'button') return button(source.index);
  if (source.kind !== 'axis' || ![-1, 1].includes(source.direction)) return null;
  return {
    kind: 'axis', index: source.index, direction: source.direction,
    center: Number.isFinite(source.center) ? Math.max(-1, Math.min(1, source.center)) : 0,
    threshold: Number.isFinite(source.threshold) ? Math.max(0.1, Math.min(1, source.threshold)) : 0.5,
  };
}

export function normalizePlayer(value, player) {
  const result = defaultPlayer(player);
  for (const action of CONTROL_ACTIONS) {
    if (validKeyboardCode(value?.keys?.[action])) result.keys[action] = value.keys[action];
    const sources = value?.gamepad?.bindings?.[action];
    if (Array.isArray(sources)) {
      const normalized = sources.slice(0, 2).map(normalizeSource).filter(Boolean);
      if (!sources.length || normalized.length) result.gamepad.bindings[action] = normalized;
    }
  }
  const device = value?.gamepad?.device;
  if (device === 'none') result.gamepad.device = 'none';
  else if (typeof device?.id === 'string' && device.id.length <= 1024 && Number.isInteger(device.index) && device.index >= 0 && device.index <= 255) {
    result.gamepad.device = { id: device.id, index: device.index };
  }
  return result;
}

export function sourcePressed(pad, source) {
  if (source.kind === 'button') {
    const value = pad.buttons[source.index];
    return !!value && (value.pressed || value.value > 0.5);
  }
  const value = pad.axes[source.index];
  return Number.isFinite(value) && (value - source.center) * source.direction > source.threshold;
}

export function gamepadMask(pad, bindings = defaultPlayer(0).gamepad.bindings) {
  if (!pad) return 0;
  let mask = 0;
  for (let bit = 0; bit < ACTIONS.length; bit++) {
    if (bindings[ACTIONS[bit]].some(source => sourcePressed(pad, source))) mask |= 1 << bit;
  }
  return mask;
}

export function selectGamepads(pads, players) {
  const selected = [null, null];
  for (let player = 0; player < 2; player++) {
    const device = players[player].gamepad.device;
    if (!device || device === 'none') continue;
    selected[player] = pads.find(pad => pad.id === device.id && pad.index === device.index && !selected.includes(pad)) ?? null;
  }
  for (let player = 0; player < 2; player++) {
    if (players[player].gamepad.device !== null) continue;
    selected[player] = pads.find(pad => !selected.includes(pad)) ?? null;
  }
  return selected;
}

export function keyLabel(code) {
  return code.replace(/^Key/, '').replace(/^Digit/, '').replace(/([a-z])([A-Z])/g, '$1 $2');
}

export function bindingLabel(sources) {
  return sources.length ? sources.map(source => source.kind === 'button' ? `Button ${source.index}`
    : `Axis ${source.index} ${source.direction > 0 ? '+' : '−'}`).join(' or ') : 'Not assigned';
}

export class Inputs {
  constructor(players, readPads = () => navigator.getGamepads?.() ?? []) {
    this.players = players;
    this.readPads = readPads;
    this.held = new Set();
    this.available = [];
    this.selected = [null, null];
    this.pads = 0;
    this.last = [0, 0, false];
    this.keyEvents = [0, 0];
    this.gamepadError = '';
    window.addEventListener('keydown', event => this.key(event, true));
    window.addEventListener('keyup', event => this.key(event, false));
    window.addEventListener('blur', () => this.clear());
  }

  key(event, down) {
    const editing = event.target instanceof Element && event.target.closest('input,select,textarea,button,summary,a,[contenteditable]');
    if ((editing || event.ctrlKey || event.altKey || event.metaKey) && down) return;
    let bound = false;
    for (let player = 0; player < 2; player++) {
      if (Object.values(this.players[player].keys).includes(event.code)) {
        bound = true;
        if (down && !event.repeat) this.keyEvents[player]++;
      }
    }
    if (!bound) return;
    event.preventDefault();
    if (down) this.held.add(event.code);
    else this.held.delete(event.code); // Releases work even after focus moves.
  }

  clear() {
    this.held.clear();
    this.last[0] = this.last[1] = 0;
    this.last[2] = false;
  }

  jamMask() {
    let mask = 0;
    for (let player = 0; player < 2; player++) {
      if (this.held.has(this.players[player].keys.jam)
          || this.players[player].gamepad.bindings.jam.some(source => this.selected[player] && sourcePressed(this.selected[player], source))) mask |= 1 << player;
    }
    return mask;
  }

  samplePads() {
    this.available.length = 0;
    try {
      for (const pad of this.readPads()) if (pad?.connected) this.available.push(pad);
      this.gamepadError = '';
    } catch (error) {
      this.gamepadError = String(error.message ?? error);
    }
    this.pads = this.available.length;
    this.selected = selectGamepads(this.available, this.players);
  }

  poll(p2Connected) {
    for (let player = 0; player < 2; player++) {
      let mask = gamepadMask(this.selected[player], this.players[player].gamepad.bindings);
      for (let bit = 0; bit < ACTIONS.length; bit++) {
        if (this.held.has(this.players[player].keys[ACTIONS[bit]])) mask |= 1 << bit;
      }
      this.last[player] = mask;
    }
    this.last[2] = p2Connected || !!this.selected[1];
    return this.last;
  }
}
