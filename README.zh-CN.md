<div align="center">

# hanafubuki-agent

<p align="center">
  <a href="https://github.com/licyk/hanafubuki-agent/stargazers">
    <img src="https://img.shields.io/github/stars/licyk/hanafubuki-agent?style=flat&logo=github&logoColor=silver&color=bluegreen&labelColor=grey" alt="Stars">
  </a>
  <a href="https://github.com/licyk/hanafubuki-agent/issues">
    <img src="https://img.shields.io/github/issues/licyk/hanafubuki-agent?style=flat&logo=github&logoColor=silver&color=bluegreen&labelColor=grey" alt="Issues">
  </a>
  <a href="https://github.com/licyk/hanafubuki-agent/commits/main">
    <img src="https://flat.badgen.net/github/last-commit/licyk/hanafubuki-agent/main?icon=github&color=green&label=last%20main%20commit" alt="Last main commit">
  </a>
  <a href="https://github.com/licyk/hanafubuki-agent/actions/workflows/ci.yml">
    <img src="https://github.com/licyk/hanafubuki-agent/actions/workflows/ci.yml/badge.svg" alt="CI">
  </a>
  <a href="https://github.com/licyk/hanafubuki-agent/actions/workflows/release.yml">
    <img src="https://github.com/licyk/hanafubuki-agent/actions/workflows/release.yml/badge.svg" alt="Release workflow">
  </a>
  <a href="https://github.com/licyk/hanafubuki-agent/releases">
    <img src="https://img.shields.io/github/v/release/licyk/hanafubuki-agent?color=bluegreen&labelColor=grey" alt="Latest release">
  </a>
  <a href="https://crates.io/crates/hanafubuki-agent">
    <img src="https://img.shields.io/crates/v/hanafubuki-agent?logo=rust&color=bluegreen&labelColor=grey" alt="crates.io">
  </a>
</p>

[English](https://github.com/licyk/hanafubuki-agent/blob/main/README.md) | [简体中文](https://github.com/licyk/hanafubuki-agent/blob/main/README.zh-CN.md)

**[官网](https://licyk.github.io/hanafubuki-agent/)** · **[文档](https://licyk.github.io/hanafubuki-agent/docs/)**

</div>

面向终端、脚本和 Rust 应用的 AI 编程与运维 Agent。
既可以独立使用，也可以将同一套运行时嵌入
[Hanafubuki](https://github.com/licyk/Hanafubuki) 等宿主应用。

- **终端交互**：流式回答与思考过程、文件引用、输入历史、会话恢复，以及统一的设置中心。
- **读取、编辑与执行**：操作代码、搜索网页、获取页面内容，并在 Git 工作树中开展任务。
  通过权限、自动审核和平台沙箱控制工具执行。
- **委派任务**：为子 Agent 配置角色、工具范围和模型。
- **保留上下文**：借助持久化历史、记忆、上下文精简与分层压缩，持续处理长任务。
- **连接工具**：支持 MCP、Skills、钩子和可安装插件；插件故障与运行时及其他插件隔离。
- **自动化与嵌入**：提供无界面的 `exec`、JSON 事件流，以及具有明确类型的 Rust 接口和宿主插件 API。

## 安装

Linux、macOS，或使用 MSYS2/Git Bash 的 Windows：

```sh
curl -fsSL https://raw.githubusercontent.com/licyk/hanafubuki-agent/main/install.sh | bash
```

PowerShell（Windows PowerShell 5.1，或 Windows/Linux/macOS 上的 `pwsh`）：

```powershell
irm https://raw.githubusercontent.com/licyk/hanafubuki-agent/main/install.ps1 | iex
```

安装脚本会选择当前平台的 **x64 或 ARM64** 发行包，验证 SHA-256 和版本号后，为当前用户安装。
无需 Rust 工具链或管理员权限。安装完成后，打开新终端并运行 `hanafubuki-agent --version`。
Linux 发行包要求 glibc 2.35 或更高版本；macOS 支持 Intel 和 Apple 芯片。
在 Windows 上，Shell 安装脚本即使在 PATH 中找不到 PowerShell，也会尝试定位它，并安装原生 Windows 程序。
在 Linux/macOS 上，PowerShell 安装脚本需要 `tar` 和 `chmod`，并通过当前用户的 PowerShell Profile 配置 PATH。

要指定版本或安装目录，可先下载脚本，再带参数运行：

```sh
bash install.sh --version 0.5.0 --install-dir "$HOME/.local/bin"
```

```powershell
pwsh -File ./install.ps1 -Version 0.5.0 -NoPathUpdate
```

也可以[下载发行包](https://github.com/licyk/hanafubuki-agent/releases/latest)，
或使用 Rust 1.99 及以上版本从源码安装：

```sh
cargo install --locked hanafubuki-agent-cli
```

脚本参数、默认路径、校验和与卸载方式见[安装指南](https://licyk.github.io/hanafubuki-agent/docs/installation/)。
已有安装可通过 `hanafubuki-agent update` 更新。

## 开始使用

```sh
hanafubuki-agent providers add OpenAI --base-url https://api.openai.com/v1 --api-key-env OPENAI_API_KEY
hanafubuki-agent providers default provider-1 --model gpt-5.1
hanafubuki-agent
```

请先在环境变量中设置 `OPENAI_API_KEY`。如果 `providers add` 返回的供应商 ID 不是 `provider-1`，
请使用实际返回的 ID。供应商支持 Responses、Chat Completions 和 Anthropic 接口，
请按所用服务[配置接口与模型](https://licyk.github.io/hanafubuki-agent/docs/providers-models/)。

在终端界面中，输入 `/` 查看命令，`/config` 打开设置，`/model` 选择模型，
`/permission` 切换权限模式。使用 `/quit` 或连续两次按 Ctrl-C 退出。

## 命令行

```sh
hanafubuki-agent "解释这个仓库"
hanafubuki-agent resume --last
hanafubuki-agent exec "汇总这个仓库中尚未完成的 TODO"
hanafubuki-agent exec --json --approve allow:exec_command - < task.md
hanafubuki-agent doctor
hanafubuki-agent update --check
```

`exec` 默认在一轮执行完成后输出文本；加上 `--json` 时，以每行一个 JSON 事件的形式流式输出。
无界面运行时，除非 `--approve` 规则明确允许，否则审批请求会被拒绝；退出码反映该轮执行的结束状态。
详见[命令行参考](https://licyk.github.io/hanafubuki-agent/docs/cli-reference/)、
[JSON 事件](https://licyk.github.io/hanafubuki-agent/docs/json-events/)和
[权限模型](https://licyk.github.io/hanafubuki-agent/docs/permissions/)。

## 嵌入 Rust 应用

在宿主项目中添加 `hanafubuki-agent` 和 `hanafubuki-agent-contracts` 依赖，
版本要求与完整示例见[嵌入 Agent](https://licyk.github.io/hanafubuki-agent/docs/embedding/)。

使用 `AgentRuntime::standalone` 获取独立模式的默认实现，或通过 `AgentRuntime` 和
`AgentHostPorts` 提供宿主接口与插件。Agent 接口层负责管理自己的进程、会话和关闭流程，
不依赖 Hanafubuki。

[嵌入指南](https://licyk.github.io/hanafubuki-agent/docs/embedding/)说明生命周期，
[宿主插件](https://licyk.github.io/hanafubuki-agent/docs/host-plugins/)说明如何通过契约注册表添加能力。

## 开发

```sh
git clone https://github.com/licyk/hanafubuki-agent.git
cd hanafubuki-agent
cargo run -p hanafubuki-agent-cli
cargo xtask check
cargo test --workspace --all-features
node tests/installers/run.mjs
```

仓库通过 `rust-toolchain.toml` 固定 Rust 版本。双语文档站位于 `site/`，使用 Bun：
先运行 `bun install --frozen-lockfile`，再运行 `bun run site:dev`。

[AGENTS.md](AGENTS.md) 定义仓库规则；
[构建与测试](https://licyk.github.io/hanafubuki-agent/docs/building-testing/)、
[架构](https://licyk.github.io/hanafubuki-agent/docs/architecture/)和
[发布](https://licyk.github.io/hanafubuki-agent/docs/releasing/)说明开发流程。

## 许可证

本项目采用 **GPL-3.0-only** 许可证。完整条款及署名信息见 [LICENSE](LICENSE) 和 [NOTICE](NOTICE)，
其中包含源自 Codex 与 billion-context/acp-kernel 的代码署名。发行包附带第三方许可证。
