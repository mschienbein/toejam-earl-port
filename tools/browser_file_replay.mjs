// Private, dependency-free browser proof through an existing local CDP endpoint.
// No application server, Playwright installation, reference CPU in the browser,
// package download, public ROM payload or long-pending page evaluation is used.
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { copyFile, mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { parseArgs } from 'node:util';

export async function hashObservationStream(stream) {
  const hashes = [];
  const header = Buffer.alloc(4);
  let headerBytes = 0, remaining = 0, digest = null, totalBytes = 0;
  for await (const input of stream) {
    const chunk = Buffer.isBuffer(input) ? input : Buffer.from(input);
    let offset = 0;
    while (offset < chunk.length) {
      if (remaining === 0) {
        while (headerBytes < 4 && offset < chunk.length) header[headerBytes++] = chunk[offset++];
        if (headerBytes !== 4) break;
        remaining = header.readUInt32LE(); headerBytes = 0;
        if (remaining === 0 || remaining > 8 * 1024 * 1024) throw new Error('Invalid canonical reference observation length.');
        digest = createHash('sha1');
      }
      const count = Math.min(remaining, chunk.length - offset);
      digest.update(chunk.subarray(offset, offset + count));
      offset += count; remaining -= count; totalBytes += count;
      if (remaining === 0) hashes.push(digest.digest('hex'));
    }
  }
  if (remaining !== 0 || headerBytes !== 0) throw new Error('Truncated canonical reference stream.');
  return { hashes, totalBytes };
}
export function validateRecording(recording) {
  if (recording?.format !== 1 || recording.start?.kind !== 'power_on' || !Number.isSafeInteger(recording.frames)
      || recording.frames <= 0 || !Array.isArray(recording.runs)) throw new Error('Use one faithful power-on controller recording.');
  let frames = 0;
  for (const run of recording.runs) {
    if (!Number.isSafeInteger(run.frames) || run.frames <= 0 || !Array.isArray(run.pads) || run.pads.length !== 2
        || run.pads.some(word => !Number.isInteger(word) || word < 0 || word > 0x1ff || (word !== 0 && !(word & 0x100)))) throw new Error('Invalid faithful controller run.');
    frames += run.frames;
  }
  if (frames !== recording.frames) throw new Error('Controller run extent differs from the recording.');
}
async function referenceHashes(executable, rom, session, signal) {
  const child = spawn(executable, [rom, session], { stdio: ['ignore', 'pipe', 'pipe'], signal });
  let stderr = '';
  child.stderr.on('data', chunk => { stderr = (stderr + chunk.toString()).slice(-65536); });
  const exited = new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('close', (code, termination) => code === 0 ? resolve() : reject(new Error(`Original reference exited ${code ?? termination}: ${stderr}`)));
  });
  // Observe either failure while the other side of the stream is still active.
  exited.catch(() => {});
  try { const result = await hashObservationStream(child.stdout); await exited; return { ...result, stderr }; }
  catch (error) { child.kill(); await exited.catch(() => {}); throw error; }
}
class Cdp {
  constructor(socket) {
    this.socket = socket; this.sequence = 0; this.pending = new Map(); this.events = new Set();
    socket.addEventListener('message', event => {
      const message = JSON.parse(event.data);
      if (message.id) {
        const request = this.pending.get(message.id); if (!request) return;
        this.pending.delete(message.id);
        if (message.error) request.reject(new Error(`${request.method}: ${message.error.message}`)); else request.resolve(message.result);
      } else for (const listener of this.events) listener(message);
    });
    socket.addEventListener('close', () => {
      for (const pending of this.pending.values()) pending.reject(new Error('CDP connection closed.'));
      this.pending.clear();
      for (const listener of this.events) listener({ method: 'proof.connectionClosed' });
    });
  }
  static async connect(endpoint) {
    let address = new URL(endpoint);
    if (!['localhost', '127.0.0.1', '[::1]'].includes(address.hostname) || !['http:', 'ws:'].includes(address.protocol)) throw new Error('CDP must be an explicit existing local endpoint.');
    if (address.protocol === 'http:') {
      const response = await fetch(new URL('/json/version', address));
      if (!response.ok) throw new Error(`CDP endpoint returned HTTP ${response.status}.`);
      address = new URL((await response.json()).webSocketDebuggerUrl);
      if (!['localhost', '127.0.0.1', '[::1]'].includes(address.hostname)) throw new Error('CDP redirected outside the local machine.');
    }
    const socket = new WebSocket(address);
    await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, { once: true }); socket.addEventListener('error', reject, { once: true }); });
    return new Cdp(socket);
  }
  call(method, params = {}, sessionId) {
    const id = ++this.sequence;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject, method });
      this.socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    });
  }
  binding(name, sessionId, signal) {
    return new Promise((resolve, reject) => {
      const cleanup = () => { this.events.delete(listener); signal.removeEventListener('abort', abort); };
      const abort = () => { cleanup(); reject(new Error('Private browser replay deadline expired.')); };
      const listener = message => {
        if (message.method === 'proof.connectionClosed') { cleanup(); reject(new Error('CDP closed before the proof completed.')); }
        else if (message.sessionId === sessionId && message.method === 'Runtime.bindingCalled' && message.params.name === name) {
          cleanup(); resolve(message.params.payload);
        }
      };
      this.events.add(listener); signal.addEventListener('abort', abort, { once: true });
      if (signal.aborted) abort();
    });
  }
  close() { this.socket.close(); }
}
export async function main(argv = process.argv.slice(2)) {
  const { values } = parseArgs({ args: argv, options: {
    cdp: { type: 'string' }, pkg: { type: 'string' }, reference: { type: 'string' },
    rom: { type: 'string' }, session: { type: 'string' }, output: { type: 'string' },
    'deadline-seconds': { type: 'string', default: '1800' },
  } });
  for (const key of ['cdp', 'pkg', 'reference', 'rom', 'session', 'output']) if (!values[key]) throw new Error(`Missing --${key}.`);
  const deadlineSeconds = Number(values['deadline-seconds']);
  if (!Number.isInteger(deadlineSeconds) || deadlineSeconds < 1 || deadlineSeconds > 7200) throw new Error('Deadline must be 1..7200 seconds.');
  const signal = AbortSignal.timeout(deadlineSeconds * 1000);
  const output = resolve(values.output);
  if (!output.includes(`${sep}scratch${sep}`)) throw new Error('Private proof output must stay in an uncommitted scratch directory.');
  const rom = resolve(values.rom), sessionPath = resolve(values.session);
  const recording = JSON.parse(await readFile(sessionPath, 'utf8')); validateRecording(recording);
  await mkdir(dirname(output), { recursive: true });
  await mkdir(output); // Never leave an older success receipt beside a failed rerun.
  const reference = await referenceHashes(resolve(values.reference), rom, sessionPath, signal);
  if (reference.hashes.length !== recording.frames + 1) throw new Error('Original exporter did not cover every recording frame.');
  await writeFile(resolve(output, 'reference.json'), JSON.stringify(reference, null, 2));
  const harness = resolve(output, 'replay.html');
  await copyFile(new URL('./browser_file_replay.html', import.meta.url), harness);
  const url = pathToFileURL(harness);
  url.searchParams.set('pkg', pathToFileURL(resolve(values.pkg) + sep).href);
  const cdp = await Cdp.connect(values.cdp);
  let targetId, sessionId;
  try {
    ({ targetId } = await cdp.call('Target.createTarget', { url: 'about:blank' }));
    ({ sessionId } = await cdp.call('Target.attachToTarget', { targetId, flatten: true }));
    await cdp.call('Runtime.enable', {}, sessionId); await cdp.call('Page.enable', {}, sessionId);
    await cdp.call('Runtime.addBinding', { name: 'tjeReplayReady' }, sessionId);
    await cdp.call('Runtime.addBinding', { name: 'tjeReplayComplete' }, sessionId);
    const ready = cdp.binding('tjeReplayReady', sessionId, signal);
    await cdp.call('Page.navigate', { url: url.href }, sessionId); await ready;
    const { root } = await cdp.call('DOM.getDocument', {}, sessionId);
    const { nodeId } = await cdp.call('DOM.querySelector', { nodeId: root.nodeId, selector: '#rom' }, sessionId);
    await cdp.call('DOM.setFileInputFiles', { nodeId, files: [rom] }, sessionId);
    const completed = cdp.binding('tjeReplayComplete', sessionId, signal);
    const expression = `window.tjeFileReplay.start(${JSON.stringify(recording)},${JSON.stringify(reference.hashes)}); true`;
    const started = await cdp.call('Runtime.evaluate', { expression, returnByValue: true }, sessionId);
    if (started.exceptionDetails) throw new Error(`Replay start failed: ${started.exceptionDetails.text}`);
    const receipt = JSON.parse(await completed);
    if (!receipt.ok) throw new Error(`${receipt.error.message}\n${receipt.error.stack}`);
    if (receipt.result.comparedBytes !== reference.totalBytes) throw new Error('Compared canonical byte extent differs from the original stream.');
    await writeFile(resolve(output, 'result.json'), JSON.stringify({ ...receipt.result, romSha1: createHash('sha1').update(await readFile(rom)).digest('hex'), session: sessionPath }, null, 2));
    const capture = await cdp.call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: true }, sessionId);
    await writeFile(resolve(output, 'final-frame.png'), Buffer.from(capture.data, 'base64'));
    console.log(JSON.stringify({ passed: true, frames: receipt.result.frames, comparedBytes: receipt.result.comparedBytes, finalHash: receipt.result.finalHash, output }));
  } catch (error) {
    await writeFile(resolve(output, 'failure.json'), JSON.stringify({ message: String(error.message ?? error), stack: String(error.stack ?? '') }, null, 2));
    throw error;
  } finally {
    if (targetId) await cdp.call('Target.closeTarget', { targetId }).catch(() => {});
    cdp.close();
  }
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(error); process.exitCode = 1; });
}
