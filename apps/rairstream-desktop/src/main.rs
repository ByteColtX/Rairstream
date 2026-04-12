use anyhow::Result;
use rairstream_config::AppConfig;
use rairstream_device_discovery::StubDiscoveryService;
use rairstream_session::SessionCoordinator;

fn main() -> Result<()> {
    // 首个版本先搭建命令行可运行骨架，后续再接入托盘与桌面 UI。
    let config = AppConfig::default();
    let coordinator = SessionCoordinator::new(StubDiscoveryService);
    let devices = coordinator.discover();

    println!("Rairstream starting...");
    println!("Auto reconnect: {}", config.auto_reconnect);
    println!("Discovered devices: {}", devices.len());

    Ok(())
}
