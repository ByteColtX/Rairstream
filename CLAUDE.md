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
- `wasapi` — Windows WASAPI loopback 音频采集
- `mdns-sd` — AirPlay / RAOP 设备发现
- `tao` / `tray-icon` — 系统托盘
- `serde` — 配置模型序列化
- `thiserror` / `anyhow` — 错误表达与应用层错误传递

## 项目目的（WHY）

让 Windows 用户无需第三方软件，直接将系统音频输出到 AirPlay 设备（Apple TV、HomePod、AirPlay 扬声器等）。

**当前阶段目标：**
- 优先完成从“发现设备 → 选择设备 → 建立会话 → 采集音频 → 开始播放 → 停止/清理”的完整闭环
- 在现有免认证 RAOP 串流基础上，开始推进**需要认证/配对的 AirPlay 接收端支持**
- 认证相关实现优先关注握手、鉴权状态机、会话协商与错误反馈，不提前扩展非关键 UI/配置能力

## 开发约定（HOW）

### 构建与验证

```bash
cargo build                                      # 编译
cargo test --workspace                           # 运行所有测试
cargo clippy --workspace --all-targets -- -D warnings  # Lint（视警告为错误）
cargo fmt --check                               # 格式检查
```

**每次修改后必须确保 `cargo clippy --workspace --all-targets -- -D warnings` 和 `cargo fmt --check` 全部通过，不留警告。**

### 代码风格

- 格式化工具：`rustfmt`（遵循默认配置，不自定义）
- Lint：`clippy`，所有 warning 均需修复，不使用 `#[allow(...)]` 绕过
- 注释：**全部使用中文**，包括 `///` 文档注释和 `//` 行内注释
- 错误处理：使用 `thiserror` 定义领域错误类型，禁止在库代码中使用 `unwrap()`/`expect()`（测试和 `main` 除外）
- 安全边界：默认禁止 `unsafe` 代码；如未来确需引入，必须先明确收敛使用范围与安全性前提，再调整仓库策略

### 测试约定

- **单元测试**：在同文件底部用 `#[cfg(test)] mod tests { ... }` 包裹，测试纯逻辑（数据转换、状态机、协议编解码等）
- **集成测试**：放在 `tests/` 目录，测试跨模块行为与关键链路
- **协议测试**：优先覆盖发现结果解析、RTSP/RAOP 报文编解码、鉴权状态流转、会话协商失败路径
- **硬件/网络相关**（WASAPI、网络 IO）：通过 trait 或边界接口隔离真实依赖，默认不依赖真实设备；是否引入 mocking crate 按实现需要决定，不预设具体库
- **测试命名**：名称应直接表达场景与预期，优先可读性，其次再考虑统一前缀
- 覆盖重点：协议编解码、设备状态流转、音频缓冲区边界条件、认证/配对失败与恢复路径

### Commit 规范

遵循 [Conventional Commits v1.0.0](https://github.com/conventional-commits/conventionalcommits.org/blob/master/content/v1.0.0/index.md)。

常用类型：`feat` / `fix` / `refactor` / `test` / `docs` / `chore`

```
feat(discovery): 实现 mDNS AirPlay 设备扫描
fix(audio): 修复 WASAPI loopback 采集时的缓冲区溢出
test(transport): 补充 RAOP 握手报文编码单测
```

### 安全注意事项

- 默认禁止 `unsafe` 代码；若为必要的底层系统接入引入 `unsafe`，必须先在设计上说明边界、前提与替代方案
- 不得将设备 IP、认证 token、配对密钥、长期凭据等写死在代码中
- 认证 AirPlay 实现中，密钥交换材料、会话密钥与配对产物必须与业务逻辑分层，避免在日志或错误消息中泄漏敏感信息
