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
    pub device_id: Option<String>,
    pub pairing_id: Option<String>,
    pub model_or_am: Option<String>,
    pub features: Option<String>,
    pub flags: Option<String>,
    pub srcvers: Option<String>,
    pub receiver_public_key: Option<String>,
}
