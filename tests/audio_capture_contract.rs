use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use rairstream::audio::{
    AudioCaptureError, AudioChunk, AudioFormat, AudioSink, CaptureConfig,
    testing::{StopCallCounter, spawn_scripted_capture_worker},
};

#[derive(Debug)]
struct RecordingSink {
    chunks: Arc<Mutex<Vec<AudioChunk>>>,
    notify: mpsc::Sender<usize>,
}

impl AudioSink for RecordingSink {
    fn write(&mut self, chunk: AudioChunk) -> Result<(), AudioCaptureError> {
        let mut chunks = self
            .chunks
            .lock()
            .map_err(|_| AudioCaptureError::InvalidFormat {
                message: String::from("录制缓存已损坏"),
            })?;
        chunks.push(chunk);
        self.notify
            .send(chunks.len())
            .map_err(|_| AudioCaptureError::InvalidFormat {
                message: String::from("测试通知通道已关闭"),
            })?;
        Ok(())
    }
}

#[derive(Debug)]
struct FailingSink {
    notify: mpsc::Sender<()>,
}

impl AudioSink for FailingSink {
    fn write(&mut self, _chunk: AudioChunk) -> Result<(), AudioCaptureError> {
        self.notify
            .send(())
            .map_err(|_| AudioCaptureError::InvalidFormat {
                message: String::from("测试通知通道已关闭"),
            })?;

        Err(AudioCaptureError::InvalidFormat {
            message: String::from("sink rejected chunk"),
        })
    }
}

#[test]
fn test_scripted_capture_worker_writes_chunks_to_sink() {
    let stop_calls = StopCallCounter::default();
    let chunks = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let chunk_a = AudioChunk::new(AudioFormat::default(), vec![1; 4]).unwrap();
    let chunk_b = AudioChunk::new(AudioFormat::default(), vec![2; 8]).unwrap();
    let expected = vec![chunk_a.clone(), chunk_b.clone()];
    let sink = RecordingSink {
        chunks: Arc::clone(&chunks),
        notify: tx,
    };
    let worker = spawn_scripted_capture_worker(
        sink,
        CaptureConfig { wait_timeout_ms: 1 },
        vec![Ok(Some(chunk_a)), Ok(Some(chunk_b)), Ok(None)],
        stop_calls.clone(),
    )
    .expect("scripted capture worker should start");

    assert_eq!(rx.recv_timeout(Duration::from_millis(100)).unwrap(), 1);
    assert_eq!(rx.recv_timeout(Duration::from_millis(100)).unwrap(), 2);
    worker.stop().expect("worker stop should succeed");

    let actual = chunks.lock().unwrap();
    assert_eq!(*actual, expected);
    assert_eq!(stop_calls.count(), 1);
}

#[test]
fn test_scripted_capture_worker_propagates_sink_error() {
    let stop_calls = StopCallCounter::default();
    let (tx, rx) = mpsc::channel();
    let chunk = AudioChunk::new(AudioFormat::default(), vec![7; 4]).unwrap();
    let worker = spawn_scripted_capture_worker(
        FailingSink { notify: tx },
        CaptureConfig { wait_timeout_ms: 1 },
        vec![Ok(Some(chunk))],
        stop_calls.clone(),
    )
    .expect("scripted capture worker should start");

    rx.recv_timeout(Duration::from_millis(100)).unwrap();
    let error = worker
        .stop()
        .expect_err("sink error should propagate on stop");

    assert!(matches!(error, AudioCaptureError::InvalidFormat { .. }));
    assert_eq!(stop_calls.count(), 1);
}
