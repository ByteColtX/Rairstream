use std::sync::{Arc, Mutex};

use rairstream::audio::{AudioCaptureError, AudioChunk, AudioFormat, AudioSink};
use rairstream::session::group::FanoutAudioSink;

#[derive(Clone)]
struct RecordingSink {
    seen: Arc<Mutex<usize>>,
}

impl AudioSink for RecordingSink {
    fn write(&mut self, _chunk: AudioChunk) -> Result<(), AudioCaptureError> {
        let mut seen = self.seen.lock().unwrap();
        *seen += 1;
        Ok(())
    }
}

#[test]
fn fanout_sink_writes_to_each_target() {
    let left = Arc::new(Mutex::new(0));
    let right = Arc::new(Mutex::new(0));
    let mut sink = FanoutAudioSink::new(vec![
        Box::new(RecordingSink {
            seen: Arc::clone(&left),
        }),
        Box::new(RecordingSink {
            seen: Arc::clone(&right),
        }),
    ]);
    let chunk = AudioChunk::new(AudioFormat::default(), vec![0; 4]).unwrap();

    sink.write(chunk).unwrap();

    assert_eq!(*left.lock().unwrap(), 1);
    assert_eq!(*right.lock().unwrap(), 1);
}

#[derive(Clone)]
struct FailingSink {
    seen: Arc<Mutex<usize>>,
}

impl AudioSink for FailingSink {
    fn write(&mut self, _chunk: AudioChunk) -> Result<(), AudioCaptureError> {
        let mut seen = self.seen.lock().unwrap();
        *seen += 1;
        Err(AudioCaptureError::InvalidFormat {
            message: String::from("target rejected chunk"),
        })
    }
}

#[test]
fn fanout_sink_stops_after_the_first_error() {
    let first = Arc::new(Mutex::new(0));
    let failing = Arc::new(Mutex::new(0));
    let skipped = Arc::new(Mutex::new(0));
    let mut sink = FanoutAudioSink::new(vec![
        Box::new(RecordingSink {
            seen: Arc::clone(&first),
        }),
        Box::new(FailingSink {
            seen: Arc::clone(&failing),
        }),
        Box::new(RecordingSink {
            seen: Arc::clone(&skipped),
        }),
    ]);
    let chunk = AudioChunk::new(AudioFormat::default(), vec![0; 4]).unwrap();

    let error = sink.write(chunk).unwrap_err();

    assert!(matches!(error, AudioCaptureError::InvalidFormat { .. }));
    assert_eq!(*first.lock().unwrap(), 1);
    assert_eq!(*failing.lock().unwrap(), 1);
    assert_eq!(*skipped.lock().unwrap(), 0);
}
