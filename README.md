# hanafubuki-agent

An AI coding and operations agent. It runs embedded as a Rust library in a
host application, and is growing a standalone mode: a command-line and
terminal (TUI) application of its own.

## Status

The Agent's crates live here, moved from
[Hanafubuki](https://github.com/licyk/Hanafubuki), which embeds the Agent
through the `hanafubuki-agent` facade. The standalone command-line and TUI
front ends are not built yet ([extraction plan](docs/extraction-plan.md)).

## Embedding

A host depends on `hanafubuki-agent` (the facade) and
`hanafubuki-agent-contracts` (the plugin API its own plugins are written
against), and supplies its plugins and ports through the host interface
([docs/rules/plugins.md](docs/rules/plugins.md#the-host-interface)).

## Documentation

- [docs/architecture.md](docs/architecture.md): how the Agent is put
  together, and why.
- [docs/rules/plugins.md](docs/rules/plugins.md): the plugin rules and the
  host interface.
- [docs/spec.md](docs/spec.md): the Agent as built, in detail.
- [AGENTS.md](AGENTS.md): the workspace rules and validation.

## License

GPL-3.0-only. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
