import { defaultPlayer, normalizePlayer } from './input.js';

export const SETTINGS_VERSION = 3;
export const SETTINGS_KEY = 'tje-settings';
const LEGACY_KEY = 'tje-settings-v1';

export function defaultSettings() {
  return { version: SETTINGS_VERSION, volume: 0.7, pixelShape: 'native', scale: 'integer', filter: 'sharp', displayShader: 'off', extendedView: 0, fixed: { enabled: false, fixes: (1 << 20) - 1 }, p2: false, players: [defaultPlayer(0), defaultPlayer(1)], enhancedPlayers: [defaultPlayer(2), defaultPlayer(3)], jam: { enabled: false, sounds: true, soundsWhileBusy: false, chord: false, backing: 0 }, convenience: { saveStates: false, rewind: false, fastForward: false, slot: 0 } };
}

export function normalizeSettings(value) {
  const result = defaultSettings();
  if (Number.isFinite(value?.volume)) result.volume = Math.max(0, Math.min(1, value.volume));
  if (value?.scale === 'fit') result.scale = 'fit';
  if (['native', 'square'].includes(value?.pixelShape)) result.pixelShape = value.pixelShape;
  else if (value && value.version !== SETTINGS_VERSION && ['integer', 'fit'].includes(value.scale)) {
    // Both old browser modes used square pixels. Preserve an explicitly saved
    // choice; an unset mode opts into the new Native + integer default.
    result.pixelShape = 'square';
  }
  if (['sharp', 'nearest', 'linear'].includes(value?.filter)) result.filter = value.filter;
  if (['off', 'smooth', 'crt', 'crt-curved'].includes(value?.displayShader)) result.displayShader = value.displayShader;
  if (Number.isInteger(value?.extendedView) && (value.extendedView === 400 || (value.extendedView >= 408 && value.extendedView <= 512 && value.extendedView % 8 === 0))) result.extendedView = value.extendedView;
  result.p2 = value?.p2 === true;
  result.players = [normalizePlayer(value?.players?.[0], 0), normalizePlayer(value?.players?.[1], 1)];
  result.enhancedPlayers = [normalizePlayer(value?.enhancedPlayers?.[0], 2), normalizePlayer(value?.enhancedPlayers?.[1], 3)];
  for (const key of ['saveStates', 'rewind', 'fastForward']) result.convenience[key] = value?.convenience?.[key] === true;
  if (Number.isInteger(value?.convenience?.slot) && value.convenience.slot >= 0 && value.convenience.slot < 5) result.convenience.slot = value.convenience.slot;
  for (const key of ['enabled', 'soundsWhileBusy', 'chord']) result.jam[key] = value?.jam?.[key] === true;
  result.jam.sounds = value?.jam?.sounds !== false;
  if (Number.isInteger(value?.jam?.backing) && value.jam.backing >= 0 && value.jam.backing <= 6) result.jam.backing = value.jam.backing;
  result.fixed.enabled = value?.fixed?.enabled === true;
  if (Number.isInteger(value?.fixed?.fixes) && value.fixed.fixes >= 0 && value.fixed.fixes <= (1 << 20) - 1) {
    const placement = value.fixed.fixes & 28;
    if (placement === 0 || placement === 28) result.fixed.fixes = value.fixed.fixes;
  }
  return result;
}

export function readSettings(storage) {
  try {
    storage ??= globalThis.localStorage;
    const current = storage.getItem(SETTINGS_KEY);
    const legacy = current === null ? storage.getItem(LEGACY_KEY) : null;
    const value = JSON.parse(current ?? legacy ?? 'null');
    if (value?.version > SETTINGS_VERSION) {
      return { settings: defaultSettings(), persistable: false, notice: 'Settings from a newer app version were left untouched. Changes apply only to this visit.' };
    }
    const settings = normalizeSettings(value);
    if (value && (legacy !== null || value.version !== SETTINGS_VERSION)) {
      try {
        saveSettings(settings, storage);
        return { settings, persistable: true, notice: 'Previous settings migrated; your saved display, volume and control choices were kept.' };
      } catch (error) {
        return { settings, persistable: false, notice: `Previous settings were kept for this visit, but migration could not be saved: ${error.message ?? error}.` };
      }
    }
    return { settings, persistable: true, notice: '' };
  } catch (error) {
    return {
      settings: defaultSettings(), persistable: error instanceof SyntaxError,
      notice: error instanceof SyntaxError ? 'Malformed settings were ignored. New choices will replace them when saved.'
        : `Settings could not be read: ${error.message ?? error}. Changes apply only to this visit.`,
    };
  }
}

export function saveSettings(settings, storage = globalThis.localStorage) {
  storage.setItem(SETTINGS_KEY, JSON.stringify(normalizeSettings(settings)));
  storage.removeItem(LEGACY_KEY); // Only after the replacement was written successfully.
}

function database() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open('tje-private-cartridge', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('rom');
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
    request.onblocked = () => reject(new Error('Cartridge storage is blocked by another tab.'));
  });
}

export async function cartridgeStore(action, value) {
  const db = await database();
  try {
    return await new Promise((resolve, reject) => {
      const transaction = db.transaction('rom', action === 'get' ? 'readonly' : 'readwrite');
      const store = transaction.objectStore('rom');
      const request = action === 'get' ? store.get('cartridge')
        : action === 'put' ? store.put(value, 'cartridge') : store.delete('cartridge');
      let result;
      request.onsuccess = () => { result = request.result; };
      transaction.oncomplete = () => resolve(result);
      transaction.onerror = () => reject(transaction.error);
      transaction.onabort = () => reject(transaction.error ?? new Error('Cartridge storage transaction aborted.'));
    });
  } finally {
    db.close();
  }
}
