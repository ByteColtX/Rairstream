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
        let len = self.sinks.len();
        for (index, sink) in self.sinks.iter_mut().enumerate() {
            if index + 1 == len {
                sink.write(chunk.clone())?;
            } else {
                sink.write(chunk.clone())?;
            }
        }

        Ok(())
    }
}
