import test from 'node:test';
import assert from 'node:assert/strict';
import { decodeCode, encodeCode, NetplayControls, PUBLIC_STUN, RtcTransport } from './netplay.js';

const offer = { kind: 'offer', token: Array.from({ length: 16 }, (_, index) => index), delay: 2, window: 12, sdp: 'v=0\r\ns=-\r\n' };
test('manual codes preserve capability, delay and SDP without game data', () => {
  assert.deepEqual(decodeCode(encodeCode(offer), 'offer'), offer);
  assert.throws(() => decodeCode(encodeCode(offer), 'answer'), /answer/);
  assert.throws(() => decodeCode('tje-udp1:{}', 'offer'), /browser/);
  assert.throws(() => decodeCode('tje-rtc1:not-base64', 'offer'), /invalid/);
  assert.throws(() => decodeCode('x'.repeat(100000), 'offer'));
});
test('relay candidates and malformed configuration are rejected', () => {
  for (const patch of [{ token: [1] }, { delay: -1 }, { delay: 9 }, { window: 99 }, { sdp: 'v=0\r\na=candidate:1 1 UDP 1 1.2.3.4 9999 typ relay\r\n' }]) {
    assert.throws(() => decodeCode(encodeCode({ ...offer, ...patch }), 'offer'));
  }
});
test('default transport contacts no STUN or TURN service; opt-in uses STUN only', () => {
  class ConfigOnlyPeer {
    constructor(config) { this.config = config; }
    close() {}
  }
  for (const useStun of [false, true]) {
    const transport = new RtcTransport({ useStun, PeerConnection: ConfigOnlyPeer, onPacket() {}, onError() {} });
    assert.deepEqual(transport.pc.config.iceServers, useStun ? [{ urls: PUBLIC_STUN }] : []);
    assert(!JSON.stringify(transport.pc.config).includes('turn:'));
    transport.close();
  }
});
test('foreign answers cannot replace the selected host invitation', async () => {
  const transport = Object.create(RtcTransport.prototype);
  transport.invitation = offer;
  await assert.rejects(transport.accept(encodeCode({ ...offer, kind: 'answer', token: Array(16).fill(99) })), /another host/);
});
test('constructing netplay controls is offline and keeps cartridge import usable', () => {
  const nodes = new Map();
  const doc = { getElementById: id => {
    if (!nodes.has(id)) nodes.set(id, { disabled: false, addEventListener() {} });
    return nodes.get(id);
  } };
  const controls = new NetplayControls({ document: doc, available: () => false });
  controls.ui();
  assert.equal(controls.active, false);
  assert.equal(controls.started, false);
  assert.equal(controls.transport, undefined);
  assert.equal(controls.timer, undefined);
  assert.equal(nodes.get('rom').disabled, false);
});
