# Rairstream

> Stream Windows system audio to AirPlay devices.

Rairstream 是一个用 Rust 编写的 Windows 桌面应用，目标是在不依赖第三方转发软件的前提下，把 **Windows 系统音频** 实时流式传输到 **AirPlay / RAOP** 设备。

当前项目仍处于 **MVP 闭环阶段**，重点是先打通从设备发现到开始/停止播放的最小可用链路。

## 项目状态

Rairstream 当前更适合开发者体验、链路验证和协议实现推进，现阶段重点如下：

- 已打通 **mDNS 发现 → 设备选择 → RAOP 握手 → Windows 音频采集 → 发流 → 停止清理** 的主链路
- 当前优先支持 **免认证的 AirPlay 1 / RAOP 目标设备**
- 对需要认证或配对的接收端，暂未完成完整支持
- 目前仅支持 **Windows** 运行时

如果你想关注项目最近在做什么，可以优先看：

- 最小串流链路是否持续可用
- 认证 / 配对型 AirPlay 接收端支持推进情况
- 配置持久化与错误反馈的完善

## 当前特性

- 基于 mDNS 浏览 `_raop._tcp.local.` 与 `_airplay._tcp.local.` 设备
- 系统托盘交互：刷新设备、选择设备、停止串流、退出
- 基于 WASAPI loopback 的 Windows 系统音频采集
- 最小 RAOP / RTSP 握手流程：`OPTIONS` / `ANNOUNCE` / `SETUP` / `RECORD` / `TEARDOWN`
- 将采集到的 PCM 音频发送到目标设备
- 提供 `smoke` 模式，用于验证最小链路是否跑通
- 已有基础测试覆盖发现解析、会话状态与握手关键路径

## 当前限制

- **仅支持 Windows**
- 当前并非完整 AirPlay 2 实现
- 暂未完成需要认证 / 配对的 AirPlay 接收端支持
- 配置模型已存在，但**持久化尚未接入**
- 项目仍在快速迭代，接口与行为可能继续调整

## 技术栈

- Rust 2024
- `wasapi` — Windows 系统音频回环采集
- `mdns-sd` — AirPlay / RAOP 设备发现
- `tao` + `tray-icon` — 系统托盘 UI
- `tracing` + `tracing-subscriber` — 日志
- `serde` — 配置模型序列化
- `thiserror` / `anyhow` — 错误处理

## 快速开始

### 环境要求

- Windows 11 或其他支持 WASAPI loopback 的 Windows 环境
- Rust 1.85+
- 与目标 AirPlay / RAOP 设备处于同一可发现、可连接的网络环境

### 编译

```bash
cargo build
```

### 启动托盘模式

```bash
cargo run
```

启动后应用会进入系统托盘，你可以：

- 点击“刷新设备”重新发现目标设备
- 点击设备项开始串流
- 在串流中点击“停止串流”，或再次点击当前设备停止会话
- 点击“退出”关闭应用

### 运行最小链路烟测

```bash
cargo run -- smoke
```

只连接特定设备时，可以追加一个过滤参数：

```bash
cargo run -- smoke HomePod
```

`smoke` 模式会执行以下流程：

1. 发现设备
2. 选择首个设备或匹配过滤条件的设备
3. 建立 RAOP 会话
4. 启动系统音频采集并持续发流
5. 在终端按回车后停止并清理会话

## CLI

```bash
rairstream [OPTIONS] [smoke [DEVICE_FILTER]]
```

### 位置参数

- 无参数：启动托盘模式
- `smoke`：启动最小链路烟测
- `smoke <DEVICE_FILTER>`：仅连接名称、ID 或主机地址包含该字符串的设备

### 日志参数

- `-v` / `--verbose`：启用 `debug`
- `-vv`：启用 `trace`
- `--log-level <error|warn|info|debug|trace>`：显式指定日志级别

### 环境变量

- `RAIRSTREAM_LOG`
- `RUST_LOG`

当未显式传入 `--log-level` 时，程序会按以下优先级解析日志等级：

1. `-v` / `-vv`
2. `RAIRSTREAM_LOG`
3. `RUST_LOG`
4. 默认 `info`

## 工作原理

Rairstream 当前围绕一个尽量小但完整的链路组织：

1. 通过 mDNS 发现 AirPlay / RAOP 设备
2. 在托盘中选择目标设备
3. 建立最小 RAOP / RTSP 会话
4. 通过 WASAPI loopback 捕获 Windows 系统音频
5. 将 PCM 数据持续发送到目标设备
6. 停止串流时关闭采集线程并发送 `TEARDOWN`

## 项目结构

```text
src/
├─ main.rs              # 应用入口、CLI、日志初始化、smoke 模式
├─ lib.rs               # crate 模块导出
├─ app/                 # 应用状态、会话编排、平台检查
├─ audio/               # Windows 音频采集（WASAPI loopback）
├─ discovery/           # AirPlay / RAOP 设备发现与解析
├─ transport/           # RAOP / RTSP 会话、编解码、发包
├─ ui/                  # 系统托盘 UI 与交互控制
└─ config/              # 配置模型与持久化入口预留
```

## 开发

常用验证命令：

```bash
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

项目内约定 `clippy` 与 `fmt` 检查保持通过，不留 warning。

## 路线图

当前优先推进的方向：

- 需要认证 / 配对的 AirPlay 接收端支持
- 鉴权状态机与会话协商完善
- 配置持久化
- 更清晰的错误反馈与恢复路径

## License

Apache-2.0
