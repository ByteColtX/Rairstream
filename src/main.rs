use std::io;
use std::time::Duration;

use rairstream::app::SessionCoordinator;
use rairstream::audio::{
    AudioCaptureError, AudioChunk, AudioFormat, AudioSink, WindowsLoopbackCapture,
};
use rairstream::config::AppConfig;
use rairstream::discovery::MdnsDiscoveryService;
use rairstream::transport::{AirPlayError, RaopAudioSink};
use rairstream::ui::run_tray_app;
use tracing::{debug, error, info};

fn format_smoke_prepare_error(
    device_name: &str,
    error: rairstream::app::RairstreamError,
) -> String {
    match error {
        rairstream::app::RairstreamError::Transport(transport_error) => {
            format_smoke_transport_error(device_name, transport_error)
        }
        other => other.to_string(),
    }
}

fn format_smoke_transport_error(device_name: &str, error: AirPlayError) -> String {
    match error {
        AirPlayError::AuthenticationRequired => format!(
            "{}\n提示：你当前选择的是 {}，它更像需要配对或认证的 AirPlay 接收端（例如 macOS AirPlay Receiver 或 Apple TV）。请在托盘流程中完成配对，或检查本地是否已有可复用的配对记录。",
            AirPlayError::AuthenticationRequired,
            device_name
        ),
        AirPlayError::PairingRequired => format!(
            "{}\n提示：{} 需要先完成首次配对，请在托盘流程中输入设备显示的 PIN。",
            AirPlayError::PairingRequired,
            device_name
        ),
        AirPlayError::CredentialsMissing => format!(
            "{}\n提示：{} 需要可复用的 AirPlay Receiver 配对记录，但当前配置中没有可用记录，请先完成首次配对。",
            AirPlayError::CredentialsMissing,
            device_name
        ),
        AirPlayError::AuthenticationFailed { message } => format!(
            "AirPlay Receiver 认证失败: {message}\n提示：{device_name} 的现有配对记录可能已失效，或设备要求重新配对。"
        ),
        other => other.to_string(),
    }
}
use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone, PartialEq, Eq)]
enum LaunchMode {
    Tray,
    Smoke { device_filter: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliOptions {
    mode: LaunchMode,
    log_level: Option<String>,
    verbosity: u8,
}

fn main() {
    let cli = match parse_cli(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };

    if let Err(error) = init_logging(&cli) {
        eprintln!("{error}");
        std::process::exit(2);
    }

    match &cli.mode {
        LaunchMode::Smoke { device_filter } => {
            if let Err(error) = run_smoke_mode(device_filter.as_deref()) {
                error!(error = %error, "烟测模式执行失败");
                std::process::exit(1);
            }
        }
        LaunchMode::Tray => {
            let config = load_app_config();
            let coordinator = SessionCoordinator::with_paired_receivers(
                MdnsDiscoveryService::default(),
                config.paired_receivers.clone(),
            );

            if let Err(error) = run_tray_app(coordinator, config) {
                error!(error = %error, "托盘模式启动失败");
                std::process::exit(1);
            }
        }
    }
}

fn parse_cli(args: impl IntoIterator<Item = String>) -> Result<CliOptions, String> {
    let mut verbosity = 0_u8;
    let mut log_level = None;
    let mut positionals = Vec::new();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-v" | "--verbose" => {
                verbosity = verbosity.saturating_add(1);
            }
            "-vv" => {
                verbosity = verbosity.saturating_add(2);
            }
            "--log-level" => {
                let value = args
                    .next()
                    .ok_or_else(|| String::from("参数 --log-level 缺少日志等级值"))?;
                log_level = Some(parse_log_level(value)?);
            }
            _ if arg.starts_with("--log-level=") => {
                let value = arg.trim_start_matches("--log-level=").to_string();
                log_level = Some(parse_log_level(value)?);
            }
            _ if arg.starts_with('-') => {
                return Err(format!("未知参数: {arg}"));
            }
            _ => positionals.push(arg),
        }
    }

    let mode = match positionals.as_slice() {
        [] => LaunchMode::Tray,
        [command] if command == "smoke" => LaunchMode::Smoke {
            device_filter: None,
        },
        [command, device_filter] if command == "smoke" => LaunchMode::Smoke {
            device_filter: Some(device_filter.clone()),
        },
        [command, ..] if command == "smoke" => {
            return Err(String::from("smoke 模式最多只接受一个设备过滤参数"));
        }
        [command, ..] => {
            return Err(format!("未知命令: {command}"));
        }
    };

    Ok(CliOptions {
        mode,
        log_level,
        verbosity,
    })
}

fn parse_log_level(value: String) -> Result<String, String> {
    match value.as_str() {
        "error" | "warn" | "info" | "debug" | "trace" => Ok(value),
        _ => Err(format!(
            "不支持的日志等级: {value}。可选值: error, warn, info, debug, trace"
        )),
    }
}

fn init_logging(cli: &CliOptions) -> Result<(), String> {
    let directive = resolve_log_directive_with_env(
        cli,
        std::env::var("RAIRSTREAM_LOG").ok(),
        std::env::var("RUST_LOG").ok(),
    );
    let env_filter = EnvFilter::try_new(&directive)
        .map_err(|error| format!("日志过滤规则无效 ({directive}): {error}"))?;

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(env_filter)
        .try_init()
        .map_err(|error| format!("初始化日志失败: {error}"))?;

    debug!(log_filter = %directive, "日志系统已初始化");
    Ok(())
}

fn resolve_log_directive_with_env(
    cli: &CliOptions,
    rairstream_log: Option<String>,
    rust_log: Option<String>,
) -> String {
    if let Some(log_level) = &cli.log_level {
        return log_level.clone();
    }

    if cli.verbosity >= 2 {
        return String::from("trace");
    }

    if cli.verbosity == 1 {
        return String::from("debug");
    }

    if let Some(log_filter) = first_non_empty(rairstream_log) {
        return log_filter;
    }

    if let Some(log_filter) = first_non_empty(rust_log) {
        return log_filter;
    }

    String::from("info")
}

fn first_non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn load_app_config() -> AppConfig {
    match AppConfig::load() {
        Ok(config) => config,
        Err(error) => {
            error!(error = %error, "加载配置失败，回退默认配置");
            AppConfig::default()
        }
    }
}

fn run_smoke_mode(device_filter: Option<&str>) -> Result<(), String> {
    if !cfg!(target_os = "windows") {
        return Err(String::from("smoke 模式仅支持在 Windows 上运行"));
    }

    info!(device_filter = ?device_filter, "开始执行烟测模式");
    let coordinator = SessionCoordinator::with_paired_receivers(
        MdnsDiscoveryService::new(Duration::from_secs(3)),
        AppConfig::default().paired_receivers,
    );
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

    info!(
        device_id = %device.id,
        device_name = %device.name,
        device_host = %device.host,
        endpoint = %device.endpoint(),
        "准备启动 AirPlay 烟测串流，按回车停止"
    );

    let format = WindowsLoopbackCapture::preferred_format().map_err(|error| error.to_string())?;
    info!(
        sample_rate_hz = format.sample_rate_hz,
        channels = format.channels,
        bits_per_sample = format.bits_per_sample,
        sample_type = ?format.sample_type,
        "已获取系统回环采集格式"
    );

    let prepared = coordinator
        .prepare_transport_session(device.clone())
        .map_err(|error| format_smoke_prepare_error(&device.name, error))?;
    let connection = prepared
        .transport
        .handshake()
        .map_err(|error| format_smoke_transport_error(&device.name, error))?;
    let sink = DiagnosticAudioSink::new(
        format,
        RaopAudioSink::new(
            format,
            connection
                .stream_transport()
                .map_err(|error| error.to_string())?,
            std::sync::Arc::new(std::sync::Mutex::new(100)),
        ),
    );
    let capture = WindowsLoopbackCapture::start(sink).map_err(|error| error.to_string())?;
    info!("已启动系统音频捕获，等待音频块并持续发包");

    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;

    capture.stop().map_err(|error| error.to_string())?;
    connection.teardown().map_err(|error| error.to_string())?;
    info!("已发送 TEARDOWN，烟测结束");
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
            debug!(
                chunk_index = self.seen_chunks,
                frames = chunk.frames,
                bytes = chunk.bytes.len(),
                sample_rate_hz = self.source_format.sample_rate_hz,
                channels = self.source_format.channels,
                bits_per_sample = self.source_format.bits_per_sample,
                sample_type = ?self.source_format.sample_type,
                "收到系统音频块"
            );
        }

        self.inner.write(chunk)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CliOptions, LaunchMode, format_smoke_prepare_error, format_smoke_transport_error,
        resolve_log_directive_with_env,
    };
    use rairstream::app::RairstreamError;
    use rairstream::transport::AirPlayError;

    #[test]
    fn smoke_transport_error_formats_pairing_related_hints() {
        let pairing = format_smoke_transport_error("Receiver", AirPlayError::PairingRequired);
        let missing = format_smoke_transport_error("Receiver", AirPlayError::CredentialsMissing);
        let failed = format_smoke_transport_error(
            "Receiver",
            AirPlayError::AuthenticationFailed {
                message: String::from("forbidden"),
            },
        );

        assert!(pairing.contains("首次配对"));
        assert!(missing.contains("没有可用记录"));
        assert!(failed.contains("forbidden"));
    }

    #[test]
    fn smoke_prepare_error_reuses_transport_mapping() {
        let formatted = format_smoke_prepare_error(
            "Receiver",
            RairstreamError::Transport(AirPlayError::PairingRequired),
        );

        assert!(formatted.contains("Receiver"));
        assert!(formatted.contains("首次配对"));
    }

    #[test]
    fn verbose_level_promotes_trace_logging() {
        let cli = CliOptions {
            mode: LaunchMode::Tray,
            log_level: None,
            verbosity: 2,
        };

        assert_eq!(
            resolve_log_directive_with_env(&cli, Some(String::from("info")), None),
            "trace"
        );
    }
}
