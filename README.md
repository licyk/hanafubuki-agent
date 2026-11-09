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
and Windows (x86_64) on GitHub ([releasing](docs/release.md)).

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
exit codes are in [docs/cli.md](docs/cli.md), the configuration in
[docs/config.md](docs/config.md).

## Embedding

A host depends on `hanafubuki-agent` (the facade) and
`hanafubuki-agent-contracts` (the plugin API its own plugins are written
against), and supplies its plugins and ports through the host interface
([docs/rules/plugins.md](docs/rules/plugins.md#the-host-interface)).

## Documentation

The documentation site's source is in [site/](site) (`bun run site:dev`;
see [AGENTS.md](AGENTS.md#site)); the pages below move there.

- [docs/architecture.md](docs/architecture.md): how the Agent is put
  together, and why.
- [docs/rules/plugins.md](docs/rules/plugins.md): the plugin rules and the
  host interface.
- [docs/spec.md](docs/spec.md): the Agent as built, in detail.
- [docs/cli.md](docs/cli.md): the `hanafubuki-agent` command line.
- [docs/config.md](docs/config.md): the standalone Agent's configuration:
  locations, layers, keys.
- [docs/release.md](docs/release.md): how a release is made and resumed.
- [AGENTS.md](AGENTS.md): the workspace rules and validation.

## License

GPL-3.0-only. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
