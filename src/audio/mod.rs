//! 音频采集层：对上层暴露统一的输入抽象。

mod convert;
mod decode;
mod raop;

#[cfg(target_os = "windows")]
mod windows;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};

pub use decode::FileChunkDecoder;
pub use raop::AudioResampler;
#[cfg(test)]
pub(crate) use raop::RAOP_STARTUP_LATENCY_FRAMES;
pub(crate) use raop::{
    CodecDescription, RAOP_FRAMES_PER_PACKET, RAOP_SAMPLE_RATE_HZ, RAOP_STARTUP_LATENCY_MILLIS,
};

/// PCM 样本的数据语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSampleType {
    Int,
    Float,
}

/// 音频流基础格式信息。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub sample_type: AudioSampleType,
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self {
            sample_rate_hz: 44_100,
            channels: 2,
            bits_per_sample: 16,
            sample_type: AudioSampleType::Int,
        }
    }
}

impl AudioFormat {
    pub fn block_align_bytes(&self) -> Result<usize, AudioCaptureError> {
        if self.channels == 0 {
            return Err(AudioCaptureError::InvalidFormat {
                message: String::from("channel count must be greater than 0"),
            });
        }

        if self.bits_per_sample == 0 || self.bits_per_sample % 8 != 0 {
            return Err(AudioCaptureError::InvalidFormat {
                message: format!(
                    "bit depth must be a positive multiple of 8, got {}",
                    self.bits_per_sample
                ),
            });
        }

        Ok(usize::from(self.channels) * usize::from(self.bits_per_sample / 8))
    }
}

/// 交付给上层的原始音频数据块。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioChunk {
    pub format: AudioFormat,
    pub frames: usize,
    pub bytes: Vec<u8>,
}

impl AudioChunk {
    pub fn new(format: AudioFormat, bytes: Vec<u8>) -> Result<Self, AudioCaptureError> {
        let block_align = format.block_align_bytes()?;

        if bytes.len() % block_align != 0 {
            return Err(AudioCaptureError::InvalidChunkLength {
                received: bytes.len(),
                block_align,
            });
        }

        Ok(Self {
            format,
            frames: bytes.len() / block_align,
            bytes,
        })
    }
}

/// 采集循环的最小运行参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureConfig {
    pub wait_timeout_ms: u32,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            wait_timeout_ms: 100,
        }
    }
}

/// 消费音频数据块的上层接口。
pub trait AudioSink: Send + 'static {
    fn write(&mut self, chunk: AudioChunk) -> Result<(), AudioCaptureError>;
}

/// 音频采集层返回的领域错误。
#[derive(Debug, thiserror::Error)]
pub enum AudioCaptureError {
    #[error("current platform does not support Windows loopback capture")]
    UnsupportedPlatform,
    #[error("failed to initialize audio runtime: {message}")]
    RuntimeInitialization { message: String },
    #[error("invalid audio format: {message}")]
    InvalidFormat { message: String },
    #[error("audio chunk length {received} is not a multiple of frame alignment {block_align}")]
    InvalidChunkLength { received: usize, block_align: usize },
    #[error("failed to start capture thread: {source}")]
    ThreadSpawn {
        #[source]
        source: std::io::Error,
    },
    #[error("capture thread exited unexpectedly")]
    CaptureThreadPanicked,
    #[cfg(target_os = "windows")]
    #[error("WASAPI call failed: {0}")]
    Wasapi(#[from] wasapi::WasapiError),
}

/// 已启动采集任务的控制句柄。
#[derive(Debug)]
pub struct RunningCapture {
    stop_requested: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<(), AudioCaptureError>>>,
}

impl RunningCapture {
    pub fn stop(mut self) -> Result<(), AudioCaptureError> {
        self.stop_requested.store(true, Ordering::Release);
        self.join_worker()
    }

    fn join_worker(&mut self) -> Result<(), AudioCaptureError> {
        match self.worker.take() {
            Some(worker) => match worker.join() {
                Ok(result) => result,
                Err(_) => Err(AudioCaptureError::CaptureThreadPanicked),
            },
            None => Ok(()),
        }
    }
}

impl Drop for RunningCapture {
    fn drop(&mut self) {
        self.stop_requested.store(true, Ordering::Release);

        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

trait CaptureDriver: 'static {
    fn start(&mut self) -> Result<(), AudioCaptureError>;

    fn next_chunk(&mut self, timeout_ms: u32) -> Result<Option<AudioChunk>, AudioCaptureError>;

    fn stop(&mut self) -> Result<(), AudioCaptureError>;
}

fn spawn_capture_worker<S, D, F>(
    sink: S,
    config: CaptureConfig,
    build_driver: F,
    thread_name: &str,
) -> Result<RunningCapture, AudioCaptureError>
where
    S: AudioSink,
    D: CaptureDriver,
    F: FnOnce() -> Result<D, AudioCaptureError> + Send + 'static,
{
    let stop_requested = Arc::new(AtomicBool::new(false));
    let worker_stop_requested = Arc::clone(&stop_requested);
    let thread_name = String::from(thread_name);
    let worker = thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            let mut sink = sink;
            let mut driver = build_driver()?;

            run_capture_loop(&mut driver, &mut sink, config, &worker_stop_requested)
        })
        .map_err(|source| AudioCaptureError::ThreadSpawn { source })?;

    Ok(RunningCapture {
        stop_requested,
        worker: Some(worker),
    })
}

fn run_capture_loop<D, S>(
    driver: &mut D,
    sink: &mut S,
    config: CaptureConfig,
    stop_requested: &AtomicBool,
) -> Result<(), AudioCaptureError>
where
    D: CaptureDriver,
    S: AudioSink,
{
    driver.start()?;

    let capture_result = loop {
        if stop_requested.load(Ordering::Acquire) {
            break Ok(());
        }

        match driver.next_chunk(config.wait_timeout_ms) {
            Ok(Some(chunk)) => {
                if let Err(error) = sink.write(chunk) {
                    break Err(error);
                }
            }
            Ok(None) => {}
            Err(error) => break Err(error),
        }
    };

    let stop_result = driver.stop();

    match (capture_result, stop_result) {
        (Err(error), _) | (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

/// 首版本预留的 Windows 回环采集后端。
#[derive(Debug, Default)]
pub struct WindowsLoopbackCapture;

impl WindowsLoopbackCapture {
    #[must_use]
    pub fn backend_name() -> &'static str {
        "windows-wasapi-loopback"
    }

    pub fn preferred_format() -> Result<AudioFormat, AudioCaptureError> {
        #[cfg(target_os = "windows")]
        {
            windows::preferred_format()
        }

        #[cfg(not(target_os = "windows"))]
        {
            Err(AudioCaptureError::UnsupportedPlatform)
        }
    }

    pub fn start_with_config<S>(
        sink: S,
        config: CaptureConfig,
    ) -> Result<RunningCapture, AudioCaptureError>
    where
        S: AudioSink,
    {
        #[cfg(target_os = "windows")]
        {
            spawn_capture_worker(
                sink,
                config,
                windows::build_default_driver,
                Self::backend_name(),
            )
        }

        #[cfg(not(target_os = "windows"))]
        {
            drop(sink);
            let _ = config;
            Err(AudioCaptureError::UnsupportedPlatform)
        }
    }
}

#[doc(hidden)]
#[allow(dead_code)]
pub mod testing {
    use std::collections::VecDeque;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::{
        AudioCaptureError, AudioChunk, AudioSink, CaptureConfig, CaptureDriver, RunningCapture,
        spawn_capture_worker,
    };

    #[derive(Debug, Clone, Default)]
    pub struct StopCallCounter(Arc<AtomicUsize>);

    impl StopCallCounter {
        #[must_use]
        pub fn count(&self) -> usize {
            self.0.load(Ordering::SeqCst)
        }
    }

    pub fn spawn_scripted_capture_worker<S>(
        sink: S,
        config: CaptureConfig,
        responses: Vec<Result<Option<AudioChunk>, AudioCaptureError>>,
        stop_calls: StopCallCounter,
    ) -> Result<RunningCapture, AudioCaptureError>
    where
        S: AudioSink,
    {
        spawn_capture_worker(
            sink,
            config,
            move || Ok(ScriptedDriver::new(responses, stop_calls)),
            "test-scripted-capture-worker",
        )
    }

    #[derive(Debug)]
    struct ScriptedDriver {
        responses: VecDeque<Result<Option<AudioChunk>, AudioCaptureError>>,
        stop_calls: StopCallCounter,
    }

    impl ScriptedDriver {
        fn new(
            responses: Vec<Result<Option<AudioChunk>, AudioCaptureError>>,
            stop_calls: StopCallCounter,
        ) -> Self {
            Self {
                responses: responses.into(),
                stop_calls,
            }
        }
    }

    impl CaptureDriver for ScriptedDriver {
        fn start(&mut self) -> Result<(), AudioCaptureError> {
            Ok(())
        }

        fn next_chunk(
            &mut self,
            _timeout_ms: u32,
        ) -> Result<Option<AudioChunk>, AudioCaptureError> {
            self.responses.pop_front().unwrap_or(Ok(None))
        }

        fn stop(&mut self) -> Result<(), AudioCaptureError> {
            self.stop_calls.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AudioCaptureError, AudioChunk, AudioFormat, AudioSampleType, AudioSink, CaptureConfig,
        CaptureDriver, WindowsLoopbackCapture, spawn_capture_worker,
    };
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;

    #[derive(Debug)]
    struct FakeDriver {
        responses: VecDeque<Result<Option<AudioChunk>, AudioCaptureError>>,
        stop_calls: Arc<AtomicUsize>,
    }

    impl FakeDriver {
        fn new(
            responses: Vec<Result<Option<AudioChunk>, AudioCaptureError>>,
            stop_calls: Arc<AtomicUsize>,
        ) -> Self {
            Self {
                responses: responses.into(),
                stop_calls,
            }
        }
    }

    impl CaptureDriver for FakeDriver {
        fn start(&mut self) -> Result<(), AudioCaptureError> {
            Ok(())
        }

        fn next_chunk(
            &mut self,
            _timeout_ms: u32,
        ) -> Result<Option<AudioChunk>, AudioCaptureError> {
            self.responses.pop_front().unwrap_or(Ok(None))
        }

        fn stop(&mut self) -> Result<(), AudioCaptureError> {
            self.stop_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

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
                    message: String::from("recording buffer was poisoned"),
                })?;
            chunks.push(chunk);
            self.notify
                .send(chunks.len())
                .map_err(|_| AudioCaptureError::InvalidFormat {
                    message: String::from("test notification channel is closed"),
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
                    message: String::from("test notification channel is closed"),
                })?;

            Err(AudioCaptureError::InvalidFormat {
                message: String::from("audio sink rejected chunk"),
            })
        }
    }

    #[test]
    fn default_audio_format_matches_cd_quality() {
        let format = AudioFormat::default();

        assert_eq!(format.sample_rate_hz, 44_100);
        assert_eq!(format.channels, 2);
        assert_eq!(format.bits_per_sample, 16);
        assert_eq!(format.sample_type, AudioSampleType::Int);
    }

    #[test]
    fn audio_chunk_tracks_frame_count() {
        let chunk = AudioChunk::new(AudioFormat::default(), vec![0; 8]).unwrap();

        assert_eq!(chunk.frames, 2);
    }

    #[test]
    fn audio_chunk_rejects_misaligned_bytes() {
        let error = AudioChunk::new(AudioFormat::default(), vec![0; 3]).unwrap_err();

        assert!(matches!(
            error,
            AudioCaptureError::InvalidChunkLength {
                received: 3,
                block_align: 4,
            }
        ));
    }

    #[test]
    fn capture_worker_writes_chunks_to_sink() {
        let stop_calls = Arc::new(AtomicUsize::new(0));
        let chunks = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel();
        let chunk_a = AudioChunk::new(AudioFormat::default(), vec![1; 4]).unwrap();
        let chunk_b = AudioChunk::new(AudioFormat::default(), vec![2; 8]).unwrap();
        let expected = vec![chunk_a.clone(), chunk_b.clone()];
        let sink = RecordingSink {
            chunks: Arc::clone(&chunks),
            notify: tx,
        };
        let worker = spawn_capture_worker(
            sink,
            CaptureConfig { wait_timeout_ms: 1 },
            {
                let stop_calls = Arc::clone(&stop_calls);
                move || {
                    Ok(FakeDriver::new(
                        vec![Ok(Some(chunk_a)), Ok(Some(chunk_b)), Ok(None)],
                        stop_calls,
                    ))
                }
            },
            "test-capture-worker",
        )
        .unwrap();

        assert_eq!(rx.recv_timeout(Duration::from_millis(100)).unwrap(), 1);
        assert_eq!(rx.recv_timeout(Duration::from_millis(100)).unwrap(), 2);
        worker.stop().unwrap();

        let actual = chunks.lock().unwrap();
        assert_eq!(*actual, expected);
        assert_eq!(stop_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn capture_worker_propagates_sink_error() {
        let stop_calls = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel();
        let chunk = AudioChunk::new(AudioFormat::default(), vec![7; 4]).unwrap();
        let worker = spawn_capture_worker(
            FailingSink { notify: tx },
            CaptureConfig { wait_timeout_ms: 1 },
            {
                let stop_calls = Arc::clone(&stop_calls);
                move || Ok(FakeDriver::new(vec![Ok(Some(chunk))], stop_calls))
            },
            "test-capture-error",
        )
        .unwrap();

        rx.recv_timeout(Duration::from_millis(100)).unwrap();
        let error = worker.stop().unwrap_err();

        assert!(matches!(error, AudioCaptureError::InvalidFormat { .. }));
        assert_eq!(stop_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn windows_backend_reports_name() {
        assert_eq!(
            WindowsLoopbackCapture::backend_name(),
            "windows-wasapi-loopback"
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn preferred_format_reports_unsupported_platform_outside_windows() {
        let error = WindowsLoopbackCapture::preferred_format().unwrap_err();

        assert!(matches!(error, AudioCaptureError::UnsupportedPlatform));
    }
}
