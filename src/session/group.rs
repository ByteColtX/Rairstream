use crate::audio::{AudioCaptureError, AudioChunk, AudioSink};

pub struct FanoutAudioSink {
    sinks: Vec<Box<dyn AudioSink + Send>>,
}

impl FanoutAudioSink {
    #[must_use]
    pub fn new(sinks: Vec<Box<dyn AudioSink + Send>>) -> Self {
        Self { sinks }
    }
}

impl AudioSink for FanoutAudioSink {
    fn write(&mut self, chunk: AudioChunk) -> Result<(), AudioCaptureError> {
        for sink in &mut self.sinks {
            sink.write(chunk.clone())?;
        }

        Ok(())
    }
}
