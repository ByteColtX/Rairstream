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

## Development

```bash
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

## License

Apache-2.0
