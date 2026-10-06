// Browser lifecycle/transport only. Kit selection and decode/DSP remain Rust.
let context, mix, decoder, streamDecoder, node, boot, initTimer, fatal = false, epoch = 0, workerReady = false;
let state = "awaiting-gesture", lastControls;
const events = [], decoding = new Set(), requests = new Map();
const decodeTimers = new Map();
const updates = new Map();
const streamStates=new Map();
const failedSources=new Map();
let nextStream=0x80000000;
let updateScheduled = false;
function emit(event) {
  if (events.length >= 256) { fatalError("audio event queue overflow"); return; }
  events.push(event);
}
function fatalError(error) {
  if (fatal) return;
  fatal = true; state = "failed";
  clearTimeout(initTimer);
  console.error("Benilla audio:", error);
  mix?.terminate(); decoder?.terminate(); streamDecoder?.terminate();node?.disconnect();
  for(const s of streamStates.values()){s.state="Stopped";s.error=String(error);}
  context?.close().catch((failure) => console.error("Benilla audio close:", failure));
  requests.clear(); decoding.clear();
  for (const timer of decodeTimers.values()) clearTimeout(timer);
  decodeTimers.clear();
  events.length = 0; events.push({ type: "fatal", error: String(error) });
}
async function initialize() {
  try {
    initTimer = setTimeout(() => fatalError("audio initialization exceeded 30 seconds"), 30000);
    mix = new Worker(new URL("./mix-worker.js", import.meta.url), { type: "module" });
    decoder = new Worker(new URL("./decode-worker.js", import.meta.url), { type: "module" });
    streamDecoder = new Worker(new URL("./decode-worker.js", import.meta.url), { type: "module" });
    mix.onerror = decoder.onerror = streamDecoder.onerror = (event) => fatalError(event.message);
    streamDecoder.onmessage=({data})=>{if(data.type==="fatal")fatalError(data.error);};
    const streaming=new MessageChannel();
    streamDecoder.postMessage({type:"stream-connect",port:streaming.port1},[streaming.port1]);
    decoder.onmessage = ({ data }) => {
      if (data.type === "fatal") { fatalError(data.error); return; }
      clearTimeout(decodeTimers.get(data.path)); decodeTimers.delete(data.path);
      decoding.delete(data.path);
      if (data.type === "decoded") { ++audio.decodes; mix.postMessage({ ...data, type: "upload" }, [data.pcm]); }
      else { failedSources.set(data.path,data.error);console.error("Benilla sound decode:", data.path, data.error); mix.postMessage(data); }
    };
    mix.onmessage = ({ data }) => {
      if(data.type==="stream-state") {const s=streamStates.get(data.id);if(s)Object.assign(s,data);return;}
      if(data.type==="stream-failed") {const s=streamStates.get(data.id);if(s){s.state="Stopped";s.error=data.error;if(data.terminal)failedSources.set(s.path,data.error);}console.error("Benilla stream:",data.error);return;}
      if (data.type === "fatal") { fatalError(data.error); return; }
      if (data.type === "ready") {
        clearTimeout(initTimer);
        workerReady = true;
        mix.postMessage({ type: "reset", epoch });
        if (lastControls) mix.postMessage(lastControls);
        return;
      }
      if (data.type === "need") { emit(data); return; }
      if(streamStates.has(data.id)&&["playing","ended","failed","dropped","stolen"].includes(data.type)){
        const s=streamStates.get(data.id);s.state=data.type==="playing"?"Playing":"Stopped";if(data.error)s.error=data.error;
      }
      if (["failed", "ended", "dropped","stolen"].includes(data.type)) requests.delete(data.id);
      if (data.type === "failed") console.error("Benilla sound:", data.error);
      if (data.type === "dropped" && data.reason === "voice-capacity") ++audio.voiceDrops;
      emit(data);
    };
    await context.audioWorklet.addModule(new URL("./output-worklet.js", import.meta.url));
    node = new AudioWorkletNode(context, "benilla-output", { numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [2] });
    node.onprocessorerror = () => fatalError("audio output processor failed");
    node.port.onmessage = ({ data }) => {
      if (data.type === "fatal") fatalError(data.error);
      else if (data.type === "health") {
        audio.health = data;
        if (data.underruns) console.warn("Benilla audio underruns:", data.underruns);
      }
    };
    const channel = new MessageChannel();
    mix.postMessage({ type: "init", rate: context.sampleRate, port: channel.port1, streamPort:streaming.port2 }, [channel.port1,streaming.port2]);
    node.port.postMessage({ type: "connect", port: channel.port2 }, [channel.port2]);
    node.connect(context.destination);
    state = context.state;
  } catch (error) { fatalError(error); }
}
function unlock(event) {
  if (!event.isTrusted || fatal || globalThis.BENILLA_CONFIG?.audio === false) return;
  try {
    if (!globalThis.isSecureContext) { fatalError("browser audio requires HTTPS (localhost is exempt)"); return; }
    if (!context) {
      const Audio = globalThis.AudioContext || globalThis.webkitAudioContext;
      if (!Audio) { fatalError("AudioContext unavailable"); return; }
      context = new Audio({ latencyHint: "interactive" });
      if (!context.audioWorklet) { fatalError("AudioWorklet unavailable in this browser"); return; }
      context.onstatechange = () => { state = context.state; };
    }
    // Must be called synchronously in the trusted gesture, before any await.
    context.resume().catch((error) => {
      state = "awaiting-gesture"; console.error("Benilla audio resume:", error);
    });
    boot ||= initialize();
  } catch (error) { fatalError(error); }
}
document.addEventListener("pointerdown", unlock, true);
document.addEventListener("keydown", unlock, true);

export const audio = {
  voiceDrops: 0,
  decodes: 0,
  health: null,
  status() {
    if (fatal) return "failed";
    if (!context) return "awaiting-gesture";
    if (context.state !== "running") return state === "awaiting-gesture" ? state : context.state;
    return workerReady ? "running" : "initializing";
  },
  takeEvents() { return JSON.stringify(events.splice(0)); },
  play(request) {
    if (this.status() !== "running" || requests.size >= 32 || failedSources.has(request.path)) return false;
    requests.set(request.id, request);
    // A still-owned loop is desired state, not a short-lived click. Its owner
    // cancels it on StopHold/despawn/cull/logout/worldport even while bytes wait.
    mix.postMessage({ type: "play", ...request, epoch, deadline: request.looping ? Infinity : Date.now() + 1500 });
    return true;
  },
  decode(path, bytes) {
    if (fatal || !decoder || decoding.has(path)) return;
    if (decoding.size >= 8) {
      this.fail(path, "decode queue full"); return;
    }
    decoding.add(path);
    decodeTimers.set(path, setTimeout(() => fatalError(`sound decode exceeded 30 seconds: ${path}`), 30000));
    const copy = bytes.slice();
    decoder.postMessage({ type: "decode", path, bytes: copy.buffer }, [copy.buffer]);
  },
  fail(path, error) { if (workerReady) mix.postMessage({ type: "decode-failed", path, error }); },
  controls(controls) {
    lastControls = { type: "controls", ...controls };
    if (workerReady) mix.postMessage(lastControls);
  },
  listener(pose) { if (workerReady) mix.postMessage({type:"listener",...pose}); },
  reverb(preset){if(workerReady)mix.postMessage({type:"reverb",...preset});},
  update(channel) {
    if (!workerReady || !requests.has(channel.id)) return;
    updates.set(channel.id, channel);
    if (!updateScheduled) {
      updateScheduled = true;
      queueMicrotask(() => {
        updateScheduled = false;
        if (workerReady && updates.size) mix.postMessage({type:"update",channels:Array.from(updates.values())});
        updates.clear();
      });
    }
  },
  stop(id) {
    updates.delete(id);
    if (workerReady) mix.postMessage({type:"stop",id});
  },
  openStream(path,volume){
    if(this.status()!=="running"||streamStates.size>=32||failedSources.has(path))return 0;
    const id=++nextStream;
    streamStates.set(id,{state:"Pending",position:0,path});
    mix.postMessage({type:"stream-reserve",id,volume});
    emit({type:"need-stream",id,path});return id;
  },
  openStatic(path,volume,looping){
    const id=++nextStream;
    if(!this.play({id,path,volume,amp:volume,looping,frequency:0,preScaled:true,spatial:false}))return 0;
    streamStates.set(id,{state:"Pending",position:0});return id;
  },
  staticCommand(id,command,volume,ms){
    if(command==="stop")updates.delete(id);
    if(workerReady)mix.postMessage({type:"channel-command",id,command,volume,ms});
  },
  streamBytes(id,bytes){
    const state=streamStates.get(id);if(!state||state.state==="Stopped")return;
    const copy=bytes.slice();streamDecoder.postMessage({type:"stream-open",id,bytes:copy.buffer},[copy.buffer]);
  },
  streamFail(id,error){
    const s=streamStates.get(id);if(s){s.state="Stopped";s.error=error;if(s.path)failedSources.set(s.path,error);}
    mix?.postMessage({type:"stream-command",id,command:"stop",volume:0,ms:0});
  },
  streamCommand(id,command,volume,ms){
    const s=streamStates.get(id);if(!s||s.state==="Stopped")return;
    // Pending is honest: no Playing until prepared PCM is accepted by Kira.
    if(command==="stop"&&s.state==="Pending")s.state="Stopped";
    mix.postMessage({type:"stream-command",id,command,volume,ms});
  },
  streamState(id){return JSON.stringify(streamStates.get(id)||{state:"Stopped",position:0});},
  forgetStream(id){streamStates.delete(id);},
  reset() {
    ++epoch; requests.clear();
    // Music has its original owner lifecycle, independent of the SFX epoch.
    const streamEvents=events.filter(event=>event.type==="need-stream");
    events.length=0;events.push(...streamEvents);updates.clear();
    if (workerReady) mix.postMessage({ type: "reset", epoch });
    return epoch;
  },
};
globalThis.BENILLA_AUDIO = audio;
