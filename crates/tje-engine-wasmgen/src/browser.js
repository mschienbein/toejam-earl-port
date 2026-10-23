// Generated modules contain the user's cartridge instructions. Keep them local.
const DATABASE = 'tje-private-engine';
const DATABASE_VERSION = 2;
const STORE = 'modules';
const FORMAT = 2;
const MAX_MODULE_BYTES = 64 * 1024 * 1024;

export function now() { return performance.now(); }

function requireRomSha1(romSha1) {
  if (!/^[0-9a-f]{40}$/.test(romSha1)) {
    throw new TypeError('Engine cache requires a normalized ROM SHA-1.');
  }
}

function keyFor(romSha1, engineVersion, m68kDivider) {
  requireRomSha1(romSha1);
  if (typeof engineVersion !== 'string' || !engineVersion.length) {
    throw new TypeError('Engine cache requires a normalized ROM SHA-1 and engine version.');
  }
  if (!Number.isInteger(m68kDivider) || m68kDivider < 1 || m68kDivider > 0xffffffff) {
    throw new RangeError('Engine cache requires the actual nonzero 68000 master-clock divider.');
  }
  return JSON.stringify([romSha1, engineVersion, m68kDivider]);
}

function database() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DATABASE, DATABASE_VERSION);
    request.onupgradeneeded = () => {
      // V1 stored bare, unverified byte arrays. Never treat them as V2 records.
      if (request.result.objectStoreNames.contains(STORE)) request.result.deleteObjectStore(STORE);
      request.result.createObjectStore(STORE);
    };
    request.onsuccess = () => {
      request.result.onversionchange = () => request.result.close();
      resolve(request.result);
    };
    request.onerror = () => reject(request.error);
    request.onblocked = () => reject(new Error('Engine storage is blocked by another tab.'));
  });
}

async function transaction(mode, action) {
  const db = await database();
  try {
    return await new Promise((resolve, reject) => {
      const tx = db.transaction(STORE, mode);
      const request = action(tx.objectStore(STORE));
      let value;
      request.onsuccess = () => { value = request.result; };
      tx.oncomplete = () => resolve(value);
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error ?? new Error('Engine cache transaction aborted.'));
    });
  } finally {
    db.close();
  }
}

async function sha256(bytes) {
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
  return Array.from(digest, byte => byte.toString(16).padStart(2, '0')).join('');
}

async function boundDigest(romSha1, engineVersion, m68kDivider, bytes) {
  // Hash the large payload separately: no second module-sized concatenation.
  // The outer hash binds that content digest to the complete compile identity.
  const content = await sha256(bytes);
  return sha256(new TextEncoder().encode(JSON.stringify([
    FORMAT, romSha1, engineVersion, m68kDivider, content,
  ])));
}

export async function evictBytes(romSha1, engineVersion, m68kDivider) {
  const key = keyFor(romSha1, engineVersion, m68kDivider);
  await transaction('readwrite', store => store.delete(key));
}

/** Forget every stored engine/configuration for this cartridge, not other ROMs. */
export async function forgetRom(romSha1) {
  requireRomSha1(romSha1);
  await transaction('readwrite', store => {
    const keys = store.getAllKeys();
    // Keep this listener alongside transaction's result-value callback.
    keys.addEventListener('success', () => {
      for (const key of keys.result) {
        if (typeof key !== 'string') continue;
        let identity;
        try { identity = JSON.parse(key); } catch { continue; }
        if (Array.isArray(identity) && identity[0] === romSha1) store.delete(key);
      }
    });
    return keys;
  });
}

/** Verify identity and every module byte before allowing instantiation. */
export async function cachedBytes(romSha1, engineVersion, m68kDivider) {
  const key = keyFor(romSha1, engineVersion, m68kDivider);
  const record = await transaction('readonly', store => store.get(key));
  if (record === undefined) return null;
  const shaped = record?.format === FORMAT && record.romSha1 === romSha1 &&
    record.engineVersion === engineVersion && record.m68kDivider === m68kDivider &&
    record.bytes instanceof Uint8Array && record.bytes.byteLength <= MAX_MODULE_BYTES &&
    typeof record.digest === 'string' && /^[0-9a-f]{64}$/.test(record.digest);
  if (!shaped || record.digest !== await boundDigest(romSha1, engineVersion, m68kDivider, record.bytes)) {
    await evictBytes(romSha1, engineVersion, m68kDivider);
    return null;
  }
  return record.bytes;
}

export async function storeBytes(romSha1, engineVersion, m68kDivider, bytes) {
  const key = keyFor(romSha1, engineVersion, m68kDivider);
  if (!(bytes instanceof Uint8Array) || bytes.byteLength > MAX_MODULE_BYTES) {
    throw new RangeError('Engine module exceeds the 64 MiB browser module limit.');
  }
  const digest = await boundDigest(romSha1, engineVersion, m68kDivider, bytes);
  const record = { format: FORMAT, romSha1, engineVersion, m68kDivider, digest, bytes };
  await transaction('readwrite', store => {
    const put = store.put(record, key);
    const keys = store.getAllKeys();
    keys.onsuccess = () => {
      // Retain distinct compile configurations, but remove obsolete engines.
      for (const oldKey of keys.result) {
        if (typeof oldKey !== 'string') continue;
        let identity;
        try { identity = JSON.parse(oldKey); } catch { continue; }
        if (Array.isArray(identity) && identity[0] === romSha1 && identity[1] !== engineVersion) store.delete(oldKey);
      }
    };
    return put;
  });
}

/** Always use asynchronous compilation, including verified cache hits. */
export async function instantiateBytes(bytes, imports) {
  if (!(bytes instanceof Uint8Array) || bytes.byteLength > MAX_MODULE_BYTES) {
    throw new RangeError('Engine module exceeds the 64 MiB browser module limit.');
  }
  const start = performance.now();
  const { instance } = await WebAssembly.instantiate(bytes, imports);
  return { instance, instantiateMs: performance.now() - start };
}

function moduleError(error) {
  return error instanceof WebAssembly.CompileError || error instanceof WebAssembly.LinkError ||
    error instanceof WebAssembly.RuntimeError;
}

/** An invalid cache is evicted and regenerated once; fresh compilation errors propagate. */
export async function openEngine(romSha1, engineVersion, m68kDivider, imports, generate) {
  keyFor(romSha1, engineVersion, m68kDivider);
  const metrics = { cacheHit: false, cacheReadMs: 0, cacheWriteMs: 0, generationMs: 0,
    instantiateMs: 0, moduleBytes: 0, storageError: null };
  let start = performance.now();
  let bytes;
  try {
    bytes = await cachedBytes(romSha1, engineVersion, m68kDivider);
  } catch (error) {
    metrics.storageError = String(error);
  }
  metrics.cacheReadMs = performance.now() - start;
  if (bytes) {
    try {
      const compiled = await instantiateBytes(bytes, imports);
      metrics.cacheHit = true;
      metrics.instantiateMs = compiled.instantiateMs;
      metrics.moduleBytes = bytes.byteLength;
      return { instance: compiled.instance, metrics };
    } catch (error) {
      if (!moduleError(error)) throw error;
      try { await evictBytes(romSha1, engineVersion, m68kDivider); }
      catch (storageError) { metrics.storageError = String(storageError); }
    }
  }
  start = performance.now();
  bytes = await generate();
  metrics.generationMs = performance.now() - start;
  const compiled = await instantiateBytes(bytes, imports);
  metrics.instantiateMs = compiled.instantiateMs;
  metrics.moduleBytes = bytes.byteLength;
  start = performance.now();
  try {
    await storeBytes(romSha1, engineVersion, m68kDivider, bytes);
  } catch (error) {
    metrics.storageError = String(error);
  }
  metrics.cacheWriteMs = performance.now() - start;
  return { instance: compiled.instance, metrics };
}
