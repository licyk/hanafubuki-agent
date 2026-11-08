# Configuration

How to configure the standalone Agent (the `hanafubuki-agent` binary):
where its files are, how their layers combine, and the keys they hold. An
application that embeds the Agent (Hanafubuki) names its own configuration
directory and reads the user's files alone, without the layers below; its
own settings page writes them.

The implementation is `hanafubuki-agent-config` (`AgentConfig`,
`ConfigLayers`, `AgentDirs`); the design is in
[architecture.md §10](architecture.md#10-persistence-and-compatibility).

## Locations

| | Linux | macOS | Windows |
| --- | --- | --- | --- |
| Configuration | `~/.config/hanafubuki-agent/` | `~/Library/Application Support/hanafubuki-agent/` | `%APPDATA%\hanafubuki-agent\` |
| Data | `~/.local/share/hanafubuki-agent/` | the same directory | the same directory |

The Linux directories follow `$XDG_CONFIG_HOME` and `$XDG_DATA_HOME`.

`HANAFUBUKI_AGENT_HOME`, when set, replaces both with one directory, like
Codex's `CODEX_HOME`: the configuration files and the data sit side by side
in it, as they already do on macOS and Windows. A relative path is taken
from the working directory.

The configuration directory holds three files:

- `config.toml`: the settings, the profiles and the trusted projects. The
  only file with layers.
- `providers.toml`: the model Providers, their tokens and the model policy.
- `mcp.toml`: the MCP servers and their credentials.

The last two hold credentials and are owner-only (`0600` on Unix). A file the
Agent had to repair, or could not read at all, is copied into `.recovery/`
first; nothing is ever deleted.

The data directory holds the history (`history/`), memory (`memory/`), the
Skills (`skills/`), the logs (`logs/hanafubuki-agent.log`), the crash-loop
marker, the process-group ledger and `.lock`. Only one Agent process uses a
data directory at a time: a second one fails to start with an error naming
the directory.

## Layers

The settings in `config.toml` are read in layers, lowest first. Each layer
sets only the keys it names; a key it leaves out, or sets to a value the
Agent cannot use, keeps the value of the layer below.

1. **Defaults.**
2. **The user's `config.toml`.**
3. **The profile** `-p <name>` selects: the table `[profiles.<name>]` of the
   user's `config.toml`, holding any of the keys below. Selecting a profile
   the file does not have is an error.
4. **The project's file**, `<project>/.hanafubuki-agent/config.toml`, where
   the project is the Git root of the working directory (the working
   directory itself outside a repository). It is read only when the user's
   `config.toml` trusts the project:

   ```toml
   [projects."/home/me/src/my-project"]
   trustLevel = "trusted"
   ```

   The path is compared as a location, not as text (a symbolic link or a
   different spelling of the same directory matches). A project's file is
   never repaired or rewritten by the Agent; a value in it that cannot be
   used is logged and skipped.
5. **`-c key=value` overrides**, in the order given. The key is a dotted path
   of the keys below (`memory.enabled`, `network.proxy`); the value is TOML
   (`true`, `7`, `"full_access"`, `["a", "b"]`), and a value that is not
   valid TOML is taken as a string, so `-c logLevel=debug` needs no quotes.

The Agent writes only the user's `config.toml`, and only the keys its
settings change (the permission mode, memory, plugins, web tools); a write
never copies a profile's, a project's or an override's value into it, and
keeps comments and keys it does not know. `providers.toml` and `mcp.toml`
have no layers.

## Keys of `config.toml`

Keys are camelCase, in the file and in `-c` paths.

| Key | Values | Default |
| --- | --- | --- |
| `permissionMode` | `"manual"`, `"auto_review"`, `"full_access"` | `"manual"` |
| `memory.enabled` | `true`, `false` | `false` |
| `memory.useMemories`, `memory.generateMemories`, `memory.excludeExternalContext` | booleans | `true`, `true`, `false` |
| `memory.extractionModel`, `memory.consolidationModel` | a model name | the session's |
| `memory.maxSourceAgeDays` (1–3650), `memory.minimumIdleHours` (0–8760), `memory.maxSessionsPerPass` (1–64), `memory.maxStage1Outputs` (1–4096), `memory.maxUnusedDays` (1–3650) | integers | 10, 6, 2, 256, 30 |
| `disabledPlugins` | plugin names (`["titles", "web"]`) | `[]` |
| `webSearchProvider` | `"auto"`, `"duckduckgo"`, `"searxng"` (needs `searxngBaseUrl`) | `"auto"` |
| `searxngBaseUrl` | an `http(s)` endpoint URL | none |
| `webFetchUseJinaReader` | `true`, `false` | `false` |
| `network.proxy` | `"system"`, `"disabled"`, or a proxy URL (`http`, `https`, `socks4`, `socks4a`, `socks5`, `socks5h`, credentials allowed) | `"system"` |
| `logLevel` | `"off"`, `"error"`, `"warn"`, `"info"`, `"debug"`, `"trace"` | `"info"` |
| `profiles.<name>` | a table of any keys above | |
| `projects."<path>".trustLevel` | `"trusted"` | |

`network.proxy` is the proxy of the web tools, MCP servers and OAuth.
`logLevel` sets the log file's level; only `trace` records model and tool
content, bounded, and no level records credentials. Records of other
libraries are kept at `info` and above whatever the level. Both keys are
the standalone Agent's: an application that embeds the Agent supplies its
own proxy and logger and ignores them.

## Example

```toml
# ~/.config/hanafubuki-agent/config.toml
permissionMode = "manual"
logLevel = "info"

[memory]
enabled = true

[network]
proxy = "http://127.0.0.1:7890"

# hanafubuki-agent -p review
[profiles.review]
permissionMode = "auto_review"
disabledPlugins = ["titles"]

[projects."/home/me/src/my-project"]
trustLevel = "trusted"
```

```toml
# /home/me/src/my-project/.hanafubuki-agent/config.toml
webSearchProvider = "duckduckgo"

[memory]
enabled = false
```

Started in `/home/me/src/my-project/crates/core` as
`hanafubuki-agent -p review -c logLevel=debug`, the Agent runs with the
permission mode `auto_review` (the profile), memory off (the project), web
search on DuckDuckGo (the project), the proxy `http://127.0.0.1:7890` (the
user's file) and the log level `debug` (`-c`).

## Secrets

Provider tokens live in `providers.toml`. MCP OAuth tokens live in the
operating system's keyring (Keychain, Credential Manager, the Secret
Service), under the service `org.licyk.hanafubuki-agent.secrets`; Hanafubuki
keeps its own under another service, so the two never share an entry.
