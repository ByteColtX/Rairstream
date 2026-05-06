<a id="readme-top"></a>

[![Contributors][contributors-shield]][contributors-url]
[![Forks][forks-shield]][forks-url]
[![Stargazers][stars-shield]][stars-url]
[![Issues][issues-shield]][issues-url]
[![License][license-shield]][license-url]

<div align="center">
  <h3 align="center">Rairstream</h3>
  <p align="center">
    面向 AirPlay / RAOP 音频发送的 CLI-first Rust 工程
    <br />
    当前已验证设备发现、AirPlay 2 接收端配对、本地文件播放，以及 Windows 实时系统音频串流
    <br />
    未来 GUI / TUI 将通过统一 facade 接入
    <br />
    <a href="./README.en.md"><strong>English README</strong></a>
    ·
    <a href="https://github.com/ByteColtX/Rairstream/issues">报告问题</a>
    ·
    <a href="https://github.com/ByteColtX/Rairstream/issues">功能建议</a>
  </p>
</div>

## 目录

- [关于项目](#关于项目)
  - [当前阶段](#当前阶段)
  - [技术栈](#技术栈)
- [开始使用](#开始使用)
  - [环境要求](#环境要求)
  - [安装](#安装)
    - [方式 A：下载预编译二进制](#方式-a下载预编译二进制)
    - [方式 B：从源码构建](#方式-b从源码构建)
- [使用方式](#使用方式)
  - [发现与检查设备](#发现与检查设备)
  - [首次配对](#首次配对)
  - [播放本地音频文件](#播放本地音频文件)
  - [实时串流系统音频](#实时串流系统音频)
  - [Selector 规则](#selector-规则)
  - [配置文件](#配置文件)
- [路线图](#路线图)
- [贡献](#贡献)
  - [贡献要求](#贡献要求)
- [许可证](#许可证)
- [联系方式](#联系方式)
- [致谢](#致谢)

## 关于项目

Rairstream 的目标，是把 AirPlay / RAOP 音频发送整理成一套清晰、可维护、可继续扩展的 Rust 工程。

当前仓库已经不再围绕旧的 tray 形态组织，而是先把 CLI 主链路跑通，再把 discovery、pairing、session、audio、rtsp、timing 等核心模块边界稳定下来。这样后续无论补 GUI、TUI，还是继续扩展到更多平台，都不需要直接碰协议细节。

当前已验证的主链路如下：

| 能力 | 状态 |
| --- | --- |
| `_raop` / `_airplay` 发现接收端 | 已验证 |
| `inspect` 查看 receiver 元数据 | 已验证 |
| AirPlay 2 接收端首次 PIN 配对 | 已验证 |
| 恢复已保存凭据 | 已验证 |
| 本地音频文件播放 | 已验证 |
| Windows WASAPI loopback 实时系统音频串流 | 已验证 |
| 多设备 fan-out | 未验证 |

### 当前阶段

- 当前主入口是 CLI
- 未来 GUI / TUI 会通过 `src/app` facade 接入，而不是直接耦合协议层
- 当前主验证平台是 Windows
- 当前项目聚焦 AirPlay 音频发送，不覆盖视频或屏幕镜像
- 当前活跃发送 codec 仍是固定 `PCM/L16`
- `ALAC` / `AAC` 已进入 capability 模型，但还不是当前活跃发送路径

<p align="right">(<a href="#readme-top">back to top</a>)</p>

### 技术栈

- [Rust](https://www.rust-lang.org/)
- [mdns-sd](https://crates.io/crates/mdns-sd)
- [symphonia](https://crates.io/crates/symphonia)
- [wasapi](https://crates.io/crates/wasapi)（Windows capture）
- `serde` / `tracing`

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## 开始使用

### 环境要求

- Rust `1.85+`
- 发送端与接收端位于同一局域网
- 若使用 `play capture`，当前必须运行在 Windows

### 安装

#### 方式 A：下载预编译二进制

如果你只是希望直接使用 CLI，而不是参与开发，优先使用 GitHub Releases 中的预编译二进制包：

- 下载页面：[Releases](https://github.com/ByteColtX/Rairstream/releases)
- Windows x64：`rairstream-vX.Y.Z-windows-x86_64.zip`
- Windows ARM64：`rairstream-vX.Y.Z-windows-arm64.zip`
- 校验文件：对应的 `.sha256`

解压后可直接运行 `rairstream.exe`，也可以把它所在目录加入 `PATH` 后直接使用 `rairstream`。

#### 方式 B：从源码构建

如果你希望自行构建或参与开发：

1. 克隆仓库

   ```bash
   git clone https://github.com/ByteColtX/Rairstream.git
   cd Rairstream
   ```

2. 构建项目

   ```bash
   cargo build --release
   ```

3. 直接运行本地构建出的二进制

   ```bash
   target\release\rairstream.exe discover
   ```

4. 如果你更习惯通过 Cargo 启动，也可以使用：

   ```bash
   cargo run -- discover
   ```

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## 使用方式

当前 CLI 入口如下：

```bash
rairstream [-v|-vv] [--log-level <error|warn|info|debug|trace>] <command>
```

### 发现与检查设备

发现设备：

```bash
rairstream discover
```

示例输出：

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

检查设备详情：

```bash
rairstream inspect --device 001122334455
```

`inspect` 会输出 receiver 的 profile、auth、pairing、codecs、support、group 等元数据。

### 首次配对

交互式配对：

```bash
rairstream pair --device 001122334455
```

这条命令会先请求接收端显示 PIN，然后在终端中提示输入 PIN。

非交互式配对：

```bash
rairstream pair --device 001122334455 --pin 123456
```

查看已保存配对：

```bash
rairstream paired list
```

移除已保存配对：

```bash
rairstream paired forget --device 001122334455
```

### 播放本地音频文件

```bash
rairstream play file "%WINDIR%\Media\Alarm01.wav" --device 001122334455
```

多设备播放：

```bash
rairstream play file "%WINDIR%\Media\Alarm01.wav" --device "Living Room" --device "Kitchen"
```

### 实时串流系统音频

```bash
rairstream play capture --device 001122334455
```

当前这条路径走的是 Windows `WASAPI shared loopback`。运行后按 `Ctrl+C` 停止。

### Selector 规则

`selector` 当前支持以下匹配方式：

- 设备名
- 设备 ID
- 带冒号的设备 ID，例如 `00:11:22:33:44:55`
- 主机地址，例如 `192.168.1.20`
- `host:port`，例如 `192.168.1.20:7000`

匹配逻辑上：

- 优先使用 exact match
- 如果没有 exact match，再回退到设备名的 partial match

### 配置文件

默认配置文件位置：

- Windows：`%APPDATA%\Rairstream\config.json`
- Linux / Unix：`$XDG_CONFIG_HOME/rairstream/config.json`
- 若未设置 `XDG_CONFIG_HOME`：`~/.config/rairstream/config.json`

当前会持久化：

- 已配对接收端凭据
- 接收端缓存
- 发送端音量配置

当前发送链路：

- 输出 codec：`L16`
- 输出采样率：`44.1 kHz`
- 输出位深：`16-bit`
- 输出声道：`2-channel stereo`
- 多声道输入会在发送前 downmix 到 stereo

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## 路线图

- [ ] 为 GUI / TUI 补稳定前端入口
- [ ] 为非 Windows 平台补 capture backend
- [ ] 让 `ALAC` / `AAC` 成为可用发送路径，而不只是 capability 元数据
- [ ] 继续细化 modern receiver 的 session / auth / timing 路径
- [ ] 扩大真实设备验证矩阵

查看当前与后续工作项，也可以直接进入 [Issues](https://github.com/ByteColtX/Rairstream/issues)。

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## 贡献

如果你希望改进文档、补测试、修复协议细节或扩展平台支持，欢迎提交 Issue 或 Pull Request。

建议流程：

1. Fork 仓库
2. 创建分支
3. 提交修改
4. 推送分支
5. 创建 Pull Request

### 贡献要求

提交前请至少完成以下检查：

```bash
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

额外约定：

- 如果改动了 CLI 命令面、行为或用户可见文案，请同步更新 README 与相关测试
- 保持日志输出为英文
- 新增代码应继续遵守当前模块边界，不把 UI / platform 细节反向带回协议核心

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## 许可证

基于 Apache-2.0 许可证分发。详见 [LICENSE](./LICENSE)。

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## 联系方式

- GitHub: [@ByteColtX](https://github.com/ByteColtX)
- Project Link: [https://github.com/ByteColtX/Rairstream](https://github.com/ByteColtX/Rairstream)

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## 致谢

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
