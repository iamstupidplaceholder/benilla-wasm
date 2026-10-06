import init, { decode_sound, StreamDecoder } from "../pkg-audio/benilla_audio.js";
const streams = new Map();
const cancelled = new Set();
let streamPort, encodedBytes=0;
function readStream(id) {
  const stream=streams.get(id);if(!stream)return;
  let block;
  try {
    block=stream.decoder.read();
    const indices=block.indices(),pcm=block.pcm(),ended=block.ended();
    streamPort.postMessage({type:"stream-block",id,indices:indices.buffer,pcm:pcm.buffer,ended},[indices.buffer,pcm.buffer]);
  } catch(error){streamPort.postMessage({type:"stream-error",id,error:String(error)});closeStream(id);}
  finally{block?.free();}
}
function closeStream(id){const stream=streams.get(id);if(stream){encodedBytes-=stream.bytes;stream.decoder.free();streams.delete(id);}cancelled.add(id);if(cancelled.size>64)cancelled.delete(cancelled.values().next().value);}
const boot = init().catch(error => {
  self.postMessage({ type: "fatal", error: `decoder module: ${error}` });
  return null;
});
// One decoder worker serializes jobs; the main adapter bounds its inflight queue.
self.onmessage = async ({ data }) => {
  if(data.type==="stream-connect"){
    streamPort=data.port;streamPort.onmessage=({data:m})=>{
      if(m.type==="stream-credit")readStream(m.id);
      else if(m.type==="stream-close")closeStream(m.id);
    };streamPort.start();return;
  }
  if(data.type==="stream-open"){
    try{
      if(!await boot)return;
      if(cancelled.delete(data.id))return;
      if(streams.size>=8||encodedBytes+data.bytes.byteLength>96*1024*1024)throw Error("encoded stream residency budget reached");
      const decoder=new StreamDecoder(new Uint8Array(data.bytes));
      streams.set(data.id,{decoder,bytes:data.bytes.byteLength});encodedBytes+=data.bytes.byteLength;
      streamPort.postMessage({type:"stream-ready",id:data.id,rate:decoder.sample_rate()});
    }catch(error){streamPort.postMessage({type:"stream-error",id:data.id,error:String(error)});}return;
  }
  if (data.type !== "decode") return;
  let sound;
  try {
    if (!await boot) return;
    sound = decode_sound(new Uint8Array(data.bytes));
    const rate = sound.sample_rate();
    const pcm = sound.pcm();
    self.postMessage({ type: "decoded", path: data.path, rate, pcm: pcm.buffer }, [pcm.buffer]);
  } catch (error) {
    self.postMessage({ type: "decode-failed", path: data.path, error: String(error) });
  } finally { sound?.free(); }
};
