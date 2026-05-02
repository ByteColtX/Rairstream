use rairstream::audio::{AudioChunk, AudioFormat, AudioResampler, AudioSampleType};

#[test]
fn audio_resampler_emits_raop_sized_packets() {
    let format = AudioFormat {
        sample_rate_hz: 44_100,
        channels: 2,
        bits_per_sample: 16,
        sample_type: AudioSampleType::Int,
    };
    let mut bytes = Vec::new();
    for _ in 0..352 {
        bytes.extend_from_slice(&1_000_i16.to_le_bytes());
        bytes.extend_from_slice(&(-1_000_i16).to_le_bytes());
    }
    let chunk = AudioChunk::new(format, bytes).unwrap();
    let mut resampler = AudioResampler::new(format);

    let packets = resampler.push_chunk(&chunk).unwrap();

    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].len(), 352 * 4);
}

#[test]
fn audio_resampler_buffers_partial_chunks_until_packet_is_full() {
    let format = AudioFormat::default();
    let mut resampler = AudioResampler::new(format);
    let mut first_bytes = Vec::new();
    let mut second_bytes = Vec::new();

    for _ in 0..200 {
        first_bytes.extend_from_slice(&1_000_i16.to_le_bytes());
        first_bytes.extend_from_slice(&(-1_000_i16).to_le_bytes());
    }
    for _ in 0..152 {
        second_bytes.extend_from_slice(&1_000_i16.to_le_bytes());
        second_bytes.extend_from_slice(&(-1_000_i16).to_le_bytes());
    }

    let first_chunk = AudioChunk::new(format, first_bytes).unwrap();
    let second_chunk = AudioChunk::new(format, second_bytes).unwrap();

    let first_packets = resampler.push_chunk(&first_chunk).unwrap();
    let second_packets = resampler.push_chunk(&second_chunk).unwrap();

    assert!(first_packets.is_empty());
    assert_eq!(second_packets.len(), 1);
    assert_eq!(second_packets[0].len(), 352 * 4);
}
