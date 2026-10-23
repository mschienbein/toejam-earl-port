// Wire bits match crates/tje-web/src/input.rs: Up Down Left Right A B C Start.
const KEYS = new Map([
  ['ArrowUp', [0, 1]], ['ArrowDown', [0, 2]], ['ArrowLeft', [0, 4]], ['ArrowRight', [0, 8]],
  ['KeyZ', [0, 16]], ['KeyX', [0, 32]], ['KeyC', [0, 64]], ['Enter', [0, 128]],
  ['KeyW', [1, 1]], ['KeyS', [1, 2]], ['KeyA', [1, 4]], ['KeyD', [1, 8]],
  ['KeyJ', [1, 16]], ['KeyK', [1, 32]], ['KeyL', [1, 64]], ['ShiftRight', [1, 128]],
]);

export function gamepadMask(pad) {
  if (!pad || pad.mapping !== 'standard') return 0;
  const pressed = index => pad.buttons[index]?.pressed || pad.buttons[index]?.value > 0.5;
  let mask = 0;
  if (pressed(12) || pad.axes[1] < -0.35) mask |= 1;
  if (pressed(13) || pad.axes[1] > 0.35) mask |= 2;
  if (pressed(14) || pad.axes[0] < -0.35) mask |= 4;
  if (pressed(15) || pad.axes[0] > 0.35) mask |= 8;
  if (pressed(0)) mask |= 16;
  if (pressed(1)) mask |= 32;
  if (pressed(2)) mask |= 64;
  if (pressed(9)) mask |= 128;
  return mask;
}

export class Inputs {
  constructor() {
    this.keys = [0, 0];
    this.pads = 0;
    this.last = [0, 0, false];
    this.keyEvents = [0, 0];
    window.addEventListener('keydown', event => this.key(event, true));
    window.addEventListener('keyup', event => this.key(event, false));
    window.addEventListener('blur', () => this.clear());
  }

  key(event, down) {
    const binding = KEYS.get(event.code);
    if (!binding) return;
    const editing = event.target instanceof Element && event.target.closest('input,select,button,summary,a');
    // Key releases always clear held bits, even if focus moved to a control.
    if (editing && down) return;
    event.preventDefault();
    const [player, bit] = binding;
    if (down) {
      this.keys[player] |= bit;
      if (!event.repeat) this.keyEvents[player]++;
    } else {
      this.keys[player] &= ~bit;
    }
  }

  clear() {
    this.keys.fill(0);
    this.last[0] = this.last[1] = 0;
    this.last[2] = false;
  }

  poll(p2Connected) {
    let first;
    let second;
    this.pads = 0;
    for (const pad of navigator.getGamepads?.() ?? []) {
      if (!pad?.connected || pad.mapping !== 'standard') continue;
      this.pads++;
      if (!first) first = pad;
      else if (!second) second = pad;
    }
    this.last[0] = this.keys[0] | gamepadMask(first);
    this.last[1] = this.keys[1] | gamepadMask(second);
    this.last[2] = p2Connected || !!second;
    return this.last;
  }
}
