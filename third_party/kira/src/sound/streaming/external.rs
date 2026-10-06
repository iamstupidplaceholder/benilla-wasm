//! Bounded message-passing host bridge. Reuses the original decode scheduler,
//! transport, StreamingSound, resampler and handle; no browser codec/player.
use std::{sync::{Arc,Mutex},convert::Infallible};
use rtrb::{Consumer,Producer,RingBuffer};
use super::{StreamingSoundData,StreamingSoundHandle,StreamingSoundSettings,command_writers_and_readers};
use super::sound::{Shared,StreamingSound,TimestampedFrame,decode_scheduler::{DecodeScheduler,NextStep}};
use crate::sound::{SoundData,Sound};

pub struct ExternalDecode<Error: Send + 'static> {
    scheduler: DecodeScheduler<Error>, consumer: Consumer<TimestampedFrame>, finished: bool,
}
impl<Error:Send+'static> StreamingSoundData<Error> {
    pub fn external_decode(self) -> Result<ExternalDecode<Error>,Error> {
        let (sound,_handle,scheduler) = self.split()?;
        Ok(ExternalDecode { scheduler,consumer:sound.frame_consumer,finished:false })
    }
}
impl<Error:Send+'static> ExternalDecode<Error> {
    pub fn read(&mut self,max_frames:usize) -> Result<Vec<(usize,crate::Frame)>,Error> {
        let max_frames = max_frames.min(4096);
        while !self.finished && self.consumer.slots() < max_frames {
            match self.scheduler.run()? { NextStep::Continue=>{},NextStep::Wait=>break,NextStep::End=>self.finished=true }
        }
        let count = self.consumer.slots().min(max_frames);
        let mut frames = Vec::with_capacity(count);
        for _ in 0..count { let frame=self.consumer.pop().expect("slots counted");frames.push((frame.index,frame.frame)); }
        Ok(frames)
    }
    pub fn finished(&self)->bool { self.finished && self.consumer.is_empty() }
}

pub struct ExternalStreamData { pub sample_rate:u32, pub settings:StreamingSoundSettings }
pub struct ExternalStreamHandle {
    pub playback:StreamingSoundHandle<Infallible>, producer:Producer<TimestampedFrame>, shared:Arc<Shared>,
}
impl ExternalStreamHandle {
    pub fn slots(&self)->usize { self.producer.slots() }
    pub fn feed(&mut self,indices:&[u32],pcm:&[f32],finished:bool)->Result<(), &'static str> {
        if pcm.len()!=indices.len()*2 || indices.len()>self.producer.slots() {return Err("stream PCM credit overflow");}
        for (index,samples) in indices.iter().zip(pcm.chunks_exact(2)) {
            self.producer.push(TimestampedFrame{index:*index as usize,frame:crate::Frame{left:samples[0],right:samples[1]}}).map_err(|_|"stream PCM credit overflow")?;
        }
        if finished {self.shared.finish_decode();} Ok(())
    }
}
impl SoundData for ExternalStreamData {
    type Error=Infallible;
    type Handle=ExternalStreamHandle;
    fn into_sound(self)->Result<(Box<dyn Sound>,Self::Handle),Infallible> {
        let (writers,readers,_decode_readers)=command_writers_and_readers();
        let (_error_producer,error_consumer)=RingBuffer::new(1);
        let (producer,consumer)=RingBuffer::new(16384);
        let shared=Arc::new(Shared::new());
        let sound=StreamingSound::new(self.sample_rate,self.settings,shared.clone(),consumer,readers,0);
        let playback=StreamingSoundHandle{shared:shared.clone(),command_writers:writers,error_consumer:Mutex::new(error_consumer)};
        Ok((Box::new(sound),ExternalStreamHandle{playback,producer,shared}))
    }
}
