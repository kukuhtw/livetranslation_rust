// pcm-worklet.js
// Capture audio -> PCM16 mono with selectable target sample rate (16k/24k/48k)
// Batch ~120 ms per postMessage (>= 100 ms requirement)

class PCM16Selectable extends AudioWorkletProcessor {
  constructor(options = {}) {
    super();

    // Runtime input sample rate (from AudioWorklet global)
    this.inHz = sampleRate; // typically 48000 in browsers

    // ---- Settings ----
    const po = (options && options.processorOptions) || {};
    this.BATCH_MS = po.batchMs > 0 ? po.batchMs : 120; // >=100 ms recommended
    // Allowed target rates
    this.ALLOWED = new Set([16000, 24000, 48000]);
    const initialHz = this.ALLOWED.has(po.targetHz) ? po.targetHz : 48000;

    // State
    this._carryMono = [];   // leftover mono floats for decimation
    this.acc = null;        // Int16 output accumulator
    this.accLen = 0;
    this.mode = 'passthrough'; // 'passthrough' | 'decimate2' | 'decimate3'
    this.outHz = this.inHz;
    this.batchSamples = 0;

    this._configureTarget(initialHz);

    // Control messages
    this.port.onmessage = (e) => {
      const msg = e.data || {};
      if (msg.type === 'flush' && this.accLen > 0) {
        this._emitChunk();
      } else if (msg.type === 'set-target' && this.ALLOWED.has(msg.hz)) {
        this._configureTarget(msg.hz);
      }
    };
  }

  _configureTarget(targetHz) {
    // Choose mode based on input rate and desired target
    if (targetHz === this.inHz) {
      this.mode = 'passthrough';
      this.outHz = targetHz;
    } else if (this.inHz === 48000 && targetHz === 24000) {
      this.mode = 'decimate2'; // 2:1 average
      this.outHz = 24000;
    } else if (this.inHz === 48000 && targetHz === 16000) {
      this.mode = 'decimate3'; // 3:1 average
      this.outHz = 16000;
    } else {
      // Fallback: unsupported ratio -> passthrough and warn
      this.mode = 'passthrough';
      this.outHz = this.inHz;
      this.port.postMessage({
        type: 'warn',
        message:
          `Unsupported resample ratio (in=${this.inHz} -> target=${targetHz}). ` +
          `Falling back to passthrough @ ${this.inHz} Hz.`,
      });
    }

    // Rebuild accumulator to batch >= BATCH_MS at outHz
    this.batchSamples = Math.max(1, Math.round(this.outHz * this.BATCH_MS / 1000));
    this.acc = new Int16Array(this.batchSamples);
    this.accLen = 0;
    this._carryMono.length = 0;

    // Notify config
    this.port.postMessage({
      type: 'config',
      inHz: this.inHz,
      outHz: this.outHz,
      mode: this.mode,
      batchMs: this.BATCH_MS,
      batchSamples: this.batchSamples,
    });
  }

  _toI16(x) {
    // Clip to [-1, 1] then convert float -> int16
    const y = x < -1 ? -1 : (x > 1 ? 1 : x);
    return (y < 0 ? y * 0x8000 : y * 0x7fff) | 0;
  }

  _pushI16(i16) {
    this.acc[this.accLen++] = i16;
    if (this.accLen >= this.acc.length) this._emitChunk();
  }

  _emitChunk() {
    this.port.postMessage(
      {
        type: 'pcm',
        sampleRate: this.outHz,
        channels: 1,
        pcm: this.acc.buffer,   // ArrayBuffer (transferred)
        samples: this.accLen,   // valid sample count
        ms: Math.round((this.accLen / this.outHz) * 1000),
      },
      [this.acc.buffer]
    );
    this.acc = new Int16Array(this.batchSamples);
    this.accLen = 0;
  }

  process(inputs /*, outputs, parameters */) {
    const input = inputs[0];
    if (!input || input.length === 0) return true;

    const ch0 = input[0];
    const ch1 = input[1] || null;
    if (!ch0 || ch0.length === 0) return true;

    const n = ch0.length;

    // Build mono with possible carry from prior quantum
    // (small allocations here are fine; n is usually 128)
    const mono = new Float32Array(this._carryMono.length + n);
    let p = 0;

    // Copy carry
    for (let i = 0; i < this._carryMono.length; i++) mono[p++] = this._carryMono[i];

    // Downmix current quantum to mono
    if (ch1) {
      for (let i = 0; i < n; i++) mono[p++] = 0.5 * (ch0[i] + ch1[i]);
    } else {
      // single-channel input
      mono.set(ch0, p);
      p += n;
    }

    // Clear carry (we'll refill based on decimation leftovers)
    this._carryMono.length = 0;

    if (this.mode === 'passthrough') {
      // No resample: output all samples
      for (let i = 0; i < mono.length; i++) this._pushI16(this._toI16(mono[i]));
    } else if (this.mode === 'decimate2') {
      // 2:1 with simple box filter (avg every 2 samples)
      const m = mono.length;
      const stop = m & ~1; // largest even
      for (let i = 0; i < stop; i += 2) {
        const s = 0.5 * (mono[i] + mono[i + 1]);
        this._pushI16(this._toI16(s));
      }
      if (m % 2 === 1) {
        // keep the last unpaired sample for next round
        this._carryMono.push(mono[m - 1]);
      }
    } else if (this.mode === 'decimate3') {
      // 3:1 with simple box filter (avg every 3 samples)
      const m = mono.length;
      const stop = m - (m % 3);
      for (let i = 0; i < stop; i += 3) {
        const s = (mono[i] + mono[i + 1] + mono[i + 2]) / 3;
        this._pushI16(this._toI16(s));
      }
      const rem = m % 3;
      if (rem === 1) {
        this._carryMono.push(mono[m - 1]);
      } else if (rem === 2) {
        this._carryMono.push(mono[m - 2], mono[m - 1]);
      }
    }

    return true;
  }
}

registerProcessor('pcm16-selectable', PCM16Selectable);
