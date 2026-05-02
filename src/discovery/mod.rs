pub mod browser;
pub mod model;
mod parser;

use crate::receiver::Receiver;

pub use browser::MdnsDiscoveryService;
pub use model::{MdnsServiceKind, ResolvedMdnsService};

#[doc(hidden)]
pub mod testing {
    use std::net::Ipv4Addr;

    use crate::receiver::Receiver;

    use super::parser::parse_resolved_services;
    use super::{MdnsServiceKind, ResolvedMdnsService};

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
        pub pairing_id: Option<String>,
        pub model_or_am: Option<String>,
        pub features: Option<String>,
        pub flags: Option<String>,
        pub srcvers: Option<String>,
        pub receiver_public_key: Option<String>,
    }

    #[must_use]
    pub fn parse_test_services(services: Vec<MdnsTestResolvedService>) -> Vec<Receiver> {
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
                    pairing_id: service.pairing_id,
                    model_or_am: service.model_or_am,
                    features: service.features,
                    flags: service.flags,
                    srcvers: service.srcvers,
                    receiver_public_key: service.receiver_public_key,
                })
                .collect(),
        )
    }
}

pub trait DiscoveryService {
    fn discover_devices(&self) -> Vec<Receiver>;
}

#[derive(Debug, Default)]
pub struct StubDiscoveryService;

impl DiscoveryService for StubDiscoveryService {
    fn discover_devices(&self) -> Vec<Receiver> {
        vec![Receiver {
            id: String::from("stub-speaker"),
            name: String::from("Stub Speaker"),
            host: String::from("127.0.0.1"),
            port: 7000,
            generation: crate::receiver::AirPlayGeneration::AirPlay1,
            pairing_id: None,
            receiver_public_key: None,
            receiver_kind: crate::receiver::ReceiverKind::ClassicRaop,
            support: crate::receiver::DeviceSupport::Supported,
            capabilities: crate::receiver::ReceiverCapabilities::default(),
        }]
    }
}
