# Rairstream

> 让 Windows 也能将任意系统音频实时串流到 AirPlay 接收端。

[English README](./README.en.md)

Rairstream 是一个面向 Windows 的原生 Rust 桌面应用，可将浏览器、音乐播放器、游戏和系统提示音等任意系统音频直接串流到兼容 AirPlay 的接收端，无需额外转发软件。

## 当前状态

Rairstream 仍在持续开发中，但核心端到端链路已经可用：

- 发现接收端
- 从系统托盘选择设备
- 对需要认证的接收端执行 PIN 配对
- 通过 WASAPI loopback 采集并发送音频
- 停止串流并完成清理

## 支持的接收端类型

| 接收端类型 | 示例 | 状态 |
| --- | --- | --- |
| AirPlay 1 / RAOP | AirPort Express、较老的音箱 / AVR、`shairport-sync` 一类兼容接收端 | 已支持 |
| AirPlay Receiver | macOS AirPlay Receiver、需要配对或认证的 Apple TV 类接收端 | 已支持 |
| 完整 AirPlay 2 特性集 | 多房间播放、分组播放、完整 AirPlay 2 兼容 | 暂未实现 |

## 当前能力

- 通过 `_raop._tcp.local.` 与 `_airplay._tcp.local.` 发现接收端
- 向经典 `AirPlay 1 / RAOP` 接收端发送 Windows 系统音频
- 对 `AirPlay Receiver` 执行 PIN 配对
- 保存配对凭据，并在后续连接中恢复认证
- 通过系统托盘启动、停止和切换设备
- 提供最小化 `smoke` 调试模式
- 当前固定发送格式为 `44.1 kHz / 16-bit / 2 声道 PCM (L16/44100/2)`；多声道输入会在发送前下混为立体声

### 当前音频配置

| 项目 | 当前值 |
| --- | --- |
| 采集后端 | `Windows WASAPI shared loopback capture`（事件驱动） |
| 输出编码 | `L16` |
| 输出采样率 | `44.1 kHz` |
| 输出位深 | `16-bit` |
| 输出声道数 | `2` |
| RTP 每包帧数 | `352` frames |
| 启动延迟 | `11025` frames（约 `250 ms @ 44.1 kHz`） |
| 多声道输入处理 | 下混为立体声 |

## 当前限制

- 仅支持 Windows
- 当前仅支持音频串流，不支持视频或屏幕镜像
- 兼容性仍取决于接收端型号与固件版本
- 某些 AirPlay 2 设备可能可通过已支持的接收路径工作，但 Rairstream 不是完整的 AirPlay 2 实现

## 快速开始

### 环境要求

- Windows 11 或其他支持 WASAPI loopback 的 Windows 版本
- Rust 1.85+
- 与接收端处于同一局域网

### 启动托盘应用

```bash
cargo run
```

### 运行 CLI 模式

```bash
cargo run -- cli discover
cargo run -- cli start --device "Living Room"
```

## CLI

```bash
rairstream [OPTIONS] [tray | cli <discover|start> [--device <DEVICE_FILTER>] [--pin <PIN>]]
```

- 默认：启动托盘模式
- `tray`：显式启动托盘模式
- `cli discover`：发现并列出可用接收端
- `cli start`：不依赖托盘，直接以前台模式启动串流
- `cli start --device <DEVICE_FILTER>`：按设备名、ID 或主机地址匹配
- `cli start --pin <PIN>`：在需要时以非交互方式提供配对 PIN
- `-v` / `-vv`：启用 debug / trace 日志
- `--log-level <error|warn|info|debug|trace>`：显式设置日志级别

## 路线图 / TODO

### 音频

- [ ] 在托盘中增加 `0–100%` 发送端音量控制
- [ ] 增加静音开关
- [ ] 评估接收端音量同步与 dB 映射策略
- [ ] 保持当前固定发送格式 `44.1 kHz / 16-bit / 2 声道 PCM (L16/44100/2)` 的同时，评估可配置输出档位
- [ ] 明确覆盖 `44.1 kHz` 与 `48 kHz` 输入链路
- [ ] 明确覆盖 `16-bit / 24-bit / 32-bit` 输入格式转换
- [ ] 明确覆盖 `1 / 2 / 6 / 8` 声道输入的处理策略（当前多声道会下混为 `2` 声道）
- [ ] 评估将启动延迟从当前 `11025` frames（约 `250 ms @ 44.1 kHz`）进一步收敛
- [ ] 持续优化缓冲、保活与长时间稳定性

### 采集

- [ ] 在当前 `WASAPI shared loopback` 之外，增加 `WASAPI process loopback`
- [ ] 支持按应用选择音频来源（例如仅串流浏览器、播放器或游戏）

### 桌面体验

- [ ] 自动重连
- [ ] 记住上次设备
- [ ] 开机启动
- [ ] 最小化到托盘启动
- [ ] 更清晰的配对 / 认证失败恢复路径

### 兼容性与验证

- [ ] 扩大真实设备验证范围，并维护已验证设备列表（优先补 Apple TV、HomePod、第三方音箱 / AVR）
- [ ] 持续提升不同 AirPlay Receiver 实现的兼容性
- [ ] 评估更广泛的 AirPlay 2 接收端兼容性

## 开发

```bash
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

## License

Apache-2.0
