use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent, TxtProperty};
use tracing::trace;

use crate::receiver::Receiver;

use super::parser::parse_resolved_services;
use super::{DiscoveryService, MdnsServiceKind, ResolvedMdnsService};

const RAOP_SERVICE_TYPE: &str = "_raop._tcp.local.";
const AIRPLAY_SERVICE_TYPE: &str = "_airplay._tcp.local.";
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(2);

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
    fn discover_devices(&self) -> Vec<Receiver> {
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

        parse_resolved_services(&services)
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
            let txt_records = collect_txt_records(service.get_properties().iter());
            let ipv4_addresses = service.get_addresses_v4().into_iter().collect();
            let txt_properties = format_txt_properties(service.get_properties().iter());
            trace!(
                service_kind = ?service_kind,
                fullname = service.get_fullname(),
                hostname = service.get_hostname(),
                port = service.get_port(),
                ipv4_addresses = ?service.get_addresses_v4(),
                txt_properties = %txt_properties,
                "resolved mDNS service"
            );

            services.push(ResolvedMdnsService {
                service_kind,
                fullname: service.fullname,
                port: service.port,
                ipv4_addresses,
                txt_records,
            });
        }
    }

    let _ = daemon.stop_browse(service_type);

    services
}

fn collect_txt_records<'a>(
    properties: impl Iterator<Item = &'a TxtProperty>,
) -> BTreeMap<String, String> {
    properties
        .map(|property| {
            let value = property
                .val()
                .map_or_else(String::new, |_| property.val_str().to_string());
            (property.key().to_ascii_lowercase(), value)
        })
        .collect()
}

fn format_txt_properties<'a>(properties: impl Iterator<Item = &'a TxtProperty>) -> String {
    let formatted: Vec<String> = properties
        .map(|property| {
            let value = property.val().map_or_else(
                || String::from("<flag>"),
                |_| property.val_str().to_string(),
            );
            format!("{}={value}", property.key())
        })
        .collect();

    if formatted.is_empty() {
        return String::from("<empty>");
    }

    formatted.join(", ")
}
