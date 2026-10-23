async function resumeContext(context) {
  if (context.state === 'running') return;
  let timer;
  try {
    await Promise.race([
      context.resume(),
      new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('The browser needs another audio gesture. Click Play.')), 1500); }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

export class AudioStream {
  constructor() {
    this.context = null;
    this.node = null;
    this.ready = null;
    this.gain = null;
    this.pool = [];
    this.epoch = 0;
    this.origin = 0;
    this.wallOrigin = 0;
    this.clockOffset = 0;
    this.target = 0;
    this.produced = 0;
    this.sentBlocks = 0;
    this.recycledBlocks = 0;
    this.stats = {};
    this.failure = null;
  }

  async start(volume) {
    if (!this.context) {
      // Begin resume directly in the user's gesture, before loading the worklet.
      this.context = new AudioContext({ sampleRate: 48000, latencyHint: 'interactive' });
      this.ready = (async () => {
        try {
          await Promise.all([
            resumeContext(this.context),
            this.context.audioWorklet.addModule(new URL('./audio-worklet.js', import.meta.url)),
          ]);
          this.node = new AudioWorkletNode(this.context, 'tje-cartridge-audio', {
            numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [2],
          });
          this.node.onprocessorerror = () => { this.failure = 'AudioWorklet processor failed.'; };
          this.node.port.onmessage = ({ data }) => {
            if (data.type === 'recycle') {
              if (this.pool.length < 32) this.pool.push(data.buffer);
              this.recycledBlocks++;
            } else if (data.type === 'stats' && data.epoch === this.epoch) {
              this.stats = data;
            }
          };
          this.gain = this.context.createGain();
          this.node.connect(this.gain).connect(this.context.destination);
        } catch (error) {
          await this.context.close();
          this.context = this.node = this.gain = this.ready = null;
          throw error;
        }
      })();
    }
    await this.ready;
    await resumeContext(this.context);
    if (this.context.state !== 'running') throw new Error(`Audio context is ${this.context.state}; click Play to retry.`);
    this.setVolume(volume);
    this.begin(true);
    return this.context.sampleRate;
  }

  begin(active) {
    this.epoch++;
    this.origin = this.context.currentTime;
    this.wallOrigin = performance.now();
    this.clockOffset = 0;
    this.target = active ? Math.floor(0.065 * this.context.sampleRate) : 0;
    this.produced = 0;
    this.sentBlocks = 0;
    this.stats = {};
    this.node.port.postMessage({ type: 'flush', epoch: this.epoch, active });
  }

  async pause() {
    try { await this.ready; }
    catch { return; } // Initialization already reports its failure to start().
    if (!this.context || this.context.state === 'closed') return;
    this.begin(false);
    await this.context.suspend();
  }

  setVolume(volume) {
    this.gain?.gain.setValueAtTime(volume, this.context.currentTime);
  }

  syncClock(now) {
    if (this.failure) throw new Error(this.failure);
    const wall = (now - this.wallOrigin) / 1000;
    const device = this.context.currentTime - this.origin;
    // currentTime can advance in batches. A slow phase lock tracks device-clock
    // drift without alternating zero/two video frames on a steady 60 Hz display.
    this.clockOffset += Math.max(-0.00025, Math.min(0.00025, (device - wall - this.clockOffset) * 0.02));
    this.target = Math.floor((wall + this.clockOffset + 0.065) * this.context.sampleRate);
  }

  needed() {
    return this.target - this.produced;
  }

  push(samples) {
    if (!samples.length) return;
    let buffer = this.pool.pop();
    if (!buffer || buffer.byteLength < samples.byteLength) {
      // The rounded capacity handles variable numbers of samples per VDP frame.
      buffer = new ArrayBuffer(2 ** Math.ceil(Math.log2(Math.max(4096, samples.byteLength))));
    }
    new Float32Array(buffer, 0, samples.length).set(samples);
    this.node.port.postMessage({ type: 'samples', epoch: this.epoch, length: samples.length, buffer }, [buffer]);
    this.produced += samples.length / 2;
    this.sentBlocks++;
  }

  snapshot() {
    return {
      state: this.context?.state ?? 'not-started', sampleRate: this.context?.sampleRate ?? 0,
      produced: this.produced, sentBlocks: this.sentBlocks, recycledBlocks: this.recycledBlocks,
      ...this.stats,
    };
  }
}
