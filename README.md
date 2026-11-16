<div align="center">

# hanafubuki-agent

<p align="center">
  <a href="https://github.com/licyk/hanafubuki-agent/stargazers"><img src="https://img.shields.io/github/stars/licyk/hanafubuki-agent?style=flat&logo=github&logoColor=silver&color=bluegreen&labelColor=grey" alt="Stars"></a>
  <a href="https://github.com/licyk/hanafubuki-agent/issues"><img src="https://img.shields.io/github/issues/licyk/hanafubuki-agent?style=flat&logo=github&logoColor=silver&color=bluegreen&labelColor=grey" alt="Issues"></a>
  <a href="https://github.com/licyk/hanafubuki-agent/actions/workflows/ci.yml"><img src="https://github.com/licyk/hanafubuki-agent/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/licyk/hanafubuki-agent/releases"><img src="https://img.shields.io/github/v/release/licyk/hanafubuki-agent?color=bluegreen&labelColor=grey" alt="Release"></a>
  <a href="https://crates.io/crates/hanafubuki-agent"><img src="https://img.shields.io/crates/v/hanafubuki-agent?logo=rust&color=bluegreen&labelColor=grey" alt="crates.io"></a>
</p>

[Documentation · 简体中文 / English](https://licyk.github.io/hanafubuki-agent/docs/)

</div>

An AI coding and operations agent for your terminal, scripts and Rust applications.
Use it on its own, or embed the same runtime in a host such as
[Hanafubuki](https://github.com/licyk/Hanafubuki).

- **Work in the terminal** with streamed answers and reasoning, file mentions, input history,
  resumable sessions, and a unified settings center.
- **Read, edit and run** code, search the web, fetch pages, and work in Git worktrees.
  Permissions, automatic review and platform sandboxes control tool execution.
- **Delegate work** to agents with configurable roles, tool restrictions and model choices.
- **Keep context** across long tasks with durable history, memory, compaction and hierarchical
  context compression.
- **Connect your tools** through MCP, Skills, hooks and installable plugins. A failed plugin
  is isolated from the runtime and other plugins.
- **Automate or embed** with headless `exec`, JSON events, or a typed Rust facade and host plugin API.

## Install

Linux, macOS, or Windows with MSYS2/Git Bash:

```sh
curl -fsSL https://raw.githubusercontent.com/licyk/hanafubuki-agent/main/install.sh | bash
```

PowerShell (Windows PowerShell 5.1, or `pwsh` on Windows/Linux/macOS):

```powershell
irm https://raw.githubusercontent.com/licyk/hanafubuki-agent/main/install.ps1 | iex
```

The installers choose the native **x64 or ARM64** release, verify its SHA-256 and version,
then install for the current user. No Rust toolchain or administrator access is needed.
Open a new terminal and run `hanafubuki-agent --version`.
Linux releases require glibc 2.35 or newer; macOS supports Intel and Apple silicon.
On Windows, the shell installer finds PowerShell even outside PATH and installs the native Windows binary.
On Linux/macOS, the PowerShell installer requires `tar` and `chmod` and adds PATH to the current user’s PowerShell profile.

To choose a version or directory, download the script and run it with options:

```sh
bash install.sh --version 0.5.0 --install-dir "$HOME/.local/bin"
```

```powershell
pwsh -File ./install.ps1 -Version 0.5.0 -NoPathUpdate
```

You can also [download an archive](https://github.com/licyk/hanafubuki-agent/releases/latest)
or build with Rust 1.99 or newer:

```sh
cargo install --locked hanafubuki-agent-cli
```

See [Installation](https://licyk.github.io/hanafubuki-agent/docs/installation/) for script
options, default paths, checksums and removal. Update an existing installation with
`hanafubuki-agent update`.

## Start

```sh
hanafubuki-agent providers add OpenAI --base-url https://api.openai.com/v1 --api-key-env OPENAI_API_KEY
hanafubuki-agent providers default provider-1 --model gpt-5.1
hanafubuki-agent
```

Set `OPENAI_API_KEY` in your environment first. Use the provider ID returned by `providers add`
if it differs from `provider-1`. Providers support Responses, Chat Completions and Anthropic
interfaces; [configure the interface and model](https://licyk.github.io/hanafubuki-agent/docs/providers-models/)
for your service.

In the terminal, `/` lists commands, `/config` opens settings, `/model` selects a model,
and `/permission` chooses the permission mode. `/quit` or Ctrl-C twice exits.

## Command line

```sh
hanafubuki-agent "Explain this repository"
hanafubuki-agent resume --last
hanafubuki-agent exec "Summarize the open TODOs in this repository"
hanafubuki-agent exec --json --approve allow:exec_command - < task.md
hanafubuki-agent doctor
hanafubuki-agent update --check
```

`exec` prints a completed turn as text, or streams one JSON event per line with `--json`.
Headless approval requests are denied unless an `--approve` rule allows them; the exit code
records how the turn ended. See the [CLI reference](https://licyk.github.io/hanafubuki-agent/docs/cli-reference/),
[JSON events](https://licyk.github.io/hanafubuki-agent/docs/json-events/) and
[permission model](https://licyk.github.io/hanafubuki-agent/docs/permissions/).

## Embedding

```toml
[dependencies]
hanafubuki-agent = "0.7"
hanafubuki-agent-contracts = "0.7"
```

Use `AgentRuntime::standalone` for the standalone defaults, or supply a host's ports and
plugins through `AgentRuntime` and `AgentHostPorts`. The facade owns the Agent's processes,
sessions and shutdown; it does not depend on Hanafubuki.

[Embedding the Agent](https://licyk.github.io/hanafubuki-agent/docs/embedding/) explains the
lifecycle; [Host plugins](https://licyk.github.io/hanafubuki-agent/docs/host-plugins/) describes
how to add capabilities through the contracts registries.

## Development

```sh
git clone https://github.com/licyk/hanafubuki-agent.git
cd hanafubuki-agent
cargo run -p hanafubuki-agent-cli
cargo xtask check
cargo test --workspace --all-features
node tests/installers/run.mjs
```

The repository pins Rust with `rust-toolchain.toml`. The bilingual documentation site lives
in `site/` and uses Bun (`bun install --frozen-lockfile`, then `bun run site:dev`).
[AGENTS.md](AGENTS.md) holds the workspace rules;
[Building and testing](https://licyk.github.io/hanafubuki-agent/docs/building-testing/),
[Architecture](https://licyk.github.io/hanafubuki-agent/docs/architecture/) and
[Releasing](https://licyk.github.io/hanafubuki-agent/docs/releasing/) cover the development workflow.

## Licence

GPL-3.0-only. See [LICENSE](LICENSE) and [NOTICE](NOTICE), including attribution for code
derived from Codex and billion-context/acp-kernel. Release archives include third-party licences.
