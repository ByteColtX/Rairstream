use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent};

use crate::app::SpeakerDevice;

use super::DiscoveryService;
use super::parser::{MdnsServiceKind, ResolvedMdnsService, parse_resolved_services};

const RAOP_SERVICE_TYPE: &str = "_raop._tcp.local.";
const AIRPLAY_SERVICE_TYPE: &str = "_airplay._tcp.local.";
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(2);

/// 基于 mDNS 的 AirPlay/RAOP 设备发现实现。
#[derive(Debug, Clone, Copy)]
pub struct MdnsDiscoveryService {
    timeout: Duration,
}

impl Default for MdnsDiscoveryService {
    fn default() -> Self {
        Self {
            timeout: DISCOVERY_TIMEOUT,
        }
    }
}

impl MdnsDiscoveryService {
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }
}

impl DiscoveryService for MdnsDiscoveryService {
    fn discover_devices(&self) -> Vec<SpeakerDevice> {
        let Ok(daemon) = ServiceDaemon::new() else {
            return Vec::new();
        };

        let mut services = discover_service_type(
            &daemon,
            MdnsServiceKind::Raop,
            RAOP_SERVICE_TYPE,
            self.timeout,
        );
        services.extend(discover_service_type(
            &daemon,
            MdnsServiceKind::AirPlay,
            AIRPLAY_SERVICE_TYPE,
            self.timeout,
        ));

        parse_resolved_services(services)
    }
}

fn discover_service_type(
    daemon: &ServiceDaemon,
    service_kind: MdnsServiceKind,
    service_type: &str,
    timeout: Duration,
) -> Vec<ResolvedMdnsService> {
    let Ok(receiver) = daemon.browse(service_type) else {
        return Vec::new();
    };

    let deadline = Instant::now() + timeout;
    let mut services = Vec::new();

    loop {
        let Some(wait_time) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };

        let Ok(event) = receiver.recv_timeout(wait_time) else {
            break;
        };

        if let ServiceEvent::ServiceResolved(service) = event {
            let device_id = service.get_property_val_str("deviceid").map(String::from);
            let ipv4_addresses = service.get_addresses_v4().into_iter().collect();

            services.push(ResolvedMdnsService {
                service_kind,
                fullname: service.fullname,
                port: service.port,
                ipv4_addresses,
                device_id,
            });
        }
    }

    let _ = daemon.stop_browse(service_type);

    services
}
