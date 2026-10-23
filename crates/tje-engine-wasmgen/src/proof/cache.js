// Development-only fault injection. Not imported by the production backend.
export async function tamperRecord(romSha1, engineVersion, m68kDivider, mode) {
  const db = await new Promise((resolve, reject) => {
    const request = indexedDB.open('tje-private-engine', 2);
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  try {
    return await new Promise((resolve, reject) => {
      const tx = db.transaction('modules', 'readwrite');
      const store = tx.objectStore('modules');
      const key = JSON.stringify([romSha1, engineVersion, m68kDivider]);
      const request = store.get(key);
      let failure;
      let valid;
      request.onsuccess = () => {
        try {
          const record = request.result;
          if (!record?.bytes) throw new Error('Missing authored cache record.');
          if (mode === 'semantic') {
            let changed = 0;
            for (let index = 0; index + 3 < record.bytes.length; index++) {
              if (record.bytes[index] === 0x41 && record.bytes[index + 1] === 42 &&
                  record.bytes[index + 2] === 0x21 && record.bytes[index + 3] === 3) {
                record.bytes[index + 1] = 43;
                changed++;
              }
            }
            if (changed !== 1) throw new Error(`Expected one authored constant, found ${changed}.`);
          } else if (mode === 'magic') record.bytes[0] ^= 0xff;
          else if (mode === 'identity') record.m68kDivider++;
          else if (mode === 'digest') record.digest = '0'.repeat(64);
          else throw new Error(`Unknown cache fault ${mode}.`);
          valid = WebAssembly.validate(record.bytes);
          if (mode === 'semantic' && !valid) throw new Error('Semantic mutation must remain valid WebAssembly.');
          store.put(record, key);
        } catch (error) { failure = error; tx.abort(); }
      };
      tx.oncomplete = () => resolve(valid);
      tx.onerror = () => reject(failure ?? tx.error);
      tx.onabort = () => reject(failure ?? tx.error);
    });
  } finally { db.close(); }
}
