//! `RAOP` RTP packet 与计数器模型。

/// `RAOP` 音频流的 RTP 计数器。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RaopPacketCounters {
    pub next_sequence: u16,
    pub next_timestamp: u32,
}

impl RaopPacketCounters {
    #[must_use]
    pub fn new(initial_sequence: u16, initial_timestamp: u32) -> Self {
        Self {
            next_sequence: initial_sequence,
            next_timestamp: initial_timestamp,
        }
    }

    #[must_use]
    pub fn peek_audio_packet(self) -> (u16, u32) {
        (self.next_sequence, self.next_timestamp)
    }

    pub fn allocate_audio_packet(&mut self, frames: usize) -> Result<(u16, u32), &'static str> {
        let frames = u32::try_from(frames)
            .map_err(|_| "audio frame count exceeds RAOP RTP timestamp range")?;
        let sequence = self.next_sequence;
        let timestamp = self.next_timestamp;

        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.next_timestamp = self.next_timestamp.wrapping_add(frames);

        Ok((sequence, timestamp))
    }
}

/// 发送到音频数据端口的最小 RTP 包模型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtpAudioPacket {
    pub marker: bool,
    pub sequence: u16,
    pub timestamp: u32,
    pub payload_type: u8,
    pub ssrc: u32,
    pub payload: Vec<u8>,
}

impl RtpAudioPacket {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(12 + self.payload.len());
        bytes.push(0x80);
        bytes.push((u8::from(self.marker) << 7) | (self.payload_type & 0x7f));
        bytes.extend_from_slice(&self.sequence.to_be_bytes());
        bytes.extend_from_slice(&self.timestamp.to_be_bytes());
        bytes.extend_from_slice(&self.ssrc.to_be_bytes());
        bytes.extend_from_slice(&self.payload);
        bytes
    }
}

/// 发送到控制端口的最小同步包模型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RaopSyncPacket {
    pub first_packet_in_stream: bool,
    pub sequence: u16,
    pub ntp_timestamp: u64,
    pub rtp_timestamp: u32,
    pub next_rtp_timestamp: u32,
}

impl RaopSyncPacket {
    #[must_use]
    pub fn encode(&self) -> [u8; 20] {
        let mut bytes = [0_u8; 20];
        bytes[0] = if self.first_packet_in_stream {
            0x90
        } else {
            0x80
        };
        bytes[1] = 0x80 | 0x54;
        bytes[2..4].copy_from_slice(&self.sequence.to_be_bytes());
        bytes[4..8].copy_from_slice(&self.rtp_timestamp.to_be_bytes());
        bytes[8..16].copy_from_slice(&self.ntp_timestamp.to_be_bytes());
        bytes[16..20].copy_from_slice(&self.next_rtp_timestamp.to_be_bytes());
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::{RaopPacketCounters, RaopSyncPacket, RtpAudioPacket};

    #[test]
    fn packet_counters_peek_next_packet_without_advancing() {
        let counters = RaopPacketCounters::new(7, 11);

        assert_eq!(counters.peek_audio_packet(), (7, 11));
        assert_eq!(counters.peek_audio_packet(), (7, 11));
    }

    #[test]
    fn packet_counters_advance_with_frame_count() {
        let mut counters = RaopPacketCounters::new(7, 11);

        assert_eq!(counters.allocate_audio_packet(352).unwrap(), (7, 11));
        assert_eq!(counters.allocate_audio_packet(352).unwrap(), (8, 363));
    }

    #[test]
    fn packet_counters_wrap_sequence_number() {
        let mut counters = RaopPacketCounters::new(u16::MAX, 1);

        assert_eq!(counters.allocate_audio_packet(1).unwrap(), (u16::MAX, 1));
        assert_eq!(counters.allocate_audio_packet(1).unwrap(), (0, 2));
    }

    #[test]
    fn rtp_audio_packet_encode_writes_standard_header() {
        let packet = RtpAudioPacket {
            marker: true,
            sequence: 0x1234,
            timestamp: 0x0102_0304,
            payload_type: 96,
            ssrc: 0x5566_7788,
            payload: vec![1, 2, 3, 4],
        };

        let bytes = packet.encode();

        assert_eq!(
            bytes[..12],
            [
                0x80, 0xe0, 0x12, 0x34, 0x01, 0x02, 0x03, 0x04, 0x55, 0x66, 0x77, 0x88
            ]
        );
        assert_eq!(&bytes[12..], &[1, 2, 3, 4]);
    }

    #[test]
    fn sync_packet_encode_writes_short_raop_header() {
        let packet = RaopSyncPacket {
            first_packet_in_stream: true,
            sequence: 0x1234,
            ntp_timestamp: 0x0102_0304_0506_0708,
            rtp_timestamp: 0x1112_1314,
            next_rtp_timestamp: 0x2122_2324,
        };

        let bytes = packet.encode();

        assert_eq!(bytes[0], 0x90);
        assert_eq!(bytes[1], 0xd4);
        assert_eq!(&bytes[2..4], &[0x12, 0x34]);
        assert_eq!(&bytes[4..8], &[0x11, 0x12, 0x13, 0x14]);
        assert_eq!(&bytes[8..16], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(&bytes[16..20], &[0x21, 0x22, 0x23, 0x24]);
    }
}
