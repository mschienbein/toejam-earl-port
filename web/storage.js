const SETTINGS_KEY = 'tje-settings-v1';
const DEFAULTS = Object.freeze({ volume: 0.7, scale: 'integer', p2: false });

export function readSettings() {
  try {
    const value = JSON.parse(localStorage.getItem(SETTINGS_KEY));
    return {
      volume: Number.isFinite(value?.volume) ? Math.max(0, Math.min(1, value.volume)) : DEFAULTS.volume,
      scale: value?.scale === 'fit' ? 'fit' : 'integer',
      p2: value?.p2 === true,
    };
  } catch {
    return { ...DEFAULTS };
  }
}

export function saveSettings(settings) {
  localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
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
