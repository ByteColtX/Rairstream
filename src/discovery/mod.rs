mod browser;
mod model;
mod parser;

use crate::receiver::Receiver;

pub use browser::MdnsDiscoveryService;
pub use model::{MdnsServiceKind, ResolvedMdnsService};

#[doc(hidden)]
#[allow(dead_code)]
pub mod testing {
    use std::collections::BTreeMap;
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
        pub system_pairing_identity: Option<String>,
        pub raop_codecs: Option<String>,
        pub raop_encryption_types: Option<String>,
        pub raop_transport: Option<String>,
        pub raop_metadata_types: Option<String>,
        pub group_public_name: Option<String>,
        pub group_id: Option<String>,
        pub home_group_id: Option<String>,
        pub household_id: Option<String>,
        pub parent_group_id: Option<String>,
    }

    impl MdnsTestResolvedService {
        fn into_resolved_service(self) -> ResolvedMdnsService {
            let mut txt_records = BTreeMap::new();
            insert_txt_record(&mut txt_records, "deviceid", self.device_id);
            insert_txt_record(&mut txt_records, "pi", self.pairing_id);
            insert_txt_record(&mut txt_records, "am", self.model_or_am);
            insert_txt_record(&mut txt_records, "features", self.features);
            insert_txt_record(&mut txt_records, "flags", self.flags);
            insert_txt_record(&mut txt_records, "srcvers", self.srcvers);
            insert_txt_record(&mut txt_records, "pk", self.receiver_public_key);
            insert_txt_record(&mut txt_records, "psi", self.system_pairing_identity);
            insert_txt_record(&mut txt_records, "cn", self.raop_codecs);
            insert_txt_record(&mut txt_records, "et", self.raop_encryption_types);
            insert_txt_record(&mut txt_records, "tp", self.raop_transport);
            insert_txt_record(&mut txt_records, "md", self.raop_metadata_types);
            insert_txt_record(&mut txt_records, "gpn", self.group_public_name);
            insert_txt_record(&mut txt_records, "gid", self.group_id);
            insert_txt_record(&mut txt_records, "hgid", self.home_group_id);
            insert_txt_record(&mut txt_records, "hmid", self.household_id);
            insert_txt_record(&mut txt_records, "pgid", self.parent_group_id);

            ResolvedMdnsService {
                service_kind: match self.service_kind {
                    MdnsTestServiceKind::Raop => MdnsServiceKind::Raop,
                    MdnsTestServiceKind::AirPlay => MdnsServiceKind::AirPlay,
                },
                fullname: self.fullname,
                port: self.port,
                ipv4_addresses: self.ipv4_addresses,
                txt_records,
            }
        }
    }

    fn insert_txt_record(records: &mut BTreeMap<String, String>, key: &str, value: Option<String>) {
        if let Some(value) = value {
            records.insert(String::from(key), value);
        }
    }

    #[must_use]
    pub fn parse_test_services(services: Vec<MdnsTestResolvedService>) -> Vec<Receiver> {
        parse_resolved_services(
            services
                .into_iter()
                .map(MdnsTestResolvedService::into_resolved_service)
                .collect(),
        )
    }
}

pub trait DiscoveryService {
    fn discover_devices(&self) -> Vec<Receiver>;
}
