//! 设备发现层：负责抽象 `AirPlay` / `RAOP` 设备浏览结果。

mod mdns;
mod parser;

use crate::app::{AirPlayGeneration, SpeakerDevice};

pub use mdns::MdnsDiscoveryService;

#[doc(hidden)]
pub mod testing {
    use std::net::Ipv4Addr;

    use crate::app::SpeakerDevice;

    use super::parser::{MdnsServiceKind, ResolvedMdnsService, parse_resolved_services};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum MdnsTestServiceKind {
        Raop,
        AirPlay,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct MdnsTestResolvedService {
        pub service_kind: MdnsTestServiceKind,
        pub fullname: String,
        pub port: u16,
        pub ipv4_addresses: Vec<Ipv4Addr>,
        pub device_id: Option<String>,
    }

    #[must_use]
    pub fn parse_test_services(services: Vec<MdnsTestResolvedService>) -> Vec<SpeakerDevice> {
        parse_resolved_services(
            services
                .into_iter()
                .map(|service| ResolvedMdnsService {
                    service_kind: match service.service_kind {
                        MdnsTestServiceKind::Raop => MdnsServiceKind::Raop,
                        MdnsTestServiceKind::AirPlay => MdnsServiceKind::AirPlay,
                    },
                    fullname: service.fullname,
                    port: service.port,
                    ipv4_addresses: service.ipv4_addresses,
                    device_id: service.device_id,
                })
                .collect(),
        )
    }
}

/// 发现服务的最小接口。
pub trait DiscoveryService {
    fn discover_devices(&self) -> Vec<SpeakerDevice>;
}

/// 当前阶段的内存实现，可用于无网络环境下的稳定测试。
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
