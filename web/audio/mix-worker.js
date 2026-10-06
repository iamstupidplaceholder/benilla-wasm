import init, { AudioEngine } from "../pkg-audio/benilla_audio.js";
const boot = init().catch(error => {
  self.postMessage({ type: "fatal", error: `mixer module: ${error}` });
  return null;
});
let engine, port, epoch = 0;
let streamPort, progressTick=0, mixSeconds=0;
const streams = new Map();
function pumpStreams(){
  if(!engine||!streamPort)return;
  for(const [id,s] of streams){
    if(s.open&&!s.inflight&&!s.ended&&engine.stream_slots(id)>=4096){
      s.inflight=true;
      clearTimeout(s.watchdog);
      s.watchdog=setTimeout(()=>{engine.stop(id);closeStream(id);reply({type:"stream-failed",id,error:"stream decode exceeded 30s"});},30000);
      streamPort.postMessage({type:"stream-credit",id});
    }
  }
}
function closeStream(id){clearTimeout(streams.get(id)?.watchdog);streams.delete(id);streamPort?.postMessage({type:"stream-close",id});}
const pending = new Map();
function reply(message) { self.postMessage(message); }
function failPath(path, error) {
  for (const [id, request] of pending) if (request.path === path) {
    pending.delete(id); reply({ type: "failed", id, epoch: request.epoch, error: String(error) });
  }
}
function play(request) {
  if (request.epoch !== epoch || Date.now() > request.deadline) {
    reply({ type: "dropped", id: request.id, epoch: request.epoch }); return;
  }
  try {
    const admitted=engine.play(request.id, request.path, request.amp ?? request.volume, request.frequency, request.looping,
      request.spatial ?? false, !request.preScaled, request.wet ?? false, request.protected ?? false, request.x ?? 0, request.y ?? 0, request.z ?? 0);
    if(!admitted){reply({type:"dropped",id:request.id,epoch,reason:"voice-capacity"});return;}
    for(const id of engine.take_stolen())reply({type:"stolen",id,epoch});
    for(const command of request.commands||[])engine.voice_command(request.id,command.command,command.volume,command.ms);
    reply({ type: "playing", id: request.id, epoch });
  } catch (error) { reply({ type: "failed", id: request.id, epoch, error: String(error) }); }
}
self.onmessage = async ({ data }) => {
  try {
    if (data.type === "init") {
      if (!await boot) return;
      engine = new AudioEngine(data.rate);
      streamPort=data.streamPort;
      streamPort.onmessage=({data:m})=>{
        const stream=streams.get(m.id);
        if(!stream){streamPort.postMessage({type:"stream-close",id:m.id});return;}
        try{
          if(m.type==="stream-ready"){
            engine.stream_open(m.id,m.rate,stream.volume);stream.open=true;
            for(const cmd of stream.commands)engine.stream_command(m.id,cmd.command,cmd.volume,cmd.ms);
            stream.commands=[];pumpStreams();
          }else if(m.type==="stream-block"){
            clearTimeout(stream.watchdog);
            engine.stream_feed(m.id,new Uint32Array(m.indices),new Float32Array(m.pcm),m.ended);
            stream.inflight=false;stream.ended=m.ended;
            if(!stream.playing){stream.playing=true;reply({type:"stream-state",id:m.id,state:engine.stream_state(m.id),position:0,mixSeconds});}
            pumpStreams();
          }else if(m.type==="stream-error"){throw Error(m.error);}
        }catch(error){engine.stop(m.id);closeStream(m.id);reply({type:"stream-failed",id:m.id,error:String(error),terminal:true});}
      };streamPort.start();
      port = data.port;
      port.onmessage = ({ data: credit }) => {
        if (credit.type !== "credit") return;
        try {
          const pcm = new Float32Array(credit.buffer);
          engine.render(pcm);
          mixSeconds += pcm.length / 2 / data.rate;
          port.postMessage({ type: "pcm", buffer: pcm.buffer }, [pcm.buffer]);
          for (const id of engine.take_ended()) reply({ type: "ended", id, epoch });
          for(const [id,s] of streams){
            if(s.open&&engine.stream_state(id)==="Stopped"){closeStream(id);reply({type:"stream-state",id,state:"Stopped",position:0});}
          }
          pumpStreams();
          if(++progressTick>=10){progressTick=0;for(const [id,s] of streams)if(s.open&&s.playing)reply({type:"stream-state",id,state:engine.stream_state(id),position:engine.stream_position(id),mixSeconds});}
          for (const [id, request] of pending) {
            if (Date.now() > request.deadline) { pending.delete(id); reply({ type: "dropped", id, epoch: request.epoch }); }
          }
        } catch (error) { reply({ type: "fatal", error: String(error) }); }
      };
      port.start();
      reply({ type: "ready" });
    } else if (data.type === "upload") {
      try {
        engine.upload(data.path, data.rate, new Float32Array(data.pcm));
        reply({ type: "resident", path: data.path });
        for (const [id, request] of pending) if (request.path === data.path) { pending.delete(id); play(request); }
      } catch (error) { failPath(data.path, error); }
    } else if (data.type === "play") {
      if (engine.has(data.path)) play(data);
      else if (pending.size < 32) { pending.set(data.id, data); reply({ type: "need", path: data.path }); }
      else reply({ type: "failed", id: data.id, epoch: data.epoch, error: "audio request queue full" });
    } else if (data.type === "decode-failed") {
      failPath(data.path, data.error);
    } else if (data.type === "controls") {
      engine.controls(data.master, data.sfx, data.enabled, data.gate, data.limiter);
    } else if (data.type === "listener") {
      engine.listener(data.x,data.y,data.z,data.qx,data.qy,data.qz,data.qw);
    } else if(data.type==="reverb"){
      engine.reverb(data.feedback,data.damping,data.amp);
    } else if (data.type === "update") {
      for (const update of data.channels) {
        const request = pending.get(update.id);
        if (request) Object.assign(request, update);
        else engine.update(update.id,update.amp,update.x,update.y,update.z);
      }
    } else if (data.type === "stop") {
      const request = pending.get(data.id);
      if (request) { pending.delete(data.id); reply({type:"dropped",id:data.id,epoch:request.epoch}); }
      else engine.stop(data.id);
    } else if (data.type === "reset") {
      epoch = data.epoch;
      pending.clear();
      engine.stop_all();
    } else if(data.type==="stream-reserve"){
      if(streams.size>=8){reply({type:"stream-failed",id:data.id,error:"stream capacity reached"});return;}
      const watchdog=setTimeout(()=>{engine.stop(data.id);closeStream(data.id);reply({type:"stream-failed",id:data.id,error:"stream initialization exceeded 30s"});},30000);
      streams.set(data.id,{volume:data.volume,commands:[],open:false,inflight:false,playing:false,ended:false,watchdog});
    } else if(data.type==="stream-command"){
      const stream=streams.get(data.id);if(!stream)return;
      if(!stream.open&&data.command==="stop"){closeStream(data.id);reply({type:"stream-state",id:data.id,state:"Stopped",position:0});return;}
      if(stream.open)engine.stream_command(data.id,data.command,data.volume,data.ms);
      else if(data.command==="volume")stream.volume=data.volume;
      else {stream.commands=stream.commands.filter(c=>!(["pause","resume"].includes(data.command)&&["pause","resume"].includes(c.command)));stream.commands.push(data);}
    } else if(data.type==="channel-command"){
      const request=pending.get(data.id);
      if(request){
        if(data.command==="stop"){pending.delete(data.id);reply({type:"dropped",id:data.id,epoch:request.epoch});}
        else if(data.command==="volume"){request.amp=data.volume;}
        else{request.commands=(request.commands||[]).filter(c=>c.command!==data.command);request.commands.push(data);}
      }else engine.voice_command(data.id,data.command,data.volume,data.ms);
    }
  } catch (error) { reply({ type: "fatal", error: String(error) }); }
};
