// Runs on the audio render thread. No SharedArrayBuffer, fetch, or allocation
// proportional to play time. Blocks are recycled to the main thread after use.
class CartridgeAudio extends AudioWorkletProcessor {
  constructor() {
    super();
    this.blocks = new Array(64).fill(null);
    this.head = 0;
    this.tail = 0;
    this.count = 0;
    this.offset = 0;
    this.queued = 0;
    this.epoch = 0;
    this.active = false;
    this.primed = false;
    this.received = 0;
    this.played = 0;
    this.nonzero = 0;
    this.underruns = 0;
    this.dropped = 0;
    this.reportAt = 0;
    this.port.onmessage = ({ data }) => {
      if (data.type === 'flush') {
        while (this.count) this.release();
        this.offset = 0;
        this.queued = 0;
        this.epoch = data.epoch;
        this.active = data.active;
        this.primed = false;
        this.received = this.played = this.nonzero = this.underruns = this.dropped = 0;
        this.reportAt = 0;
      } else if (data.type === 'samples') {
        const frames = data.length / 2;
        if (data.epoch !== this.epoch || !this.active || this.count === this.blocks.length || this.queued + frames > sampleRate / 4) {
          if (data.epoch === this.epoch) this.dropped += frames;
          this.port.postMessage({ type: 'recycle', buffer: data.buffer }, [data.buffer]);
          return;
        }
        this.blocks[this.tail] = new Float32Array(data.buffer, 0, data.length);
        this.tail = (this.tail + 1) % this.blocks.length;
        this.count++;
        this.queued += frames;
        this.received += frames;
      }
    };
  }

  release() {
    const block = this.blocks[this.head];
    this.blocks[this.head] = null;
    this.head = (this.head + 1) % this.blocks.length;
    this.count--;
    this.offset = 0;
    this.port.postMessage({ type: 'recycle', buffer: block.buffer }, [block.buffer]);
  }

  process(_inputs, outputs) {
    const channels = outputs[0];
    if (!channels?.length) return true;
    const left = channels[0];
    const right = channels[1] ?? left;
    if (this.active && !this.primed && this.queued >= sampleRate * 0.04) this.primed = true;
    if (this.active && this.primed) {
      for (let i = 0; i < left.length; i++) {
        if (!this.count) {
          this.underruns += left.length - i;
          break; // Web Audio zero-initializes every output quantum.
        }
        const block = this.blocks[this.head];
        const l = block[this.offset++];
        const r = block[this.offset++];
        left[i] = l;
        right[i] = r;
        this.played++;
        this.queued--;
        if (l !== 0 || r !== 0) this.nonzero++;
        if (this.offset === block.length) this.release();
      }
    }
    if (currentFrame >= this.reportAt) {
      this.reportAt = currentFrame + Math.floor(sampleRate / 4);
      this.port.postMessage({
        type: 'stats', epoch: this.epoch, received: this.received, played: this.played,
        nonzero: this.nonzero, underruns: this.underruns, dropped: this.dropped, queued: this.queued,
      });
    }
    return true;
  }
}

registerProcessor('tje-cartridge-audio', CartridgeAudio);
