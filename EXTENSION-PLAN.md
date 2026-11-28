# 扩展系统开放计划（kardia 0.7）

起点 `916a9141`，kardia 0.6.0。目标：让安装的插件（guest）能从"只读两个 JSON 视图"变成"能在受控、可授权的前提下真正做事"，并把这类能力从内置插件（`agent/src/host/manifest.rs` 里写死的 `PluginDef`）下沉到清单声明，使第三方插件与内置插件的差距只剩可用能力，而不是接入方式。

设计约束沿用 `AGENTS.md`：最小连贯改动、不预设扩展点、不加重失败路径的防御性检查、外部数据只在入口校验一次、模块 ≤600 行（测试 900）、函数 ≤200 行、`apply_patch` 改文件、每步跑对应范围的校验。

## 一、现状与缺口

| 面 | 现状 | 缺口 |
| --- | --- | --- |
| guest 读 | `#host` 行提供 `workspace`（`{access, roots}`）与 `settings` 两个 JSON 视图 | 只能看到路径字符串，读内容、查设置、列工具有没有通道 |
| guest 写 | 什么都没有 | 插件无法产生任何副作用 |
| 内置插件 | 在 `agent/src/host/manifest.rs` 逐个手写 | 设置项必须写死在 Rust 里（如 `plugin-web/src/options/mod.rs` 的 60 个 `SettingSpec`），第三方插件只能靠 `configSchema` 的受限 lowering |
| 前置条件 | `plugin_tool_call`（`ApprovalActionKind::PluginToolCall`）审批随工具调用流动 | 工具之外的调用没有审批通道 |

kardia 0.7 已提供三件本机确实要用的东西：`HostAction`/`register_action`（宿主把方法注册成 guest 可 `invoke` 的服务）、`Context::migrate_binding`/`refresh_consumers`（不重启就能换绑定）、`Context::plugin_def`（在已运行的插件里挂子 fiber）。它们此前都没有被用到。

## 二、任务

### 任务 1：宿主动作通道（HostAction）

`crates/plugin-packages/src/host_services.rs` 在既有的 `#host` 行里注册一个 `agent` 动作，guest 用现有的 `invoke` 调用；方法的可见性由包现有的 `granted`/`permissions` 决定，与 `workspace` 视图同源。

| 方法 | 参数 | 回答 | 依据 |
| --- | --- | --- | --- |
| `workspace.list` | `{ path, depth? }` | `DirectoryListing` | `WorkspaceFsService::prepare/execute`，受根目录与受保护子树约束 |
| `workspace.read` | `{ path, offset?, limit? }` | `FileReadResult` | 同上 |
| `settings.get` | 无 | 自己的设置（与 `settings` 视图同一对象） | 已有 |

拒绝发生在两处：`access != Read` 时 `workspace.*` 直接报"not granted workspace read"；WASM guest 因为 `access` 恒为 `None` 自然拒绝，不需要额外分支。未知方法报 kardia 默认的 "no action"，不另造错误。

验收：`crates/plugin-packages/tests/guests/host_calls.rs` 覆盖允许、拒绝（无授权）、未知方法三态，以及读取受保护子树被拒。

### 任务 2：清单声明的设置选项（`configSchema` 放开）

现在 lowering 只能表达 boolean/integer/number/string/enum/字符串数组（`crates/packages/src/options/lower.rs`），且"没有任何选项是秘密"。补两项：

- `x-secret: true` → `SettingKind::Text { secret: true }`（插件才能放 token）；
- `["string", "null"]` 类型或含 `null` 的 `enum` → 可空的文本/选项，使 `default: null` 能成立。

`enum` 原本已 lower 成 `Choice`，无需改动；数值 `enum` 未做，因为 `Choice` 在 foundation 层只按字符串读写，要动读写与三处 UI，超出本任务的收益。

验收：`crates/packages/src/options` 的单元测试与 `plugins validate` 的 issues 列表。

### 任务 3：内置插件的选项改用声明式清单

评估结论：**不做整表迁移**。`plugin-web` 的选项作为内置行（`[pluginOptions.web]`）时，缺失的键取 `SettingSpec::default`；作为插件设置（`[plugins."id".settings]`）时，缺失的键保持缺失、由 guest 自己的默认值生效。把默认值搬进 `configSchema` 会改变设置中心显示的层（"default" 变成 "user"）、使 `packages` 的 `to_object()` 塞入 10+ 个可计算默认值、并把 `region` 的 `nullable` 语义挤进 lowering。改动的风险与收益不成比例。

替代做法：任务 2 补齐的 lowering 词汇（secret、可空文本、可空 enum）已经覆盖了第三方插件最缺的部分；`plugin-web` 继续用 Rust 声明，因为它的默认值来自 `SearchOptions` 的计算值，不是常量。

### 任务 4：会话与记忆工具的动作面

评估结论：**有副作用的方法不在同步 `HostAction` 上做**。`HostAction::call` 是同步的，而会话、记忆、终端都在 Agent 的异步服务里；`run_blocking_operation` 从宿主运行时自己的 blocking 线程上等待会同池自等。把 `session.list`/`memory.write`/`terminal.spawn` 塞进 `invoke` 只能是阻塞桥或第二套授权模型，两条都坏。

替代做法：`agent` 动作保持在**只读**面（任务 1 的 `workspace.list`/`workspace.read` + 本任务的 `workspace.search`），全部走 workspace 策略；需要副作用的插件用**插件工具**，那里已有 `PluginToolCall` 审批、调用上下文与取消，`sessionId`/`turnId`/`callId` 随参数到达。这条分界写进文档。

验收：`host_calls` 覆盖 `workspace.search` 的允许/拒绝，以及未授权时的拒绝理由。

### 任务 5：声明式渲染面

第三方工具目前只有 fallback 展示（结构化结果列键值）。新增：

- `contributes.toolPresenters[]`：`{ tool, fields: [{ key, label, kind }] }`，kind 沿用选项词汇（text/integer/boolean/choice/list）；
- `contributes.statusItems[]`：`{ id, label, action? }`，`/plugins` 里展示并从 `invoke` 取值。

两者都注册为普通的 `ToolPresenter`/状态视图，不新增框架。渲染端在 TUI 的插件页与设置中心复用。

验收：TUI 快照（`crates/tui` 的截图测试）与 `plugin-rules` 文档同步。

### 任务 6：不重启的配置迁移

评估结论：**本机做不到，记录理由**。实测（`new_settings_reach_the_guest_without_restarting_it`，随后删除）显示改设置后进程号变了：设置变化确实走 kardia 的 `patch_context`，但 `patch_context` 对 `config` 这个键调用 `inner.update_with(config, true)`，而 guest 桥接（kardia-hmr `bridge()`）没有注册 `on_update`，配置变化只能通过重启插件生效。`migrate_binding`/`refresh_consumers` 只迁移 Realm 里的绑定，不改变插件自身对配置的处理。

可行的替代是一份真正的协议扩展：guest ABI 增加一个 `reconfigure` 导出，桥接用 `on_update` 接管配置，届时不重启；这要改 kardia-hmr 与两个 SDK，属于上游工作。计划在这里停：**不改**，并在文档里写明"改设置会重启 guest"。

### 任务 7：文档

任务 1–6 完成后，把计划里仍然有效的部分重写进正式文档，然后删除本文件：

- `site/content/docs/writing-plugins.{mdx,en.mdx}`：清单新字段、设置选项新词法、guest 可调用的动作与方法表；
- `site/content/docs/plugin-rules.{mdx,en.mdx}`：`#guest-host-calls` 与 `#guest-contributions` 的边界说明；
- `site/content/docs/architecture.{mdx,en.mdx}`："Code plugins" 一句指向动作通道。

## 三、执行方式

每个任务一次提交（英文祈使句消息），提交前跑该任务相关范围：

```
cargo fmt --all --check
RUSTFLAGS="-D warnings" cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo xtask check
cargo test --workspace --all-features        # ulimit -n 65535
```

全部任务完成后跑一遍完整集合，通过后再做任务 7。新增公共面需要在 `architecture/*-policy.toml` 里登记时一并提交并说明原因。

已知无关失败：`tui::render::highlight::the_bundled_grammars_and_themes_licences_are_current`（`crates/tui/syntax-licenses.txt` 需 `UPDATE_SYNTAX_LICENSES=1` 重新生成），与本计划无关，单独处理。
