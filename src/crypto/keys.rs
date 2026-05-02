#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionKeys {
    pub control: Vec<u8>,
    pub data: Vec<u8>,
}
