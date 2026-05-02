use std::collections::BTreeMap;
use std::net::Ipv4Addr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MdnsServiceKind {
    Raop,
    AirPlay,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMdnsService {
    pub service_kind: MdnsServiceKind,
    pub fullname: String,
    pub port: u16,
    pub ipv4_addresses: Vec<Ipv4Addr>,
    pub txt_records: BTreeMap<String, String>,
}

impl ResolvedMdnsService {
    #[must_use]
    pub fn txt_value(&self, key: &str) -> Option<&str> {
        self.txt_records.get(key).map(String::as_str)
    }

    #[must_use]
    pub fn txt_value_any<'a>(&'a self, keys: &[&str]) -> Option<&'a str> {
        keys.iter().find_map(|key| self.txt_value(key))
    }
}
