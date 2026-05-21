use std::collections::HashMap;
use std::hash::BuildHasher;
use std::sync::{Arc, Mutex};

use crate::audio::AudioFormat;
use crate::config::MAX_SENDER_VOLUME_PERCENT;
use crate::error::RairstreamError;
use crate::pairing::ReceiverCredentials;
use crate::receiver::Receiver;
use crate::session::{LatencyProfile, PreparedSession, SessionConnection, SessionDescriptor};
use crate::transport::RaopAudioSink;

use super::group::FanoutAudioSink;

pub struct ConnectedReceiver {
    pub connection: SessionConnection,
}

impl ConnectedReceiver {
    pub fn build_sink(
        &self,
        source_format: AudioFormat,
        sender_volume_percent: Arc<Mutex<u16>>,
    ) -> Result<Box<dyn crate::audio::AudioSink + Send>, RairstreamError> {
        let transport = self.connection.stream_transport()?;
        Ok(Box::new(RaopAudioSink::new(
            source_format,
            transport,
            sender_volume_percent,
        )))
    }
}

pub fn connect_receivers<S>(
    receivers: &[Receiver],
    input_format: AudioFormat,
    paired_receivers: &HashMap<String, ReceiverCredentials, S>,
    sender_volume_percent: u16,
    latency_profile: LatencyProfile,
) -> Result<Vec<ConnectedReceiver>, RairstreamError>
where
    S: BuildHasher,
{
    let sender_volume_percent = sender_volume_percent.clamp(100, MAX_SENDER_VOLUME_PERCENT);
    let mut connected = Vec::with_capacity(receivers.len());
    for receiver in receivers {
        let mut descriptor = SessionDescriptor::new(receiver.clone(), input_format);
        descriptor.sender_volume_percent = sender_volume_percent;
        descriptor.latency_profile = latency_profile;
        if let Some(credentials) = paired_receivers.get(&receiver.id).cloned() {
            descriptor = descriptor.with_receiver_credentials(credentials);
        }
        let connection = PreparedSession::prepare(&descriptor)?.handshake()?;
        connected.push(ConnectedReceiver { connection });
    }

    Ok(connected)
}

pub fn pair_receiver_with_pin(
    receiver: &Receiver,
    pin: &str,
    sender_volume_percent: u16,
    existing_credentials: Option<ReceiverCredentials>,
) -> Result<ReceiverCredentials, RairstreamError> {
    let mut descriptor = SessionDescriptor::new(receiver.clone(), AudioFormat::default());
    descriptor.sender_volume_percent = sender_volume_percent.clamp(100, MAX_SENDER_VOLUME_PERCENT);
    if let Some(credentials) = existing_credentials {
        descriptor = descriptor.with_receiver_credentials(credentials);
    }

    PreparedSession::prepare(&descriptor)?
        .pair_with_pin(pin)
        .map_err(Into::into)
}

pub fn request_pairing_pin_display(
    receiver: &Receiver,
    sender_volume_percent: u16,
    existing_credentials: Option<ReceiverCredentials>,
) -> Result<(), RairstreamError> {
    let mut descriptor = SessionDescriptor::new(receiver.clone(), AudioFormat::default());
    descriptor.sender_volume_percent = sender_volume_percent.clamp(100, MAX_SENDER_VOLUME_PERCENT);
    if let Some(credentials) = existing_credentials {
        descriptor = descriptor.with_receiver_credentials(credentials);
    }

    PreparedSession::prepare(&descriptor)?
        .request_pairing_pin_display()
        .map_err(Into::into)
}

pub fn build_group_sink(
    connections: &[ConnectedReceiver],
    source_format: AudioFormat,
    sender_volume_percent: u16,
) -> Result<FanoutAudioSink, RairstreamError> {
    let sender_volume_percent = Arc::new(Mutex::new(
        sender_volume_percent.clamp(100, MAX_SENDER_VOLUME_PERCENT),
    ));
    let sinks = connections
        .iter()
        .map(|connection| connection.build_sink(source_format, Arc::clone(&sender_volume_percent)))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(FanoutAudioSink::new(sinks))
}
