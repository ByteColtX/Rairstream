use std::io::{self, Write};
use std::sync::mpsc;
use std::time::Duration;

use rairstream::app::{AppController, SessionCoordinator, SessionState, SpeakerDevice};
use rairstream::config::AppConfig;
use rairstream::discovery::MdnsDiscoveryService;
use rairstream::ui::run_tray_app;
use tracing::{debug, error, info};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone, PartialEq, Eq)]
enum LaunchMode {
    Tray,
    Cli(CliCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliCommand {
    Discover,
    Start {
        device_filter: Option<String>,
        pin: Option<String>,
    },
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
        LaunchMode::Cli(command) => {
            if let Err(error) = run_cli_command(command) {
                error!(error = %error, "CLI 模式执行失败");
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
            "--device" | "--pin" => positionals.push(arg),
            _ if arg.starts_with('-') => {
                return Err(format!("未知参数: {arg}"));
            }
            _ => positionals.push(arg),
        }
    }

    let mode = parse_launch_mode(&positionals)?;

    Ok(CliOptions {
        mode,
        log_level,
        verbosity,
    })
}

fn parse_launch_mode(positionals: &[String]) -> Result<LaunchMode, String> {
    match positionals {
        [] => Ok(LaunchMode::Tray),
        [command] if command == "tray" => Ok(LaunchMode::Tray),
        [command, subcommand] if command == "cli" && subcommand == "discover" => {
            Ok(LaunchMode::Cli(CliCommand::Discover))
        }
        [command, subcommand] if command == "cli" && subcommand == "start" => {
            Ok(LaunchMode::Cli(CliCommand::Start {
                device_filter: None,
                pin: None,
            }))
        }
        [command, subcommand, flag, value]
            if command == "cli" && subcommand == "start" && flag == "--device" =>
        {
            Ok(LaunchMode::Cli(CliCommand::Start {
                device_filter: Some(value.clone()),
                pin: None,
            }))
        }
        [command, subcommand, flag, value]
            if command == "cli" && subcommand == "start" && flag == "--pin" =>
        {
            Ok(LaunchMode::Cli(CliCommand::Start {
                device_filter: None,
                pin: Some(value.clone()),
            }))
        }
        [
            command,
            subcommand,
            first_flag,
            first_value,
            second_flag,
            second_value,
        ] if command == "cli" && subcommand == "start" => {
            let mut device_filter = None;
            let mut pin = None;
            for (flag, value) in [(first_flag, first_value), (second_flag, second_value)] {
                match flag.as_str() {
                    "--device" => device_filter = Some(value.clone()),
                    "--pin" => pin = Some(value.clone()),
                    _ => return Err(format!("未知参数: {flag}")),
                }
            }
            Ok(LaunchMode::Cli(CliCommand::Start { device_filter, pin }))
        }
        [command, subcommand, ..] if command == "cli" && subcommand == "discover" => {
            Err(String::from("cli discover 不接受额外参数"))
        }
        [command, subcommand, ..] if command == "cli" && subcommand == "start" => Err(
            String::from("cli start 仅支持 --device <值> 和 --pin <值>，每个参数最多出现一次"),
        ),
        [command, ..] if command == "cli" => {
            Err(String::from("未知 cli 子命令，可用子命令：discover、start"))
        }
        [command, ..] => Err(format!("未知命令: {command}")),
    }
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

fn build_cli_controller() -> AppController<SessionCoordinator<MdnsDiscoveryService>> {
    let config = load_app_config();
    let coordinator = SessionCoordinator::with_paired_receivers(
        MdnsDiscoveryService::new(Duration::from_secs(3)),
        config.paired_receivers.clone(),
    );
    AppController::new(coordinator, config)
}

fn run_cli_command(command: &CliCommand) -> Result<(), String> {
    if !cfg!(target_os = "windows") {
        return Err(String::from("CLI 模式仅支持在 Windows 上运行"));
    }

    let mut controller = build_cli_controller();
    controller.refresh_devices();

    match command {
        CliCommand::Discover => {
            print_discovered_devices(controller.devices())?;
            Ok(())
        }
        CliCommand::Start { device_filter, pin } => {
            run_cli_start(&mut controller, device_filter.as_deref(), pin.as_deref())
        }
    }
}

fn print_discovered_devices(devices: &[SpeakerDevice]) -> Result<(), String> {
    if devices.is_empty() {
        return Err(String::from("未发现可用 AirPlay / RAOP 设备"));
    }

    for device in devices {
        println!(
            "- {} | id={} | host={} | kind={:?}",
            device.name, device.id, device.host, device.receiver_kind
        );
    }

    Ok(())
}

fn run_cli_start(
    controller: &mut AppController<SessionCoordinator<MdnsDiscoveryService>>,
    device_filter: Option<&str>,
    pin: Option<&str>,
) -> Result<(), String> {
    let device = controller
        .resolve_target_device(device_filter)
        .map_err(|error| controller.describe_error(None, &error))?;
    let device_id = device.id.clone();
    let device_name = device.name.clone();

    match controller.select_device(&device_id) {
        Ok(()) => {}
        Err(error) => {
            controller.handle_error(Some(&device_id), &error);
            return Err(controller.describe_error(Some(&device_id), &error));
        }
    }

    if matches!(
        controller.app_state().active_session,
        SessionState::AwaitingPairing { .. }
    ) {
        let pin = match pin {
            Some(pin) => pin.to_string(),
            None => prompt_pairing_pin(&device_name)?,
        };
        if let Err(error) = controller.submit_pairing_pin(&device_id, &pin) {
            controller.handle_error(Some(&device_id), &error);
            return Err(controller.describe_error(Some(&device_id), &error));
        }
    }

    match controller.app_state().active_session {
        SessionState::Streaming { .. } => {
            info!(device_id = %device_id, device_name = %device_name, "CLI 串流已启动，等待 Ctrl+C 停止");
            wait_for_ctrl_c().map_err(|error| error.to_string())?;
            controller
                .stop_streaming()
                .map_err(|error| controller.describe_error(Some(&device_id), &error))?;
            info!(device_id = %device_id, device_name = %device_name, "CLI 串流已停止");
            Ok(())
        }
        SessionState::Authenticating { .. } => Err(format!("{device_name} 正在认证，请稍后重试")),
        SessionState::AwaitingPairing { .. } => Err(format!("{device_name} 需要先完成首次配对")),
        _ => Err(format!("{device_name} 未能进入串流状态")),
    }
}

fn prompt_pairing_pin(device_name: &str) -> Result<String, String> {
    print!("请输入 {device_name} 上显示的 PIN: ");
    io::stdout()
        .flush()
        .map_err(|error| format!("刷新 CLI 提示失败: {error}"))?;
    let mut pin = String::new();
    io::stdin()
        .read_line(&mut pin)
        .map_err(|error| format!("读取 PIN 失败: {error}"))?;
    let pin = pin.trim();
    if pin.is_empty() {
        return Err(String::from("PIN 不能为空"));
    }
    Ok(pin.to_string())
}

fn wait_for_ctrl_c() -> Result<(), io::Error> {
    let (sender, receiver) = mpsc::channel();
    ctrlc::set_handler(move || {
        let _ = sender.send(());
    })
    .map_err(io::Error::other)?;
    receiver.recv().map_err(io::Error::other)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CliCommand, CliOptions, LaunchMode, parse_cli, parse_launch_mode,
        resolve_log_directive_with_env,
    };

    #[test]
    fn parse_cli_defaults_to_tray_mode() {
        let cli = parse_cli(Vec::<String>::new()).expect("default parse should succeed");

        assert_eq!(cli.mode, LaunchMode::Tray);
    }

    #[test]
    fn parse_cli_accepts_cli_discover() {
        let cli = parse_cli([String::from("cli"), String::from("discover")])
            .expect("discover parse should succeed");

        assert_eq!(cli.mode, LaunchMode::Cli(CliCommand::Discover));
    }

    #[test]
    fn parse_cli_accepts_cli_start_with_device_and_pin() {
        let cli = parse_cli([
            String::from("cli"),
            String::from("start"),
            String::from("--device"),
            String::from("Living Room"),
            String::from("--pin"),
            String::from("123456"),
        ])
        .expect("start parse should succeed");

        assert_eq!(
            cli.mode,
            LaunchMode::Cli(CliCommand::Start {
                device_filter: Some(String::from("Living Room")),
                pin: Some(String::from("123456")),
            })
        );
    }

    #[test]
    fn parse_cli_rejects_unknown_subcommand() {
        let args = vec![String::from("cli"), String::from("oops")];
        let error = parse_launch_mode(&args).expect_err("unknown subcommand should fail");

        assert!(error.contains("未知 cli 子命令"));
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
