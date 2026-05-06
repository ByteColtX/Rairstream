<a id="readme-top"></a>

[![Contributors][contributors-shield]][contributors-url]
[![Forks][forks-shield]][forks-url]
[![Stargazers][stars-shield]][stars-url]
[![Issues][issues-shield]][issues-url]
[![License][license-shield]][license-url]

<div align="center">
  <h3 align="center">Rairstream</h3>
  <p align="center">
    CLI-first Rust project for AirPlay / RAOP audio sending
    <br />
    Currently validated for discovery, AirPlay 2 receiver pairing, local file playback, and Windows real-time system audio streaming
    <br />
    Future GUI / TUI frontends are expected to enter through a stable facade layer
    <br />
    <a href="./README.md"><strong>中文说明</strong></a>
    ·
    <a href="https://github.com/ByteColtX/Rairstream/issues">Report Bug</a>
    ·
    <a href="https://github.com/ByteColtX/Rairstream/issues">Request Feature</a>
  </p>
</div>

## Table of Contents

- [About The Project](#about-the-project)
  - [Current Stage](#current-stage)
  - [Built With](#built-with)
- [Getting Started](#getting-started)
  - [Prerequisites](#prerequisites)
  - [Installation](#installation)
    - [Option A: Download A Prebuilt Binary](#option-a-download-a-prebuilt-binary)
    - [Option B: Build From Source](#option-b-build-from-source)
- [Usage](#usage)
  - [Discover And Inspect Receivers](#discover-and-inspect-receivers)
  - [Pair For The First Time](#pair-for-the-first-time)
  - [Play A Local Audio File](#play-a-local-audio-file)
  - [Stream System Audio In Real Time](#stream-system-audio-in-real-time)
  - [Selector Rules](#selector-rules)
  - [Configuration](#configuration)
- [Roadmap](#roadmap)
- [Contributing](#contributing)
  - [Contribution Requirements](#contribution-requirements)
- [License](#license)
- [Contact](#contact)
- [Acknowledgments](#acknowledgments)

## About The Project

Rairstream is intended to be a clear, maintainable, and extensible Rust codebase for AirPlay / RAOP audio sending.

The repository is no longer organized around the old tray-first shape. The current direction is to harden the CLI path first, then stabilize the boundaries between discovery, pairing, session, audio, RTSP, and timing modules. That gives future GUI, TUI, and broader platform work a stable surface without coupling frontends directly to protocol internals.

The currently validated paths are:

| Capability | Status |
| --- | --- |
| receiver discovery over `_raop` / `_airplay` | validated |
| receiver metadata inspection through `inspect` | validated |
| first-time PIN pairing for AirPlay 2 receivers | validated |
| restoring saved credentials | validated |
| local audio file playback | validated |
| Windows WASAPI loopback real-time system audio streaming | validated |
| multi-device fan-out | unverified |

### Current Stage

- the active entrypoint today is the CLI
- future GUI / TUI frontends are expected to go through `src/app`
- Windows is the primary validated platform right now
- the project currently focuses on AirPlay audio sending, not video or screen mirroring
- the active transport codec is still fixed `PCM/L16`
- `ALAC` and `AAC` are already represented in capability modeling, but they are not the active sender path yet

<p align="right">(<a href="#readme-top">back to top</a>)</p>

### Built With

- [Rust](https://www.rust-lang.org/)
- [mdns-sd](https://crates.io/crates/mdns-sd)
- [symphonia](https://crates.io/crates/symphonia)
- [wasapi](https://crates.io/crates/wasapi) for Windows capture
- `serde` / `tracing`

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Getting Started

### Prerequisites

- Rust `1.85+`
- sender and receivers on the same local network
- Windows is currently required for `play capture`

### Installation

#### Option A: Download A Prebuilt Binary

If you just want to use the CLI instead of building from source, prefer the prebuilt binaries from GitHub Releases:

- download page: [Releases](https://github.com/ByteColtX/Rairstream/releases)
- Windows x64: `rairstream-vX.Y.Z-windows-x86_64.zip`
- Windows ARM64: `rairstream-vX.Y.Z-windows-arm64.zip`
- checksum file: matching `.sha256`

After extracting the archive, you can run `rairstream.exe` directly, or add its directory to `PATH` and use `rairstream`.

#### Option B: Build From Source

If you want to build it yourself or contribute to development:

1. Clone the repository

   ```bash
   git clone https://github.com/ByteColtX/Rairstream.git
   cd Rairstream
   ```

2. Build the project

   ```bash
   cargo build --release
   ```

3. Run the locally built binary

   ```bash
   target\release\rairstream.exe discover
   ```

4. If you prefer starting through Cargo, you can also use:

   ```bash
   cargo run -- discover
   ```

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Usage

Current CLI surface:

```bash
rairstream [-v|-vv] [--log-level <error|warn|info|debug|trace>] <command>
```

### Discover And Inspect Receivers

Discover receivers:

```bash
rairstream discover
```

Example output:

```text
╭─ Living Room  ✓ Supported
│ ID:       living-room
│ Endpoint: 192.168.1.20:7000
│ Profile:  Modern Auth RAOP
│ Auth:     HomeKit transient
│ Pairing:  PIN or credentials
│ Codecs:   L16, ALAC
╰─
```

Inspect receiver details:

```bash
rairstream inspect --device 001122334455
```

`inspect` reports receiver metadata such as profile, auth method, pairing mode, codecs, support state, and grouping-related fields.

### Pair For The First Time

Interactive pairing:

```bash
rairstream pair --device 001122334455
```

This requests the receiver to show a PIN, then prompts for that PIN in the terminal.

Non-interactive pairing:

```bash
rairstream pair --device 001122334455 --pin 123456
```

List saved pairings:

```bash
rairstream paired list
```

Forget a saved pairing:

```bash
rairstream paired forget --device 001122334455
```

### Play A Local Audio File

```bash
rairstream play file "%WINDIR%\Media\Alarm01.wav" --device 001122334455
```

Multi-device playback:

```bash
rairstream play file "%WINDIR%\Media\Alarm01.wav" --device "Living Room" --device "Kitchen"
```

### Stream System Audio In Real Time

```bash
rairstream play capture --device 001122334455
```

This currently uses Windows `WASAPI shared loopback`. Press `Ctrl+C` to stop the session.

### Selector Rules

`selector` currently supports:

- receiver name
- receiver id
- colon-delimited receiver id such as `00:11:22:33:44:55`
- host such as `192.168.1.20`
- `host:port` such as `192.168.1.20:7000`

Resolution prefers:

- exact matches first
- a partial name match only when no exact match exists

### Configuration

Default config path:

- Windows: `%APPDATA%\Rairstream\config.json`
- Linux / Unix: `$XDG_CONFIG_HOME/rairstream/config.json`
- if `XDG_CONFIG_HOME` is not set: `~/.config/rairstream/config.json`

The current config stores:

- paired receiver credentials
- receiver cache
- sender volume configuration

The current send path is:

- output codec: `L16`
- output sample rate: `44.1 kHz`
- output bit depth: `16-bit`
- output channels: `2-channel stereo`
- multi-channel input is downmixed to stereo before transmission

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Roadmap

- [ ] add stable GUI / TUI entrypoints above the app facade
- [ ] add non-Windows capture backends
- [ ] promote `ALAC` / `AAC` from capability metadata into active sender paths
- [ ] continue refining modern receiver session / auth / timing flows
- [ ] expand the real-device validation matrix

For ongoing and future work, see [Issues](https://github.com/ByteColtX/Rairstream/issues).

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Contributing

Issues and pull requests are welcome, especially for documentation, tests, protocol cleanup, and platform support.

Suggested flow:

1. Fork the project
2. Create your feature branch
3. Commit your changes
4. Push the branch
5. Open a Pull Request

### Contribution Requirements

Before opening a pull request, please run at least:

```bash
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

Additional expectations:

- if you change CLI behavior, user-facing output, or command structure, update the README and related tests in the same change
- keep runtime log output in English
- preserve the current module boundaries instead of leaking UI or platform details back into the protocol core

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## License

Distributed under Apache-2.0. See [LICENSE](./LICENSE) for more information.

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Contact

- GitHub: [@ByteColtX](https://github.com/ByteColtX)
- Project Link: [https://github.com/ByteColtX/Rairstream](https://github.com/ByteColtX/Rairstream)

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Acknowledgments

- [Best-README-Template](https://github.com/othneildrew/Best-README-Template)
- [symphonia](https://github.com/pdeljanov/Symphonia)
- [mdns-sd](https://github.com/keepsimple1/mdns-sd)
- [wasapi-rs](https://github.com/HEnquist/wasapi-rs)

<p align="right">(<a href="#readme-top">back to top</a>)</p>

[contributors-shield]: https://img.shields.io/github/contributors/ByteColtX/Rairstream.svg?style=for-the-badge
[contributors-url]: https://github.com/ByteColtX/Rairstream/graphs/contributors
[forks-shield]: https://img.shields.io/github/forks/ByteColtX/Rairstream.svg?style=for-the-badge
[forks-url]: https://github.com/ByteColtX/Rairstream/network/members
[stars-shield]: https://img.shields.io/github/stars/ByteColtX/Rairstream.svg?style=for-the-badge
[stars-url]: https://github.com/ByteColtX/Rairstream/stargazers
[issues-shield]: https://img.shields.io/github/issues/ByteColtX/Rairstream.svg?style=for-the-badge
[issues-url]: https://github.com/ByteColtX/Rairstream/issues
[license-shield]: https://img.shields.io/github/license/ByteColtX/Rairstream.svg?style=for-the-badge
[license-url]: https://github.com/ByteColtX/Rairstream/blob/main/LICENSE
