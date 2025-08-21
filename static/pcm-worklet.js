class PCM16Downsampler extends AudioWorkletProcessor {
  constructor() {
    super();
    this.acc = new Int16Array(480 * 2); // ~40ms @24k
    this.accLen = 0;
  }

  pushBlock(int16) {
    const need = int16.length;
    if (this.accLen + need > this.acc.length) {
      const out = this.acc.slice(0, this.accLen);
      this.port.postMessage(out.buffer, [out.buffer]);
      this.accLen = 0;
    }
    this.acc.set(int16, this.accLen);
    this.accLen += need;

    // flush every ≥ ~20ms (480 samples @24k)
    if (this.accLen >= 480) {
      const out = this.acc.slice(0, this.accLen);
      this.port.postMessage(out.buffer, [out.buffer]);
      this.accLen = 0;
    }
  }

  process (inputs) {
    const input = inputs[0];
    if (!input || !input[0]) return true;

    const x = input[0];           // Float32 mono @ sampleRate (usually 48000)
    const r = sampleRate / 24000; // target 24k
    let out;

    if (Math.abs(r - 2) < 0.01) {
      // 48k -> 24k with simple 2-tap LPF
      const n = Math.floor(x.length / 2);
      out = new Int16Array(n);
      let j = 0;
      for (let i = 0; i + 1 < x.length; i += 2) {
        const s = (x[i] + x[i + 1]) * 0.5;
        const c = Math.max(-1, Math.min(1, s));
        out[j++] = c < 0 ? c * 0x8000 : c * 0x7FFF;
      }
    } else {
      const win = Math.max(1, Math.floor(r));
      const n = Math.floor(x.length / win);
      out = new Int16Array(n);
      let j = 0;
      for (let i = 0; i + win <= x.length; i += win) {
        let sum = 0;
        for (let k = 0; k < win; k++) sum += x[i + k];
        const c = Math.max(-1, Math.min(1, sum / win));
        out[j++] = c < 0 ? c * 0x8000 : c * 0x7FFF;
      }
    }

    this.pushBlock(out);
    return true;
  }
}
registerProcessor('pcm16-downsampler', PCM16Downsampler);
