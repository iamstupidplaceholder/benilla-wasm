// Device adapter only: bounded PCM FIFO, never fetch/decode/mix or touch gameplay.
class BenillaOutput extends AudioWorkletProcessor {
  constructor() {
    super();
    this.queue = new Array(8);
    this.read = 0; this.write = 0; this.count = 0; this.offset = 0;
    this.port.onmessage = ({ data }) => {
      if (data.type !== "connect") return;
      this.mix = data.port;
      this.mix.onmessage = ({ data: message }) => {
        if (message.type !== "pcm") return;
        if (this.count === this.queue.length) {
          this.port.postMessage({ type: "fatal", error: "PCM credit overflow" }); return;
        }
        this.queue[this.write] = new Float32Array(message.buffer);
        this.write = (this.write + 1) % this.queue.length;
        ++this.count;
      };
      this.mix.start();
      for (let i = 0; i < this.queue.length; ++i) {
        const buffer = new ArrayBuffer(512 * 2 * 4);
        this.mix.postMessage({ type: "credit", buffer }, [buffer]);
      }
    };
    this.underrun = 0; this.quanta = 0; this.primed = false;
    this.started = false; this.recoveries = 0;
  }
  process(_, outputs) {
    const channels = outputs[0];
    if (!channels?.length) return true;
    if (!this.primed && this.count >= (this.started ? 6 : 4)) {
      this.primed = true; this.started = true;
    }
    const frames = channels[0].length;
    let missing = false;
    for (let i = 0; i < frames; ++i) {
      const pcm = this.primed && this.count ? this.queue[this.read] : null;
      if (pcm) {
        channels[0][i] = pcm[this.offset++];
        if (channels[1]) channels[1][i] = pcm[this.offset++]; else ++this.offset;
        if (this.offset === pcm.length) {
          this.queue[this.read] = null;
          this.read = (this.read + 1) % this.queue.length;
          --this.count; this.offset = 0;
          this.mix.postMessage({ type: "credit", buffer: pcm.buffer }, [pcm.buffer]);
        }
      } else {
        channels[0][i] = 0; if (channels[1]) channels[1][i] = 0;
        if (this.started) missing = true;
      }
    }
    if (missing) ++this.underrun;
    if (missing && this.primed) {
      this.primed = false; ++this.recoveries;
    }
    if (++this.quanta >= 375) {
      this.quanta = 0;
      this.port.postMessage({ type: "health", underruns: this.underrun, recoveries: this.recoveries, bufferedFrames: this.count * 512 - this.offset / 2 });
      this.underrun = 0;
      this.recoveries = 0;
    }
    return true;
  }
}
registerProcessor("benilla-output", BenillaOutput);
