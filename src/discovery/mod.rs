//! 设备发现层：负责抽象 `AirPlay` / `RAOP` 设备浏览结果。

use crate::app::{AirPlayGeneration, SpeakerDevice};

/// 发现服务的最小接口。
pub trait DiscoveryService {
    fn discover_devices(&self) -> Vec<SpeakerDevice>;
}

/// 当前阶段的内存实现，后续替换为 mDNS/Bonjour 浏览器。
#[derive(Debug, Default)]
pub struct StubDiscoveryService;

impl DiscoveryService for StubDiscoveryService {
    fn discover_devices(&self) -> Vec<SpeakerDevice> {
        vec![SpeakerDevice {
            id: String::from("stub-speaker"),
            name: String::from("Stub Speaker"),
            host: String::from("127.0.0.1"),
            port: 7000,
            generation: AirPlayGeneration::AirPlay1,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::{DiscoveryService, StubDiscoveryService};

    #[test]
    fn stub_discovery_returns_placeholder_device() {
        let devices = StubDiscoveryService.discover_devices();

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].port, 7000);
    }
}
