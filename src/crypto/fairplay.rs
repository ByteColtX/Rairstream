use crate::error::RairstreamError;

pub fn unsupported() -> RairstreamError {
    RairstreamError::InvalidInput {
        message: String::from("FairPlay is not implemented in this refactor"),
    }
}
