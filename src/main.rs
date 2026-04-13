use std::io;
use std::time::Duration;

use rairstream::app::SessionCoordinator;
use rairstream::audio::{
    AudioCaptureError, AudioChunk, AudioFormat, AudioSink, WindowsLoopbackCapture,
};
use rairstream::config::AppConfig;
use rairstream::discovery::MdnsDiscoveryService;
use rairstream::transport::{AirPlayError, RaopAudioSink, RaopSession, SessionDescriptor};
use rairstream::ui::run_tray_app;

fn main() {
    let mut args = std::env::args().skip(1);

    if matches!(args.next().as_deref(), Some("smoke")) {
        if let Err(error) = run_smoke_mode(args.next().as_deref()) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }

    let config = AppConfig::default();
    let coordinator = SessionCoordinator::new(MdnsDiscoveryService::default());

    if let Err(error) = run_tray_app(coordinator, config) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run_smoke_mode(device_filter: Option<&str>) -> Result<(), String> {
    if !cfg!(target_os = "windows") {
        return Err(String::from("smoke 模式仅支持在 Windows 上运行"));
    }

    let coordinator = SessionCoordinator::new(MdnsDiscoveryService::new(Duration::from_secs(3)));
    let devices = coordinator.discover();
    if devices.is_empty() {
        return Err(String::from("未发现可用 AirPlay / RAOP 设备"));
    }

    let device = match device_filter {
        Some(filter) => devices
            .into_iter()
            .find(|device| {
                device.id.contains(filter)
                    || device.name.contains(filter)
                    || device.host.contains(filter)
            })
            .ok_or_else(|| format!("未找到匹配设备: {filter}"))?,
        None => devices
            .into_iter()
            .next()
            .ok_or_else(|| String::from("未发现可用 AirPlay / RAOP 设备"))?,
    };

    println!(
        "准备向 {} ({}) 启动 AirPlay 串流，按回车停止。",
        device.name, device.host
    );
    println!("设备摘要: id={}, endpoint={}", device.id, device.endpoint());

    let format = WindowsLoopbackCapture::preferred_format().map_err(|error| error.to_string())?;
    println!(
        "系统回环采集格式: {} Hz, {} 声道, {} bit, {:?}",
        format.sample_rate_hz, format.channels, format.bits_per_sample, format.sample_type
    );

    let descriptor = SessionDescriptor::new(device.clone(), format);
    let session = RaopSession::connect(&descriptor).map_err(|error| error.to_string())?;
    let connection = session
        .handshake_with_progress(|message| println!("[raop] {message}"))
        .map_err(|error| match error {
            AirPlayError::AuthenticationRequired => format!(
                "{}\n提示：你当前选择的是 {}，它更像需要认证/配对的 AirPlay 接收端（例如 macOS AirPlay Receiver）。当前 MVP 仅支持免认证的 AirPlay 1 / RAOP 目标，尚未实现配对认证流程。",
                error, device.name
            ),
            other => other.to_string(),
        })?;
    let sink = DiagnosticAudioSink::new(
        format,
        RaopAudioSink::new(
            format,
            connection
                .stream_transport()
                .map_err(|error| error.to_string())?,
        ),
    );
    let capture = WindowsLoopbackCapture::start(sink).map_err(|error| error.to_string())?;
    println!("[audio] 已启动系统音频捕获，等待音频块并持续发包。按回车停止。");

    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;

    capture.stop().map_err(|error| error.to_string())?;
    connection.teardown().map_err(|error| error.to_string())?;
    println!("[raop] 已发送 TEARDOWN，烟测结束。");
    Ok(())
}

#[derive(Debug)]
struct DiagnosticAudioSink<S> {
    inner: S,
    source_format: AudioFormat,
    seen_chunks: usize,
}

impl<S> DiagnosticAudioSink<S> {
    fn new(source_format: AudioFormat, inner: S) -> Self {
        Self {
            inner,
            source_format,
            seen_chunks: 0,
        }
    }
}

impl<S> AudioSink for DiagnosticAudioSink<S>
where
    S: AudioSink,
{
    fn write(&mut self, chunk: AudioChunk) -> Result<(), AudioCaptureError> {
        self.seen_chunks = self.seen_chunks.saturating_add(1);
        if self.seen_chunks <= 5 {
            println!(
                "[audio] 收到 chunk #{}, frames={}, bytes={}, input={} Hz/{}ch/{}bit {:?}",
                self.seen_chunks,
                chunk.frames,
                chunk.bytes.len(),
                self.source_format.sample_rate_hz,
                self.source_format.channels,
                self.source_format.bits_per_sample,
                self.source_format.sample_type
            );
        }

        self.inner.write(chunk)
    }
}
