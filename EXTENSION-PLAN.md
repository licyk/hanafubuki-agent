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

现在 lowering 只能表达 boolean/integer/number/string/enum/字符串数组（`crates/packages/src/options/lower.rs`）。补两个本机需要、且能映射到既有 `SettingKind` 的：

- `enum` 常量数组 → `SettingKind::Choice`；
- `x-secret: true` → `SettingKind::Text { secret: true }`（今天"没有任何选项是秘密"，插件没法放 token）。

其它关键字仍不解析，写进文档而不是加校验。

验收：`crates/packages/src/options` 的单元测试与 `plugins validate` 的 issues 列表。

### 任务 3：内置插件的选项改用声明式清单

`plugin-web` 的 60 个 `SettingSpec` 逐条对应 `SearchOptions` 字段；改为由 `configSchema` lowering 产出，Rust 里只保留 `search_options_from_value` 这个解码函数。顺序：先迁移 `plugin-web`（它的 schema 最完整，且已有 `search_options_from_value` 兜住解码），再决定是否动其余内置插件。

验收：`crates/plugin-web/tests/plugin.rs` 与设置中心的选项快照不变；`/config` 里 web 段仍能读写每一项。

### 任务 4：会话与记忆工具的动作面

在任务 1 的通道上加三个会产生副作用的动作。副作用的授权**沿工具调用流动**：`contributes.tools[]` 新增可选 `approval: "<kind>"`，宿主为该工具生成的动作按调用自带的 `ApprovalAction` 放行，未授权的调用直接拒绝。这样不引入插件级长期授权，也复用现有审批 UI。

| 方法 | 效果 | 需要的工具授权 |
| --- | --- | --- |
| `session.list` | 已有会话（id、标题、更新时间） | 无（只读） |
| `memory.write` | 写一条记忆 | `DomainMutation` |
| `terminal.spawn` | 起一个受管后台终端 | `InteractiveExecution` |

验收：`crates/plugin-packages/tests/guests/` 新增用例证明"未经工具授权的 `invoke` 被拒、经授权后通过"，并覆盖 `terminal.spawn` 的进程随 fiber 结束被回收。

### 任务 5：声明式渲染面

第三方工具目前只有 fallback 展示（结构化结果列键值）。新增：

- `contributes.toolPresenters[]`：`{ tool, fields: [{ key, label, kind }] }`，kind 沿用选项词汇（text/integer/boolean/choice/list）；
- `contributes.statusItems[]`：`{ id, label, action? }`，`/plugins` 里展示并从 `invoke` 取值。

两者都注册为普通的 `ToolPresenter`/状态视图，不新增框架。渲染端在 TUI 的插件页与设置中心复用。

验收：TUI 快照（`crates/tui` 的截图测试）与 `plugin-rules` 文档同步。

### 任务 6：不重启的配置迁移

`GuestPackage::entry_name` 把 `settings` 之外的字段都算进身份，`settings` 变化本身不重命名，但 `#host` 行的 `config` 变化会让行重建。用 kardia 的 `migrate_binding`/`refresh_consumers`，让 `agent` 动作与 `settings` 视图在设置变化时原地迁移，而不是重启 guest。

验收：`crates/plugin-packages/tests/guests/` 覆盖"改设置后 process_id 不变、`settings.get` 返回新值"。

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
