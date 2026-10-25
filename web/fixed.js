// The bit order is the versioned Fixed v1 manifest in tje-extras::fixed.
export const FIXES = [
  ['B017', 'Bound rank recomputation'], ['B018', 'Finish ineligible health bars'],
  ['B004', 'Bound placement searches (linked B004–B006)'], ['B005', 'Reject failed player placement'], ['B006', 'Check item/telephone placement'],
  ['B008', 'Check Earthling bank exhaustion'], ['B011', 'Free only acquired sprite tiles'],
  ['B014', 'Use the promoted player as speaker'], ['B007', 'Supply the small-item placement flag'], ['B015', 'Supply reunion and collection speakers'],
  ['B001', 'Clamp both skate axes'], ['B002', 'Cap and conserve shared health'],
  ['R001', 'REV 02 falling-skate and pose guards'], ['R002', 'REV 02 whirlwind protection'], ['R003', 'REV 02 prompt sleep gate'],
  ['R004', 'REV 02 map idle reset'], ['R005', 'REV 02 inventory idle reset'], ['R006', 'REV 02 title counter reset'],
  ['R007', 'REV 02 Sega counter reset'], ['R008', 'REV 02 lean motion gate'],
];
export function toggleFix(mask, index, enabled) {
  const bit = 1 << index;
  const selection = bit & 28 ? 28 : bit;
  return enabled ? mask | selection : mask & ~selection;
}
export function fixedSettings(settings, persist) {
  const enabled = document.getElementById('fixed-enabled');
  const fixes = document.getElementById('fixed-fixes');
  const notice = document.getElementById('fixed-notice');
  const refresh = () => {
    enabled.checked = settings.fixed.enabled;
    for (const [index, input] of [...fixes.querySelectorAll('input')].entries()) {
      input.checked = Boolean(settings.fixed.fixes & (1 << index));
      input.disabled = !settings.fixed.enabled;
    }
  };
  const changed = () => {
    persist(); refresh();
    notice.textContent = 'Fixed v1 selection saved for the next ROM import/reload. The current session is unchanged. Disable other enhanced profiles before importing.';
  };
  enabled.addEventListener('change', () => { settings.fixed.enabled = enabled.checked; changed(); });
  for (const [index, [id, title]] of FIXES.entries()) {
    const label = document.createElement('label');
    const input = document.createElement('input'); input.type = 'checkbox'; input.id = `fixed-${id}`;
    input.addEventListener('change', () => { settings.fixed.fixes = toggleFix(settings.fixed.fixes, index, input.checked); changed(); });
    label.append(input, document.createTextNode(` ${id} — ${title}${id.startsWith('R') ? ' (REV 00 only)' : ''}`));
    fixes.append(label);
  }
  refresh();
}
