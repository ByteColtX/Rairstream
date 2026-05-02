//! `RAOP` 音频 `sink`，负责 packet 序列化与 UDP 发送。

use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};

use crate::audio::{
    AudioCaptureError, AudioChunk, AudioResampler, AudioSink, RAOP_FRAMES_PER_PACKET,
};
use crate::timing::clock::ntp_timestamp_now;

use super::packet::{RaopPacketCounters, RaopSyncPacket, RtpAudioPacket};
use tracing::{debug, trace, warn};

const RAOP_AUDIO_PAYLOAD_TYPE: u8 = 96;

#[derive(Debug)]
pub struct RaopStreamTransport {
    pub audio_socket: UdpSocket,
    pub control_socket: UdpSocket,
    pub audio_target: SocketAddr,
    pub control_target: SocketAddr,
    pub audio_ssrc: u32,
    pub packet_counters: RaopPacketCounters,
    pub sink_config: RaopSinkConfig,
}

/// `RAOP` 音频发送端的最小配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RaopSinkConfig {
    pub frames_per_packet: usize,
    pub sync_interval_packets: usize,
    pub sender_volume_percent: u16,
}

impl Default for RaopSinkConfig {
    fn default() -> Self {
        Self {
            frames_per_packet: RAOP_FRAMES_PER_PACKET,
            sync_interval_packets: 125,
            sender_volume_percent: 100,
        }
    }
}

/// 将统一 `AudioChunk` 转成 `RAOP` RTP 包并持续发送到目标设备。
#[derive(Debug)]
pub struct RaopAudioSink {
    resampler: AudioResampler,
    sender_volume_percent: Arc<Mutex<u16>>,
    transport: RaopStreamTransport,
    first_packet_in_stream: bool,
    sent_audio_packets: usize,
}

impl RaopAudioSink {
    #[must_use]
    pub fn new(
        source_format: crate::audio::AudioFormat,
        transport: RaopStreamTransport,
        sender_volume_percent: Arc<Mutex<u16>>,
    ) -> Self {
        debug!(
            sample_rate_hz = source_format.sample_rate_hz,
            channels = source_format.channels,
            bits_per_sample = source_format.bits_per_sample,
            sample_type = ?source_format.sample_type,
            audio_target = %transport.audio_target,
            control_target = %transport.control_target,
            sync_interval_packets = transport.sink_config.sync_interval_packets,
            "initializing RAOP audio sink"
        );
        let mut resampler = AudioResampler::new(source_format);
        let initial_sender_volume_percent = sender_volume_percent
            .lock()
            .map_or(100, |sender_volume_percent| *sender_volume_percent);
        resampler.set_sender_volume_percent(initial_sender_volume_percent);
        Self {
            resampler,
            sender_volume_percent,
            transport,
            first_packet_in_stream: true,
            sent_audio_packets: 0,
        }
    }

    fn sync_sender_volume(&mut self) {
        if let Ok(sender_volume_percent) = self.sender_volume_percent.lock() {
            self.resampler
                .set_sender_volume_percent(*sender_volume_percent);
        }
    }

    fn send_audio_payload(&mut self, payload: Vec<u8>) -> Result<(), AudioCaptureError> {
        let frames = payload.len() / 4;
        let (sequence, timestamp) = self
            .transport
            .packet_counters
            .allocate_audio_packet(frames)
            .map_err(|message| AudioCaptureError::InvalidFormat {
                message: String::from(message),
            })?;
        let packet = RtpAudioPacket {
            marker: self.first_packet_in_stream,
            sequence,
            timestamp,
            payload_type: RAOP_AUDIO_PAYLOAD_TYPE,
            ssrc: self.transport.audio_ssrc,
            payload,
        };
        let bytes = packet.encode();
        let packet_index = self.sent_audio_packets.saturating_add(1);
        if self.sent_audio_packets < 5 {
            debug!(
                packet_index,
                sequence,
                rtp_timestamp = timestamp,
                payload_bytes = bytes.len(),
                target = %self.transport.audio_target,
                "sending RTP audio packet"
            );
        }
        if self.first_packet_in_stream
            || packet_index % self.transport.sink_config.sync_interval_packets == 0
        {
            trace!(
                packet_index,
                sequence,
                rtp_timestamp = timestamp,
                payload_bytes = bytes.len(),
                target = %self.transport.audio_target,
                "sending RTP audio packet summary"
            );
        }
        self.transport
            .audio_socket
            .send_to(&bytes, self.transport.audio_target)
            .map_err(|error| {
                warn!(
                    sequence,
                    rtp_timestamp = timestamp,
                    target = %self.transport.audio_target,
                    error = %error,
                    "failed to send RTP audio packet"
                );
                AudioCaptureError::RuntimeInitialization {
                    message: error.to_string(),
                }
            })?;

        if self.first_packet_in_stream
            || self.sent_audio_packets % self.transport.sink_config.sync_interval_packets == 0
        {
            self.send_sync_packet(sequence, timestamp)?;
        }

        self.first_packet_in_stream = false;
        self.sent_audio_packets = self.sent_audio_packets.saturating_add(1);
        Ok(())
    }

    fn send_sync_packet(&mut self, sequence: u16, timestamp: u32) -> Result<(), AudioCaptureError> {
        let next_rtp_timestamp = self.transport.packet_counters.peek_audio_packet().1;
        let packet = RaopSyncPacket {
            first_packet_in_stream: self.first_packet_in_stream,
            sequence,
            ntp_timestamp: ntp_timestamp_now(),
            rtp_timestamp: timestamp,
            next_rtp_timestamp,
        };
        let bytes = packet.encode();
        if self.sent_audio_packets < 5 {
            debug!(
                sequence,
                rtp_timestamp = timestamp,
                next_rtp_timestamp,
                target = %self.transport.control_target,
                "sending RAOP sync packet"
            );
        }
        trace!(
            sequence,
            rtp_timestamp = timestamp,
            next_rtp_timestamp,
            packet_index = self.sent_audio_packets.saturating_add(1),
            target = %self.transport.control_target,
            "sending RAOP sync packet"
        );
        self.transport
            .control_socket
            .send_to(&bytes, self.transport.control_target)
            .map_err(|error| {
                warn!(
                    sequence,
                    rtp_timestamp = timestamp,
                    target = %self.transport.control_target,
                    error = %error,
                    "failed to send RAOP sync packet"
                );
                AudioCaptureError::RuntimeInitialization {
                    message: error.to_string(),
                }
            })?;
        Ok(())
    }
}

impl AudioSink for RaopAudioSink {
    fn write(&mut self, chunk: AudioChunk) -> Result<(), AudioCaptureError> {
        self.sync_sender_volume();
        let packets = self.resampler.push_chunk(&chunk).map_err(|error| {
            AudioCaptureError::InvalidFormat {
                message: error.to_string(),
            }
        })?;

        for payload in packets {
            self.send_audio_payload(payload)?;
        }

        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use std::net::UdpSocket;
    use std::time::Duration;

    use crate::audio::{AudioChunk, AudioFormat, AudioSampleType, AudioSink};

    use super::{RaopAudioSink, RaopSinkConfig, RaopStreamTransport};
    use crate::transport::packet::RaopPacketCounters;
    use std::sync::{Arc, Mutex};

    #[test]
    fn sink_config_defaults_match_raop_mvp() {
        let config = RaopSinkConfig::default();

        assert_eq!(config.frames_per_packet, 352);
        assert_eq!(config.sync_interval_packets, 125);
        assert_eq!(config.sender_volume_percent, 100);
    }

    #[test]
    fn raop_audio_sink_sends_audio_and_sync_packets() {
        let audio_receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        let control_receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        audio_receiver
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        control_receiver
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let transport = RaopStreamTransport {
            audio_socket: UdpSocket::bind("127.0.0.1:0").unwrap(),
            control_socket: UdpSocket::bind("127.0.0.1:0").unwrap(),
            audio_target: audio_receiver.local_addr().unwrap(),
            control_target: control_receiver.local_addr().unwrap(),
            audio_ssrc: 0x1122_3344,
            packet_counters: RaopPacketCounters::new(7, 11),
            sink_config: RaopSinkConfig {
                frames_per_packet: 352,
                sync_interval_packets: 1,
                sender_volume_percent: 100,
            },
        };
        let format = AudioFormat {
            sample_rate_hz: 48_000,
            channels: 8,
            bits_per_sample: 32,
            sample_type: AudioSampleType::Float,
        };
        let mut sink = RaopAudioSink::new(format, transport, Arc::new(Mutex::new(100)));
        let mut bytes = Vec::new();
        for _ in 0..768 {
            for sample in [0.1_f32, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8] {
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
        }
        let chunk = AudioChunk::new(format, bytes).unwrap();

        sink.write(chunk).unwrap();

        let mut audio_buffer = [0_u8; 1600];
        let mut control_buffer = [0_u8; 64];
        let (audio_len, _) = audio_receiver.recv_from(&mut audio_buffer).unwrap();
        let (control_len, _) = control_receiver.recv_from(&mut control_buffer).unwrap();

        assert!(audio_len > 12);
        assert_eq!(audio_buffer[1] & 0x7f, 96);
        assert_eq!(&audio_buffer[2..4], &7_u16.to_be_bytes());
        assert_eq!(&audio_buffer[4..8], &11_u32.to_be_bytes());
        assert_eq!(&audio_buffer[8..12], &0x1122_3344_u32.to_be_bytes());
        assert_eq!(control_len, 20);
        assert_eq!(control_buffer[1] & 0x7f, 84);
    }
}
