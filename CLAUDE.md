# Rairstream

## 项目目标
- 本项目是对 TuneBlade 的 Rust 重写，项目名为 **Rairstream**。
- 首版本聚焦 **Windows-first**。
- 首版本只做一件核心事情：将系统音频流式传输到 **AirPlay enabled speakers**。
- 其他能力要预留扩展点，但首版本不实现。

## 首版本范围
### 要实现
- Windows 桌面端应用骨架
- AirPlay / RAOP 设备发现
- 单设备选择与连接 / 断开
- Windows 系统音频采集（WASAPI loopback）
- 基础音频管线
- RAOP / AirPlay 传输骨架

### 明确不在首版本实现
- AirPlay 2
- 多房间 / 多设备同步
- 手动管理接收端
- 延迟调节 UI
- 高级预设
- 非 speaker 目标
- macOS 系统音频采集

## 工程约束
- 仓库结构采用工程化 Rust workspace 组织。
- 代码风格遵循 Rust 官方风格。
- 格式化工具统一使用 `rustfmt`。
- Lint 使用 `clippy`，新代码应保持 warning-free。
- 注释使用中文。
- 对外公开 API 文档保持 idiomatic rustdoc 结构；如果未来需要对外发布，再考虑双语或英文文档。

## 当前推荐目录结构
```text
apps/
  rairstream-desktop/
crates/
  rairstream-core/
  rairstream-device-discovery/
  rairstream-airplay/
  rairstream-audio-capture/
  rairstream-session/
  rairstream-config/
  rairstream-platform/
tests/
  integration/
  fixtures/
```

## 分层原则
- `apps/rairstream-desktop` 保持薄，主要负责入口、生命周期、依赖装配。
- `rairstream-core` 放领域模型、共享错误、traits、状态契约。
- 协议、领域逻辑尽量保持纯 Rust，不依赖 UI 或操作系统。
- 平台相关代码通过 traits 和 `cfg` 隔离。
- 只有边界稳定、适合独立测试的部分才拆 crate；紧耦合的应用层代码可保留为模块。

## 预留扩展点
- `CaptureBackend`：为未来 macOS 等平台音频采集预留。
- `SessionBackend`：为未来 AirPlay 2 或 FFI 实现预留。
- Receiver capability / profile：为设备差异兼容预留。
- `LatencyPolicy`：为未来延迟控制预留。
- 配置与 preset 存储面向后续扩展设计，但首版本只保留最小实现。

## 质量门禁
在提交前应尽量通过：

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## 测试策略
- 单元测试尽量内联在各 crate 中。
- 协议与状态机的集成测试放在 `tests/integration/`。
- 协议 / 设备样例放在 `tests/fixtures/`。
- Windows 特定行为测试使用 `target_os = "windows"` 做条件保护。
- 首版本优先覆盖协议、会话、管线，不优先做重 UI 的端到端测试。

## 开发偏好
- Git commit message 严格遵循 Conventional Commits 1.0.0。
- `type` / `scope` / `BREAKING CHANGE` 使用英文。
- `subject` / `body` / `footer` 使用中文。
- 优先输出单行 commit message，除非确实需要额外上下文。

## 迁移说明
- 本文件应随仓库一起带到新的开发机器。
- 计划文件是阶段性产物，不必强依赖迁移；这里已经沉淀了长期有效的约束与方向。
- 若未来架构方向发生变化，优先更新本文件，而不是依赖外部计划文件。
