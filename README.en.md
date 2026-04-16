# Rairstream

> Real-time AirPlay audio streaming for Windows.

[中文说明](./README.md)

Rairstream is a native Rust desktop app that streams any Windows audio directly to AirPlay-compatible receivers, including browser audio, music, games, and system sound, without extra forwarding software.

## Status

Rairstream is under active development, but the core end-to-end path is already working:

- receiver discovery
- device selection from the system tray
- PIN pairing for receivers that require authentication
- audio streaming from WASAPI loopback
- stop and teardown cleanup

## Supported receiver types

| Receiver type | Examples | Status |
| --- | --- | --- |
| AirPlay 1 / RAOP | AirPort Express, older speakers / AVRs, `shairport-sync`-style receivers | Supported |
| AirPlay Receiver | macOS AirPlay Receiver, Apple TV-class receivers that require pairing or authentication | Supported |
| Full AirPlay 2 feature set | Multi-room playback, grouped playback, full AirPlay 2 compatibility | Not implemented |

## Current capabilities

- Discover receivers over `_raop._tcp.local.` and `_airplay._tcp.local.`
- Stream Windows system audio to classic AirPlay 1 / RAOP receivers
- Pair AirPlay Receiver devices with a PIN
- Persist pairing credentials and restore authentication on later connections
- Start, stop, and switch devices from a system tray app
- Run a minimal `smoke` mode from the CLI for debugging
- The current sender path outputs fixed `44.1 kHz / 16-bit / 2-channel PCM (L16/44100/2)`; multi-channel input is downmixed to stereo before transmission

### Current audio profile

| Item | Current value |
| --- | --- |
| Capture backend | `Windows WASAPI shared loopback capture` (event-driven) |
| Output codec | `L16` |
| Output sample rate | `44.1 kHz` |
| Output bit depth | `16-bit` |
| Output channels | `2` |
| RTP frames per packet | `352` frames |
| Startup latency | `11025` frames (about `250 ms @ 44.1 kHz`) |
| Multi-channel input handling | Downmixed to stereo |

## Limitations

- Windows only
- Audio streaming only — no video or screen mirroring
- Compatibility depends on receiver model and firmware
- AirPlay 2 devices may work through supported receiver paths, but Rairstream is not a full AirPlay 2 implementation

## Quick start

### Requirements

- Windows 11 or another Windows version with WASAPI loopback support
- Rust 1.85+
- A receiver on the same local network

### Run the tray app

```bash
cargo run
```

### Run smoke mode

```bash
cargo run -- smoke
cargo run -- smoke "Living Room"
```

## CLI

```bash
rairstream [OPTIONS] [smoke [DEVICE_FILTER]]
```

- default: start tray mode
- `smoke`: connect to the first discovered receiver
- `smoke <DEVICE_FILTER>`: match by device name, ID, or host
- `-v` / `-vv`: enable debug / trace logging
- `--log-level <error|warn|info|debug|trace>`: set the log level explicitly

## Roadmap / TODO

### Audio

- [x] Add sender-side volume control in the tray UI with a `0–100%` range
- [x] Add a mute toggle
- [ ] Evaluate receiver-side volume sync and dB mapping
- [x] Keep the current fixed sender profile of `44.1 kHz / 16-bit / 2-channel PCM (L16/44100/2)` while evaluating configurable output profiles
- [x] Explicitly cover `44.1 kHz` and `48 kHz` input paths
- [x] Explicitly cover `16-bit / 24-bit / 32-bit` input format conversion
- [x] Define handling for `1 / 2 / 6 / 8` channel input layouts (multi-channel input is currently downmixed to `2` channels)
- [ ] Evaluate reducing startup latency from the current `11025` frames (about `250 ms @ 44.1 kHz`)
- [ ] Continue improving buffering, keepalive behavior, and long-run stability

### Capture

- [ ] Add `WASAPI process loopback` alongside the current `WASAPI shared loopback`
- [ ] Support per-app audio capture, such as streaming only a browser, player, or game

### Desktop UX

- [x] Auto reconnect
- [x] Remember the last-used device
- [ ] Launch at startup
- [x] Start minimized to tray
- [x] Clearer recovery paths after pairing / authentication failures

### Compatibility & validation

- [ ] Expand real-device validation and maintain a list of verified receivers, starting with Apple TV, HomePod, and third-party speakers / AVRs
- [ ] Continue improving compatibility across different AirPlay Receiver implementations
- [ ] Evaluate broader AirPlay 2 receiver compatibility on top of the current audio streaming path

## Development

```bash
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

## License

Apache-2.0
