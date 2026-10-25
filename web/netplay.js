// Opt-in direct WebRTC datagrams. Signaling is copy/paste only; no websocket,
// rendezvous account, telemetry, TURN relay or automatic public STUN request.
export const PUBLIC_STUN = 'stun:stun.l.google.com:19302';
const PREFIX = 'tje-rtc1:';

export function encodeCode(code) {
  return PREFIX + btoa(JSON.stringify(code));
}
export function decodeCode(text, expectedKind) {
  if (typeof text !== 'string' || text.length > 96 * 1024 || !text.trim().startsWith(PREFIX)) throw new Error('Paste a browser tje-rtc1 connection code (native UDP codes are separate).');
  let code;
  try { code = JSON.parse(atob(text.trim().slice(PREFIX.length))); }
  catch { throw new Error('Connection code is truncated or invalid. Copy the complete code again.'); }
  if (!code || code.kind !== expectedKind || !['offer', 'answer'].includes(code.kind)
      || !Array.isArray(code.token) || code.token.length !== 16 || code.token.some(byte => !Number.isInteger(byte) || byte < 0 || byte > 255)
      || !Number.isInteger(code.delay) || code.delay < 0 || code.delay > 8 || code.window !== 12
      || typeof code.sdp !== 'string' || code.sdp.length > 64 * 1024 || !code.sdp.startsWith('v=0\r\n')) throw new Error(`Expected a complete ${expectedKind} connection code.`);
  if (/\btyp relay\b/.test(code.sdp)) throw new Error('Relay candidates are not permitted. Use a direct connection; TURN is not supported.');
  return code;
}
function sameInvitation(a, b) {
  return a.delay === b.delay && a.window === b.window && a.token.every((byte, index) => byte === b.token[index]);
}
async function gathered(pc) {
  if (pc.iceGatheringState === 'complete') return;
  await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => finish(new Error('ICE gathering timed out. Retry on a reachable network, or explicitly enable the free STUN option.')), 15000);
    const changed = () => { if (pc.iceGatheringState === 'complete') finish(); };
    const finish = error => { clearTimeout(timeout); pc.removeEventListener('icegatheringstatechange', changed); if (error) reject(error); else resolve(); };
    pc.addEventListener('icegatheringstatechange', changed);
    changed();
  });
}

export class RtcTransport {
  constructor({ useStun = false, onPacket, onError, onOpen = () => {}, PeerConnection = globalThis.RTCPeerConnection }) {
    if (!PeerConnection) throw new Error('WebRTC data channels are unavailable in this browser.');
    this.pc = new PeerConnection({ iceServers: useStun ? [{ urls: PUBLIC_STUN }] : [] });
    this.onPacket = onPacket;
    this.onError = onError;
    this.onOpen = onOpen;
    this.pc.onconnectionstatechange = () => { if (this.pc.connectionState === 'failed') this.onError(new Error('Direct peer connection failed. No relay fallback exists.')); };
    this.pc.ondatachannel = event => this.attach(event.channel);
  }
  attach(channel) {
    if (this.channel || channel.label !== 'tje-inputs' || channel.ordered || channel.maxRetransmits !== 0) { channel.close(); throw new Error('Unexpected WebRTC data channel.'); }
    this.channel = channel;
    channel.binaryType = 'arraybuffer';
    channel.onopen = this.onOpen;
    channel.onmessage = event => {
      if (!(event.data instanceof ArrayBuffer) || event.data.byteLength > 1400) { this.onError(new Error('Invalid netplay datagram.')); return; }
      try { this.onPacket(new Uint8Array(event.data)); } catch (error) { this.onError(error); }
    };
    channel.onerror = () => this.onError(new Error('WebRTC data channel failed.'));
    channel.onclose = () => { if (!this.closed) this.onError(new Error('Peer disconnected.')); };
  }
  async host({ delay = 2, token = Array.from(crypto.getRandomValues(new Uint8Array(16))) } = {}) {
    this.invitation = { kind: 'offer', token, delay, window: 12 };
    this.attach(this.pc.createDataChannel('tje-inputs', { ordered: false, maxRetransmits: 0 }));
    await this.pc.setLocalDescription(await this.pc.createOffer());
    await gathered(this.pc);
    return encodeCode({ ...this.invitation, sdp: this.pc.localDescription.sdp });
  }
  async join(text) {
    this.invitation = decodeCode(text, 'offer');
    await this.pc.setRemoteDescription({ type: 'offer', sdp: this.invitation.sdp });
    await this.pc.setLocalDescription(await this.pc.createAnswer());
    await gathered(this.pc);
    return encodeCode({ ...this.invitation, kind: 'answer', sdp: this.pc.localDescription.sdp });
  }
  async accept(text) {
    const answer = decodeCode(text, 'answer');
    if (!this.invitation || !sameInvitation(this.invitation, answer)) throw new Error('This answer belongs to another host invitation.');
    await this.pc.setRemoteDescription({ type: 'answer', sdp: answer.sdp });
  }
  send(packet) {
    if (this.channel?.readyState !== 'open' || this.channel.bufferedAmount > 65536) return false;
    this.channel.send(packet);
    return true;
  }
  close() {
    this.closed = true;
    this.channel?.close();
    this.pc.close();
  }
}

export class NetplayControls {
  constructor({ game, begin, stop, fail, available, document: doc = globalThis.document }) {
    this.game = game;
    this.begin = begin;
    this.stop = stop;
    this.fail = fail;
    this.available = available;
    this.$ = id => doc.getElementById(id);
    this.active = false;
    this.started = false;
    this.$('net-host').addEventListener('click', () => { void this.host(); });
    this.$('net-join').addEventListener('click', () => { void this.join(); });
    this.$('net-answer').addEventListener('click', () => { void this.answer(); });
    this.$('net-disconnect').addEventListener('click', () => { this.disconnect(); });
  }
  ui() {
    for (const id of ['net-host', 'net-join', 'net-delay', 'net-stun']) this.$(id).disabled = this.active;
    this.$('net-answer').disabled = !this.active || !this.hosting;
    this.$('net-disconnect').disabled = !this.started;
    this.$('rom').disabled = this.active;
    for (const id of ['forget', 'reset', 'p2']) this.$(id).disabled = this.active || !this.available();
    // Snapshot convenience controls are optional, but are never usable online.
    for (const id of ['save-state', 'load-state', 'rewind-state', 'fast-forward', 'state-slot', 'enable-save-states', 'enable-rewind', 'enable-fast-forward']) {
      const control = this.$(id);
      if (this.active && control) control.disabled = true;
    }
  }
  async prepare(invitation, player) {
    if (!this.available()) throw new Error('Import your cartridge before connecting.');
    this.active = true;
    this.hosting = player === 0;
    this.ui();
    await this.begin({ ...invitation, player });
    this.started = true;
    this.ui();
    this.transport = new RtcTransport({
      useStun: this.$('net-stun').checked,
      onPacket: packet => this.game.netplay_receive(packet, performance.now()),
      onError: error => this.failed(error),
    });
    this.timer = setInterval(() => {
      try {
        this.game.netplay_poll(performance.now());
        this.transport.send(this.game.netplay_packet());
        const value = JSON.parse(this.game.netplay_status());
        const stats = value.stats;
        this.$('net-status').textContent = `${value.phase} · frame ${stats.frames} / confirmed ${stats.confirmed} · ${stats.rollbacks} rollbacks / ${stats.resimulated_frames} replayed / max ${stats.largest_rollback} · ${stats.stalls} waits`;
      } catch (error) { this.failed(error); }
    }, 16);
  }
  async host() {
    if (this.active) return;
    try {
      const invitation = { token: Array.from(crypto.getRandomValues(new Uint8Array(16))), delay: Number(this.$('net-delay').value), window: 12 };
      await this.prepare(invitation, 0);
      this.$('net-local').value = await this.transport.host(invitation);
      this.$('net-status').textContent = 'Send this offer code to your peer. Paste their answer below and apply it.';
    } catch (error) { this.failed(error); }
  }
  async join() {
    if (this.active) return;
    try {
      const text = this.$('net-remote').value;
      const invitation = decodeCode(text, 'offer');
      await this.prepare(invitation, 1);
      this.$('net-local').value = await this.transport.join(text);
      this.$('net-status').textContent = 'Send this answer code back to the host. Waiting for direct connection.';
    } catch (error) { this.failed(error); }
  }
  async answer() {
    try { await this.transport.accept(this.$('net-remote').value); }
    catch (error) { this.failed(error); }
  }
  disconnect() {
    clearInterval(this.timer);
    this.transport?.close();
    this.transport = null;
    const wasActive = this.active;
    this.active = this.started = false;
    if (wasActive) this.stop();
    this.$('net-local').value = '';
    this.ui();
    this.$('net-status').textContent = 'Off — local faithful play. Disconnection resets the cartridge.';
  }
  failed(error) {
    this.disconnect();
    this.$('net-status').textContent = `Netplay stopped: ${error.message ?? error}`;
    this.fail(error);
  }
}
