# pi-advisor 内置扩展移植计划

状态为实施中。本计划以 hanafubuki-agent 的 `8daee0e7` 和本地 [pi-advisor](../pi-advisor/README.md) 的 `6f17c808`（package 版本 0.16.0）为基线。交付对象是 standalone 和嵌入宿主共用的 Rust 内置插件；TUI 是其中一个消费者。下列默认值和分期是移植建议。

## 上游实际做了什么

| 用户行为 | 源码中的行为及边界 |
| --- | --- |
| 请求第二意见 | Executor 继续负责执行，Advisor 只看准备好的证据并返回建议。模型可用 `ask_advisor({})`、指定 `question` 或附上未验证的 `draft`。用户也可用 `/advisor-manual [focus]`。可分别指定模型和推理强度，同 Provider 同模型默认跳过，可显式允许。备用 Advisor 只在可回退错误后尝试一次，一次咨询只占一个名额。见 [咨询实现](../pi-advisor/src/tools/consultation.ts)。 |
| 决定何时审查 | 计划、连续失败和完成前审查主要通过调用提示引导 Executor。真正的自动拦截发生在工具执行前，依据连续重复的工具签名触发。普通咨询返回 Markdown；自动门控单独解析 proceed、revise、blocked，并按失败策略处理。Jev 另有按轮数触发的主动咨询。见 [生命周期](../pi-advisor/src/tools/register-lifecycle.ts)、[门控实现](../pi-advisor/src/tools/loop-gate.ts)。 |
| 选择披露内容 | 当前分支的对话、工具调用和结果可按策略选择；Git 有关闭、摘要和完整差异三档。项目规则、偏好说明、具名文件和图片有独立条件。Scout 用 Executor 从允许披露的证据中选组；Jev 用摘要决定是否值得咨询。两者都会增加一次独立的模型或分类服务请求。见 [披露约定](../pi-advisor/docs/privacy.md)。 |
| 追问与反馈 | `followUpTo` 引用短时内存里的咨询前缀。`record_advisor_outcome` 按 adviceId 记录采纳及验证情况；`/advisor-stats` 汇总实际保留的窗口。使用量、费用、会话摘要和外部 Herdr 状态另有用途。见 [追问](../pi-advisor/src/follow-up.ts)、[反馈与统计](../pi-advisor/src/outcome-stats.ts)。 |
| 启用与配置 | 持久和仅当前会话两种激活方式；Simple 模式可只提供自愿咨询。上游有六个斜杠命令、三个模型工具、一个随包 Skill。`advisor_config` 可读配置，写入时展示差异并由用户确认。原项目另有 Pi 专用设置界面、配置文件、模型选择器和分类器凭证存储。见 [注册入口](../pi-advisor/extensions/index.ts)、[配置说明](../pi-advisor/docs/configuration.md)、[真实默认值](../pi-advisor/src/config/defaults.ts)。 |

## 原生版的激活条件

Advisor 默认未激活。只有**插件已启用、主 Advisor 的 Provider 和模型已配置、用户已在适用作用域显式激活 Advisor** 三项同时成立，才向 Executor 发布 Advisor 工具及调用提示。自动审查和 Scout 还须打开各自对应的功能。启用插件和保存模型均只完成各自的操作。手动激活无需预先探测远端模型；缺少主模型时引导用户配置，保存后仍由用户决定何时激活。

用户通过 `/advisor` 激活、`/advisor-off` 停用，嵌入宿主提供等价的用户操作。持久模式记住用户的显式选择，会话模式仅对当前会话有效，新会话从未激活开始。模型配置工具不能替用户激活。

插件已启用且主模型已配置时，用户可以通过 `/advisor-manual [focus]` 单次授权咨询。未激活时，这次请求直接交给 Advisor；完成、失败或取消后仍是未激活，Executor 的工具可见性不变。宿主的手动咨询入口遵循同一规则。

## 第一阶段：让用户能配置并实际问到第二个模型

1. 在 [内置插件目录](crates/agent/src/plugins/mod.rs) 新增 `advisor` 模块。在 [公共 manifest](crates/agent/src/host/manifest.rs) 通过 `AgentPluginRow::from_settings(Capability, …)` 装配，在 [descriptor 列表](crates/agent/src/host/descriptors.rs) 放入同名 descriptor 和 `PluginOptionSchema`。使用现有 `plugins` 集合启停整行。插件安装、启动或仅打开设置均不发模型请求。缺少 Advisor 模型时可配置；主动咨询时给出选择入口，不偷偷选一个模型。
2. 按 [subagents 的选项声明方式](crates/agent/src/plugins/subagents/options.rs) 定义类型化选项。首批为 Advisor 的 provider ID、model ID、推理强度、可选备用 provider/model、同模型跳过和历史文本预算。Executor 默认取正在工作的会话。Provider ID 引用 Agent 已配置的 Provider，模型 ID 保持原样（可含斜杠）；已有凭证仍由 Provider 持有。首版不需要新模型选择器，可用现有文本、选择和重置控件。
3. 所有新增的**持久用户偏好**由 `settings_catalog()` 暴露，以 `pluginOptions.advisor.<key>` 写入，调用 `write_setting_values()` 保存或重置，按现有行配置机制应用。[目录组装及写入口](crates/agent/src/service/settings/mod.rs)。以 Rust 默认值为单一来源；同时保存主模型的 provider/model 时走同批写入。设置页、CLI、嵌入宿主和以后加入的命令均消费同一声明。无需 `advisor.json`、第二份 schema 或 TUI `Attach::Row`。动态记录类型见第二阶段。
4. 参考 [recap](crates/agent/src/plugins/recap/mod.rs) 做独立、可取消的模型请求。模型调用和手动入口共用一个咨询服务，沿用 Provider 解析、model-api 的协议适配、重试归类和取消。每次使用选定 Advisor 的上下文窗口及图像能力。请求中不提供执行工具，不创建子会话。用户取消、插件停止或上下文准备失败时终止；可回退的主模型故障才使用已配置的备用模型。返回实际回答模型、正文和上游提供的用量；备用模型也失败时保留两个失败原因。
5. 模型入口注册 `ask_advisor`，本阶段同时接通用户激活、停用和单次手动咨询入口，对应的类型化 facade 方法按 [feature 分派](crates/agent/src/service/features.rs) 查找运行中的插件服务。TUI 呈现调用中、完成、跳过或失败，详情可展开。手动调用须让用户看到回答，工具调用则将结果交还 Executor。初版请求包含问题、可选草稿和当前会话最近的用户/助手文本；先交付这条闭环，再增加披露来源。工具和提示从第一阶段起遵守上述激活条件。
6. 验收同一设置在 standalone 和嵌入实例都能发现、写入、读回、重置；停用插件后工具消失，手动接口报告功能不可用，其余工具继续工作。覆盖只启用插件、只完成模型配置和执行 `/advisor-off` 后的状态：下一步工具计划均无 `ask_advisor`，正常对话不产生 Advisor 请求。未激活时执行一次手动咨询，验证其成功、失败或取消均不改变激活状态；随后显式激活，验证下一步工具计划出现 `ask_advisor`。用可控 HTTP Provider 捕获**真实发送的请求**，验证模型选择、无工具、草稿与证据可区分、取消不回退、主备只算一次。另用两个实际配置的模型进行一次端到端咨询，以 Advisor 回复中的唯一输入标记和服务端实际路由共同确认链路。

## 第二阶段：补齐咨询工作流和上下文

| 工作项 | 落地方式和验收 |
| --- | --- |
| 模型与激活 | 扩展第一阶段的激活入口，支持持久和会话作用域；`/advisor-models`、`/advisor-settings` 定位统一设置。全局关闭插件优先。验证重启后持久模式恢复用户的显式选择，会话模式的激活不会传播到其他会话或写入全局设置。覆盖新建、恢复、分叉和插件重载，确保子代理保留启动时指定的模型。会话级工具可见性使用带 session_id 的 [AgentToolSource](crates/contracts/src/tools/source.rs)，在构建工具计划时只读本地状态；普通 handler 的 ToolPlanContext 没有会话 ID。显式固定 Executor 时复用会话模型切换机制；允许列表比较 Executor 的 provider/model 身份。 |
| 历史与工具策略 | 用 [AgentConversationReader](crates/agent/src/conversation_reader.rs) 按固定 revision 分页读取当前会话。已有 visit_messages 只给文本；工具证据走 transcript page，将调用和结果按 ID 配对。按精确工具身份实施 full/summary/exclude，命名使用共享 ToolName 投影。摘要仅含状态和大小；exclude 去掉调用和结果；不传私有推理、原始 wire 或其他会话。工具披露策略若需要键值映射，用现有 SettingsCollection 的具名记录和增删改操作，统一注册；无需通用 JSON 编辑器。验收查看出站体，包含同名不同 namespace、被压缩的对话和大结果。 |
| 工作区、规则和附件 | 从会话冻结的 workspace 和已有信任/权限、文件、图像读取入口出发。Git 只在配置允许时收集，摘要只有变更元数据；完整差异、具名 tracked/untracked 正文分别受用户许可和本次请求约束。保留上游一次性 tracked 授权语义，untracked 默认关闭。项目规则和可选偏好说明依据当前仓库信任，明确来源；不要从 Pi 私有目录寻找全局指令。附件走已有路径和批准边界，单靠工具参数不能扩大授权。对支持图片的模型发送真实图片；文本模型、summary/exclude 策略或预算不足时准确说明省略内容。可选文本脱敏在出站前统一处理。用临时 Git 仓库和捕获的多模态请求验证文件内容、像素及被拒内容，含 Windows 路径。 |
| 历史预算和追问 | 上游默认历史 15,000 字符、Git 最多 20,000 字符并受总预算约束；作为外发内容大小设置，不作为运行期限。支持无历史及按模型窗口取可用历史。按实际保留的文本、图片字节约束，不按会话消息总数中断。追问只暂存最终允许外发的前缀，按会话、来源和披露配置隔离。新输入、权限收紧、会话关闭及过期后重新准备；不落盘。重试/追问时也遵守实际目标模型能力。 |
| 结果、配置工具和引导 | 开启 outcome 记录后提供 `record_advisor_outcome`、`/advisor-stats`。用项目数据目录保存最小采纳/验证元数据，按实际字节轮转，坏记录单独跳过，不保存提示、路径或答案原文。用量按实际请求归属、避免重复计入主模型，缺少费用时显示未知。`advisor_config` 只读取和修改 Advisor 的可公开键，读不暴露密钥；用户看到具体键和值差异并确认后，通过相同设置 API 保存，取消无写入。模型选择和外发目的地由用户决定。模型工具写设置时按写操作声明，保留 Plan 模式和宿主批准约束。将工作流说明适配为内置 system Skill，配合本项目加载机制；同步工具说明、状态 presenter 与手动界面。 |

## 第三阶段：可选审查和外部集成

- **激活验收。** 预先打开门控和 Scout，保持 Advisor 未激活，执行重复工具调用及普通对话。检查工具计划、提示和捕获的请求，确认自动路径均未运行。显式激活后覆盖相同触发场景，再停用并验证这些路径随之关闭。
- **审查提示与自动门控。** 计划、失败、完成及自定义调用时机通过插件 prompt contribution 表达。Simple 模式只做自愿咨询。自动重复门控独立 opt-in，默认关闭；记录的是连续相同工具签名，轮询可能每次得到新结果。显式启用以后才计数，在现有 before_tool_use 决定是否允许当前调用。proceed 放行，revise 拒绝当前调用并给出理由；blocked/协议错误/模型故障按明确配置处理。建议默认告警继续；若用户显式选择 block-session，要能从用户入口解除，且状态只影响当前会话。Advisor 自身工具排除在重复检测之外。普通咨询给出的负面意见不会终止主任务。覆盖重复轮询、并发会话及解除后继续执行。
- **预算。** 每会话咨询次数默认为无限；仅对显式预算计费。模型工具、手动与自动咨询共享实际咨询的计数口径，回退不重复扣减；在调用尚未开始即取消或被拒时释放预留。模型或网络故障局限于本次咨询，已启用门控时按其显式策略处理。无需新增整个 turn 的时间、token 或工具次数上限。
- **Scout。** 用户选择后，先向 Executor 发经过同一披露策略的证据组清单，只让它返回组 ID，再从本地允许的原始证据重组；不发像素给 Scout。失败或超时沿用已经准备好的允许外发上下文；父操作取消则两者都停。单独展示其用量。验收应包含无效选择及双 Provider 实际接收的不同请求体。
- 外部 Jev 分类器已由用户从移植范围中移除。原生审查提示、可选重复工具门控和用户主动咨询均由 Agent 与配置的 Advisor 完成。
- **显示与生态取舍。** 折叠回复、预算反馈、用量详情、页脚和会话总结按其作用接入 presenter、现有状态栏或会话展示；持久偏好仍由目录注册。Herdr 是 Pi 的外部状态协议，Pi Codemode、虚拟模型、i18n loader 和 `.pi` 配置也依赖其运行环境。原生版以现有工具协议、Provider、本地化文档和活动事件覆盖相应用户操作。**本计划不承诺 Herdr 线协议或 Pi 文件兼容**。Scout 和门控各自能独立验收，不作为基本咨询可用的前置条件。

## 实施约束和交付检查

- 按 [插件规范](site/content/docs/plugin-rules.en.mdx) 注册贡献，插件停止时由生命周期撤销。模型 IO 放在可取消的调用中，涉及磁盘的同步读取使用 BlockingService。不要在 tool planning、设置目录读取或 TUI 渲染中访问远端。权限、路径和审批沿用现有契约；对新的外部响应在入口解析一次。git 等子进程走项目的 ExecutorManager。避免新增独立调度器、全局会话单例、另一套审批或第二个插件框架。
- 设置验收以用户路径为准：使用嵌入服务和 standalone 各写一次；检查有效值、热应用或行重启、恢复、重置及目录显示。每加入一项持久选项，在同一改动中接通运行时行为和对应测试；实验功能未实现前不预注册占位选项。诊断日志遵守 AGENTS.md 的 trace、截断及凭证规则。保留会话规范工具结果，避免额外持久化出站上下文。
- 每阶段先执行相关 Rust 测试。模型可见变化在 Linux 运行 `HANAFUBUKI_AGENT_UPDATE_GOLDEN=1 cargo test -p hanafubuki-agent --lib golden` 并审阅差异；TUI 只更新受影响的快照；历史格式未变时保留历史语料。完整验收运行 `cargo fmt --all --check`、`RUSTFLAGS="-D warnings" cargo check --workspace --all-targets --all-features`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`cargo xtask check`、`cargo test --workspace --all-features`。分别记录本地、各平台 CI 和真实 Provider 实测结果。
- 同步更新中英文 plugin-rules、architecture、embedding、terminal-interface 和受影响的配置参考，说明启用方式、外发数据、失败影响、可选预算及嵌入调用示例。遵循 [构建与测试](site/content/docs/building-testing.en.mdx) 的站点检查；生成物从源再生。上游是 MIT，直接移用提示词、Skill、代码或测试材料时随移用内容保留原始版权与许可。分阶段记录交付范围及未验收项，实施完成后按项目约定清理临时计划。
