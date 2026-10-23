import { ACTIONS, ACTION_LABELS, bindingLabel, defaultPlayer, keyLabel, sourcePressed, validKeyboardCode } from './input.js';

function element(tag, text, attributes = {}) {
  const value = document.createElement(tag);
  if (text) value.textContent = text;
  for (const [name, content] of Object.entries(attributes)) value.setAttribute(name, content);
  return value;
}

export class ControlSettings {
  constructor(settings, inputs, onChange, onCapture) {
    this.settings = settings;
    this.inputs = inputs;
    this.onChange = onChange;
    this.onCapture = onCapture;
    this.capture = null;
    this.signature = '';
    this.buttons = [];
    this.selects = [];
    this.padStates = [];
    const root = document.getElementById('player-controls');
    for (let player = 0; player < 2; player++) {
      const field = element('fieldset');
      field.append(element('legend', `Player ${player + 1}`));
      const label = element('label', 'Gamepad ', { for: `pad-${player}` });
      const select = element('select', '', { id: `pad-${player}` });
      select.addEventListener('change', () => {
        this.cancel();
        this.settings.players[player].gamepad.device = JSON.parse(select.value);
        this.changed();
      });
      label.append(select);
      field.append(label);
      const padState = element('p', '', { class: 'small' });
      field.append(padState);
      this.selects.push(select);
      this.padStates.push(padState);
      const table = element('table');
      table.append(element('caption', `Player ${player + 1} assignments; activate a binding to change it.`));
      const head = element('thead');
      const headings = element('tr');
      for (const title of ['Action', 'Keyboard', 'Gamepad']) headings.append(element('th', title, { scope: 'col' }));
      head.append(headings);
      table.append(head);
      const body = element('tbody');
      for (let index = 0; index < ACTIONS.length; index++) {
        const action = ACTIONS[index];
        const row = element('tr');
        row.append(element('th', ACTION_LABELS[index], { scope: 'row' }));
        for (const kind of ['keyboard', 'gamepad']) {
          const cell = element('td');
          const button = element('button', '', { type: 'button', 'data-player': player, 'data-action': action, 'data-kind': kind });
          button.addEventListener('click', () => this.begin(kind, player, action, button));
          cell.append(button);
          let clear;
          if (kind === 'gamepad') {
            clear = element('button', 'Clear', { type: 'button', class: 'clear-binding', 'aria-label': `Clear Player ${player + 1} ${ACTION_LABELS[index]} gamepad binding` });
            clear.addEventListener('click', () => {
              this.cancel();
              this.settings.players[player].gamepad.bindings[action] = [];
              this.changed();
            });
            cell.append(clear);
          }
          this.buttons.push({ player, action, kind, button, clear });
          row.append(cell);
        }
        body.append(row);
      }
      table.append(body);
      field.append(table);
      const reset = element('button', `Restore Player ${player + 1} controls`, { type: 'button' });
      reset.addEventListener('click', () => {
        this.cancel();
        this.settings.players[player] = defaultPlayer(player);
        this.changed();
        this.announce(`Player ${player + 1} controls restored.`);
      });
      field.append(reset);
      root.append(field);
    }
    document.getElementById('cancel-mapping').addEventListener('click', () => this.cancel());
    document.addEventListener('keydown', event => this.key(event), true);
    document.addEventListener('keyup', event => { if (event.code.startsWith('Shift')) this.key(event); }, true);
    this.refreshBindings();
  }

  announce(text) {
    document.getElementById('mapping-status').textContent = text;
  }

  changed() {
    this.inputs.clear();
    this.inputs.samplePads();
    this.signature = '';
    this.refreshBindings();
    this.onChange();
  }

  refreshBindings() {
    for (const item of this.buttons) {
      const player = this.settings.players[item.player];
      const text = item.kind === 'keyboard' ? keyLabel(player.keys[item.action]) : bindingLabel(player.gamepad.bindings[item.action]);
      item.button.textContent = text;
      item.button.setAttribute('aria-label', `Player ${item.player + 1} ${item.action} ${item.kind}: ${text}. Change assignment.`);
      if (item.clear) item.clear.disabled = !player.gamepad.bindings[item.action].length;
    }
  }

  begin(kind, player, action, button) {
    this.cancel(false);
    this.inputs.clear();
    const pad = this.inputs.selected[player];
    if (kind === 'gamepad' && !pad) return;
    this.onCapture();
    this.capture = {
      kind, player, action, button,
      buttons: pad ? pad.buttons.map(value => !!value.pressed || value.value > 0.5) : [],
      axes: pad ? Array.from(pad.axes) : [],
    };
    button.setAttribute('aria-pressed', 'true');
    document.getElementById('cancel-mapping').hidden = false;
    this.announce(`Player ${player + 1} ${action}: ${kind === 'keyboard' ? 'press a key' : 'press a button or move an axis'}. Escape cancels.`);
  }

  finish(message) {
    const button = this.capture?.button;
    button?.removeAttribute('aria-pressed');
    this.capture = null;
    this.pendingShift = null;
    document.getElementById('cancel-mapping').hidden = true;
    this.refreshBindings();
    if (message) this.announce(message);
    button?.focus({ preventScroll: true });
  }

  cancel(announce = true) {
    if (this.capture) this.finish(announce ? 'Assignment cancelled; previous binding kept.' : '');
  }

  key(event) {
    if (!this.capture) return;
    if (event.code === 'Tab') { this.cancel(); return; }
    if (event.type === 'keyup' && event.code !== this.pendingShift) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (event.code === 'Escape') { this.cancel(); return; }
    if (this.capture.kind !== 'keyboard' || event.repeat) return;
    if (event.code.startsWith('Shift') && event.type === 'keydown') {
      this.pendingShift = event.code;
      return; // Bind a lone Shift on release, so Shift+Tab remains navigation.
    }
    this.pendingShift = null;
    if (!validKeyboardCode(event.code) || event.ctrlKey || event.altKey || event.metaKey) {
      this.announce('Choose a game key without Ctrl, Alt or Meta. F is fullscreen; Tab and Escape keep their navigation functions.');
      return;
    }
    const { player, action } = this.capture;
    for (let other = 0; other < 2; other++) {
      for (const candidate of ACTIONS) {
        if (other === player && candidate === action) continue;
        if (this.settings.players[other].keys[candidate] === event.code) {
          this.announce(`${keyLabel(event.code)} already controls Player ${other + 1} ${candidate}. Choose another key or Escape.`);
          return;
        }
      }
    }
    this.settings.players[player].keys[action] = event.code;
    this.changed();
    this.finish(`Player ${player + 1} ${action} assigned to ${keyLabel(event.code)}.`);
  }

  update() {
    const pads = this.inputs.available;
    const signature = JSON.stringify([pads.map(pad => [pad.id, pad.index, pad.mapping]), this.settings.players.map(player => player.gamepad.device), this.inputs.gamepadError]);
    if (signature !== this.signature) {
      this.signature = signature;
      const descriptors = pads.map(pad => ({ pad, device: { id: pad.id, index: pad.index } }));
      for (let player = 0; player < 2; player++) {
        const select = this.selects[player];
        const selected = JSON.stringify(this.settings.players[player].gamepad.device);
        select.replaceChildren(element('option', 'Automatic (first unused pad)', { value: 'null' }), element('option', 'Keyboard only', { value: '"none"' }));
        for (const { pad, device } of descriptors) select.append(element('option', `${pad.id} · device ${pad.index}`, { value: JSON.stringify(device) }));
        if (!Array.from(select.options).some(option => option.value === selected)) {
          select.append(element('option', `${this.settings.players[player].gamepad.device.id} (not connected)`, { value: selected }));
        }
        select.value = selected;
        const pad = this.inputs.selected[player];
        this.padStates[player].textContent = pad ? `Using ${pad.id}; ${pad.mapping === 'standard' ? 'standard layout' : 'non-standard layout — assign buttons/axes below'}.` : 'No gamepad assigned; keyboard remains available.';
      }
      for (const item of this.buttons) if (item.kind === 'gamepad') item.button.disabled = !this.inputs.selected[item.player];
      const status = this.inputs.gamepadError ? `Gamepad API unavailable: ${this.inputs.gamepadError}. Keyboard controls still work.` : `${pads.length} gamepad(s) available. Press a pad button if the browser has not exposed it yet.`;
      document.getElementById('gamepad-status').textContent = status;
    }
    if (this.capture?.kind !== 'gamepad') return;
    const { player, action, buttons, axes } = this.capture;
    const pad = this.inputs.selected[player];
    if (!pad) { this.finish('Gamepad disconnected; previous binding kept.'); return; }
    let source;
    for (let index = 0; index < pad.buttons.length; index++) {
      const pressed = sourcePressed(pad, { kind: 'button', index });
      if (pressed && !buttons[index]) { source = { kind: 'button', index }; break; }
      buttons[index] = pressed;
    }
    if (!source) {
      for (let index = 0; index < pad.axes.length; index++) {
        const center = axes[index] ?? pad.axes[index];
        const delta = pad.axes[index] - center;
        if (Math.abs(delta) > 0.65) { source = { kind: 'axis', index, direction: Math.sign(delta), center, threshold: 0.5 }; break; }
      }
    }
    if (source) {
      this.settings.players[player].gamepad.bindings[action] = [source];
      this.changed();
      this.finish(`Player ${player + 1} ${action} assigned to ${bindingLabel([source])}.`);
    }
  }
}
