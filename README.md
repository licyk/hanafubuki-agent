# hanafubuki-agent

An AI coding and operations agent. It runs embedded as a Rust library in a
host application, and standalone as the `hanafubuki-agent` command line
and terminal interface (TUI).

## Status

The Agent's crates live here, moved from
[Hanafubuki](https://github.com/licyk/Hanafubuki), which embeds the Agent
through the `hanafubuki-agent` facade. The facade also runs without a host
(`AgentRuntime::standalone`), and the `hanafubuki-agent` binary runs the
terminal interface, ported from Codex's (with its slash commands, transcript
pager, `@` file mentions and session picker, and the model's thinking shown
as it streams), or one headless turn (`exec`),
and manages its configuration on it. Each release publishes the crates on
crates.io and binaries for Linux (x86_64), macOS (x86_64, Apple silicon)
and Windows (x86_64) on GitHub ([releasing](https://licyk.github.io/hanafubuki-agent/docs/releasing/)).

## Usage

```sh
cargo install hanafubuki-agent-cli    # or --path crates/cli in a checkout, or a release's binary
hanafubuki-agent providers add OpenAI --base-url https://api.openai.com/v1 --api-key-env OPENAI_API_KEY
hanafubuki-agent providers default provider-1 --model gpt-5.1
hanafubuki-agent                      # the terminal interface
hanafubuki-agent "Explain this repository"
hanafubuki-agent resume --last        # reopen the latest session
hanafubuki-agent exec "Summarize the open TODOs in this repository"
hanafubuki-agent exec --json --approve allow:exec_command - < task.md
hanafubuki-agent doctor
```

With no command it opens the terminal interface (`/` lists its commands;
`/quit`, or Ctrl-C twice, to leave), and `resume` reopens a session in it. `exec` prints the turn as text, or with `--json` one event per line, and
exits with a code that says how the turn ended; approvals follow
`--approve` (`deny` by default). The commands, flags, output formats and
exit codes are in the [command-line reference](https://licyk.github.io/hanafubuki-agent/docs/cli-reference/), the
configuration in [Configuration](https://licyk.github.io/hanafubuki-agent/docs/configuration/).

## Embedding

A host depends on `hanafubuki-agent` (the facade) and
`hanafubuki-agent-contracts` (the plugin API its own plugins are written
against), and supplies its plugins and ports through the host interface
([Embedding the Agent](https://licyk.github.io/hanafubuki-agent/docs/embedding/), [Host plugins](https://licyk.github.io/hanafubuki-agent/docs/host-plugins/)).

## Documentation

The documentation is at <https://licyk.github.io/hanafubuki-agent/docs/>, in Chinese and English. Its source is
[site/content/docs](site/content/docs) (`<slug>.mdx` in Chinese,
`<slug>.en.mdx` in English; `bun run site:dev`, see
[AGENTS.md](AGENTS.md#site)).

- Using the Agent: [Installation](https://licyk.github.io/hanafubuki-agent/docs/installation/),
  [Quick start](https://licyk.github.io/hanafubuki-agent/docs/quick-start/),
  [Terminal interface](https://licyk.github.io/hanafubuki-agent/docs/terminal-interface/), [exec](https://licyk.github.io/hanafubuki-agent/docs/exec/),
  [Providers and models](https://licyk.github.io/hanafubuki-agent/docs/providers-models/),
  [Permissions and approvals](https://licyk.github.io/hanafubuki-agent/docs/permissions/), [MCP](https://licyk.github.io/hanafubuki-agent/docs/mcp/),
  [Skills](https://licyk.github.io/hanafubuki-agent/docs/skills/), [Troubleshooting](https://licyk.github.io/hanafubuki-agent/docs/troubleshooting/).
- Configuration: [Configuration](https://licyk.github.io/hanafubuki-agent/docs/configuration/),
  [Configuration reference](https://licyk.github.io/hanafubuki-agent/docs/config-reference/).
- Reference: [Command-line reference](https://licyk.github.io/hanafubuki-agent/docs/cli-reference/),
  [exec --json events](https://licyk.github.io/hanafubuki-agent/docs/json-events/), [Exit codes](https://licyk.github.io/hanafubuki-agent/docs/exit-codes/).
- Embedding: [Embedding the Agent](https://licyk.github.io/hanafubuki-agent/docs/embedding/),
  [Host plugins](https://licyk.github.io/hanafubuki-agent/docs/host-plugins/).
- Development: [Architecture](https://licyk.github.io/hanafubuki-agent/docs/architecture/),
  [Internals](https://licyk.github.io/hanafubuki-agent/docs/internals/), [Plugin rules](https://licyk.github.io/hanafubuki-agent/docs/plugin-rules/),
  [Building and testing](https://licyk.github.io/hanafubuki-agent/docs/building-testing/),
  [Contributing](https://licyk.github.io/hanafubuki-agent/docs/contributing/), [Releasing](https://licyk.github.io/hanafubuki-agent/docs/releasing/).
- [AGENTS.md](AGENTS.md): the workspace rules and validation.

## License

GPL-3.0-only. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
