use rairstream::app::{SessionCoordinator, platform::current_platform};
use rairstream::config::AppConfig;
use rairstream::discovery::MdnsDiscoveryService;

fn main() {
    // 首个版本先搭建命令行可运行骨架，后续再接入托盘与桌面 UI。
    let platform = current_platform();
    let config = AppConfig::default();
    let coordinator = SessionCoordinator::new(MdnsDiscoveryService::default());
    let devices = coordinator.discover();

    println!("Rairstream starting...");
    println!("Platform: {}", platform.os);
    println!("Auto reconnect: {}", config.auto_reconnect);
    println!("Discovered devices: {}", devices.len());

    for device in &devices {
        println!("- {} [{}] at {}", device.name, device.id, device.endpoint());
    }

    if devices.is_empty() {
        println!("未发现可用的 AirPlay/RAOP 设备。");
    }
}
