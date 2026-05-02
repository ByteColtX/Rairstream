//! 会话准备入口与 modern `AirPlay` 认证外壳。

use super::raop::{RaopConnection, RaopSession};
use super::{AirPlayError, SessionDescriptor};
use crate::pairing::ReceiverCredentials;
use crate::receiver::ReceiverKind;
use crate::rtsp::{
    RtspRequest, build_auth_setup_request, build_info_request, build_pair_pin_start_request,
    build_pair_setup_pin_request, build_pair_setup_request, build_pair_verify_request,
};

/// 进入握手前的会话准备结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreparedSession {
    ClassicRaop(RaopSession),
    ModernAirPlay(ModernAirPlaySession),
}

impl PreparedSession {
    pub fn prepare(descriptor: &SessionDescriptor) -> Result<Self, AirPlayError> {
        match descriptor.device.receiver_kind {
            ReceiverKind::ClassicRaop => Ok(Self::ClassicRaop(RaopSession::connect(descriptor)?)),
            ReceiverKind::ModernAirPlayAuth => Ok(Self::ModernAirPlay(
                ModernAirPlaySession::connect(descriptor)?,
            )),
        }
    }

    #[must_use]
    pub fn transport_name(&self) -> &'static str {
        match self {
            Self::ClassicRaop(_) => RaopSession::transport_name(),
            Self::ModernAirPlay(_) => ModernAirPlaySession::transport_name(),
        }
    }

    pub fn handshake(self) -> Result<SessionConnection, AirPlayError> {
        match self {
            Self::ClassicRaop(session) => Ok(SessionConnection::ClassicRaop(Box::new(
                session.handshake()?,
            ))),
            Self::ModernAirPlay(session) => {
                Ok(SessionConnection::ModernAirPlay(Box::new(session.handshake()?)))
            }
        }
    }

    pub fn pair_with_pin(self, pin: &str) -> Result<ReceiverCredentials, AirPlayError> {
        match self {
            Self::ClassicRaop(_) => Err(AirPlayError::AuthenticationRequired),
            Self::ModernAirPlay(session) => session.pair_with_pin(pin),
        }
    }

    pub fn request_pairing_pin_display(self) -> Result<(), AirPlayError> {
        match self {
            Self::ClassicRaop(_) => Err(AirPlayError::AuthenticationRequired),
            Self::ModernAirPlay(session) => session.request_pairing_pin_display(),
        }
    }
}

/// 握手完成后的连接句柄。
#[derive(Debug)]
pub enum SessionConnection {
    ClassicRaop(Box<RaopConnection>),
    ModernAirPlay(Box<ModernAirPlayConnection>),
}

impl SessionConnection {
    pub fn teardown(self) -> Result<(), AirPlayError> {
        match self {
            Self::ClassicRaop(connection) => connection.teardown(),
            Self::ModernAirPlay(connection) => connection.teardown(),
        }
    }

    pub fn stream_transport(&self) -> Result<crate::transport::RaopStreamTransport, AirPlayError> {
        match self {
            Self::ClassicRaop(connection) => connection.stream_transport(),
            Self::ModernAirPlay(connection) => connection.stream_transport(),
        }
    }

    #[must_use]
    pub fn is_terminated(&self) -> bool {
        match self {
            Self::ClassicRaop(connection) => connection.is_terminated(),
            Self::ModernAirPlay(connection) => connection.is_terminated(),
        }
    }
}

/// `AirPlay` Receiver 认证阶段的最小准备态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModernAirPlaySession {
    descriptor: SessionDescriptor,
    cseq: u32,
}

impl ModernAirPlaySession {
    #[must_use]
    pub fn transport_name() -> &'static str {
        "airplay2"
    }

    pub fn connect(descriptor: &SessionDescriptor) -> Result<Self, AirPlayError> {
        descriptor.validate()?;
        Ok(Self {
            descriptor: descriptor.clone(),
            cseq: 0,
        })
    }

    #[must_use]
    pub fn descriptor(&self) -> &SessionDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub fn info_request(&mut self) -> RtspRequest {
        let cseq = self.next_cseq();
        build_info_request(&self.descriptor, cseq)
    }

    #[must_use]
    pub fn pair_setup_request(&mut self, body: impl Into<Vec<u8>>) -> RtspRequest {
        let cseq = self.next_cseq();
        build_pair_setup_request(&self.descriptor, cseq, "application/octet-stream", body)
    }

    #[must_use]
    pub fn pair_pin_start_request(&mut self) -> RtspRequest {
        let cseq = self.next_cseq();
        build_pair_pin_start_request(&self.descriptor, cseq)
    }

    #[must_use]
    pub fn pair_setup_pin_request(&mut self, body: impl Into<Vec<u8>>) -> RtspRequest {
        let cseq = self.next_cseq();
        build_pair_setup_pin_request(&self.descriptor, cseq, body)
    }

    #[must_use]
    pub fn pair_verify_request(&mut self, body: impl Into<Vec<u8>>) -> RtspRequest {
        let cseq = self.next_cseq();
        build_pair_verify_request(&self.descriptor, cseq, "application/octet-stream", body)
    }

    #[must_use]
    pub fn auth_setup_request(&mut self, body: impl Into<Vec<u8>>) -> RtspRequest {
        let cseq = self.next_cseq();
        build_auth_setup_request(&self.descriptor, cseq, body)
    }

    fn next_cseq(&mut self) -> u32 {
        self.cseq = self.cseq.saturating_add(1);
        self.cseq
    }

    #[must_use]
    pub(crate) fn cseq(&self) -> u32 {
        self.cseq
    }
}

/// `AirPlay` Receiver 认证完成后的连接句柄。
#[derive(Debug)]
pub struct ModernAirPlayConnection {
    raop_connection: RaopConnection,
}

impl ModernAirPlayConnection {
    pub(crate) fn from_raop(raop_connection: RaopConnection) -> Self {
        Self { raop_connection }
    }

    #[must_use]
    pub fn session(&self) -> &RaopSession {
        self.raop_connection.session()
    }

    pub fn teardown(self) -> Result<(), AirPlayError> {
        self.raop_connection.teardown()
    }

    pub fn stream_transport(&self) -> Result<crate::transport::RaopStreamTransport, AirPlayError> {
        self.raop_connection.stream_transport()
    }

    #[must_use]
    pub fn is_terminated(&self) -> bool {
        self.raop_connection.is_terminated()
    }
}
