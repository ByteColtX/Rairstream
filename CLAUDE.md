# CLAUDE.md

## 项目概述（WHAT）

将 Windows 系统音频实时流式传输到 AirPlay 设备的桌面端应用。

**当前目录骨架：**
- `src/main.rs` — 应用入口与启动装配
- `src/app/` — 应用状态、会话编排、平台检查
- `src/audio/` — Windows 音频采集（WASAPI loopback）
- `src/discovery/` — AirPlay 设备发现（mDNS/Bonjour）
- `src/transport/` — AirPlay/RAOP 传输协议骨架
- `src/ui/` — 系统托盘 UI（设备选择、连接控制）
- `src/config/` — 用户配置模型与持久化入口预留

**当前仓库为单 crate Rust 应用，不再使用 `apps/` / `crates/` workspace 布局。**

**主要依赖：**
- `windows` crate — WASAPI 音频 API
- `mdns-sd` 或 `zeroconf` — 设备发现
- `tao` / `tray-icon` — 系统托盘
- `tokio` — 异步运行时

## 项目目的（WHY）

让 Windows 用户无需第三方软件，直接将系统音频输出到 AirPlay 设备（Apple TV、HomePod、AirPlay 扬声器等）。

## 开发约定（HOW）

### 构建与验证

```bash
cargo build                  # 编译
cargo test                   # 运行所有测试
cargo clippy -- -D warnings  # Lint（视警告为错误）
cargo fmt --check            # 格式检查
```

**每次修改后必须确保 `cargo clippy` 和 `cargo fmt --check` 全部通过，不留警告。**

### 代码风格

- 格式化工具：`rustfmt`（遵循默认配置，不自定义）
- Lint：`clippy`，所有 warning 均需修复，不使用 `#[allow(...)]` 绕过
- 注释：**全部使用中文**，包括 `///` 文档注释和 `//` 行内注释
- 错误处理：使用 `thiserror` 定义领域错误类型，禁止在库代码中使用 `unwrap()`/`expect()`（测试和 `main` 除外）

### 测试约定

- **单元测试**：在同文件底部用 `#[cfg(test)] mod tests { ... }` 包裹，测试纯逻辑（数据转换、状态机、协议编解码等）
- **集成测试**：放在 `tests/` 目录，测试跨模块行为
- **异步测试**：使用 `#[tokio::test]`
- **硬件相关**（WASAPI、网络 IO）：用 trait 抽象后 mock，不依赖真实设备；可用 `mockall` crate
- **测试命名**：`test_<被测函数>_<场景描述>`，例如 `test_parse_mdns_record_missing_port`
- 覆盖重点：协议编解码、设备状态流转、音频缓冲区边界条件

### Commit 规范

遵循 [Conventional Commits v1.0.0](https://github.com/conventional-commits/conventionalcommits.org/blob/master/content/v1.0.0/index.md)。

常用类型：`feat` / `fix` / `refactor` / `test` / `docs` / `chore`

```
feat(discovery): 实现 mDNS AirPlay 设备扫描
fix(audio): 修复 WASAPI loopback 采集时的缓冲区溢出
test(transport): 补充 RAOP 握手报文编码单测
```

### 安全注意事项

- 涉及 Windows API 的代码用 `unsafe` 块最小化包裹，并在上方注释说明安全性前提
- 不得将设备 IP、认证 token 等写死在代码中
