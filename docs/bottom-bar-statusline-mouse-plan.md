# 底栏重构、状态行布局自定义与鼠标补全

计划文件，工作完成后删除（AGENTS.md：临时计划放在 `docs/`，工作完成后移除）。

## 目标

1. 底栏第二行右侧显示后台计数：terminals 在前，agents 次之，warnings 最后，
   代替现在挤在状态行（第一行）里的 background-tasks。
2. 三者都可鼠标点击，进入对应界面。
3. 状态行布局可配置：每个条目可选左/右、第一行/第二行，并可配置空间不足时的
   溢出行为。
4. 输入框鼠标支持：滚动与点击。

## 已确认的决策

| 问题 | 决定 |
| --- | --- |
| Q1 | background-tasks 保留配置项，但不再渲染进状态行，只在第二行右侧出现 |
| Q2 | 布局配置用 `tui.statusLine.position.<id>` 多键形式 |
| Q3 | `/statusline` 新写专用设置页 |
| Q4 | 输入框滚轮移动光标 |
| Q5 | 溢出行为默认 `wrap-all` |

## 现状事实（已核实）

- background-tasks 由 `SurfaceValues::status_value` 渲染，
  见 `crates/tui/src/appearance/mod.rs:229-263`；当前在
  `DEFAULT_STATUS_LINE` 第二位，见 `crates/config/src/tui.rs:174-179`。
- 状态行组装在 `Appearance::status_line()`（`appearance/mod.rs:243-266`）
  与自由函数 `status_line()`（`:362-384`）。
- 底栏渲染在 `crates/tui/src/bottom_pane/footer/mod.rs`：`footer_height`
  （`:118-127`）、`render_footer`（`:130-156`）、`wrapped_footer`
  （`:158-187`）、`render_footer_row`（`:190-197`）。`render` 是 `&self`，
  命中区需要 `RefCell`。
- 底部布局在 `ChatComposer::layout_areas`
  （`bottom_pane/chat_composer/render.rs:32-56`），`footer_props`（`:76-91`）。
- 鼠标链路：`App::handle_mouse`（`app/mod.rs:374-432`）经
  `ChatWidget::handle_picker_mouse`（`chatwidget/mod.rs:193-202`）、
  `BottomPane::pointer_key`（`bottom_pane/bottom_pane_view.rs:84-93`）
  到视图的 `pointer_key`。命中区先例：`ScrollState.hits`
  （`bottom_pane/scroll_state.rs:15` 与 `:31-52`）、`Tasks.hits`
  （`tasks/mod.rs:67`，查询在 `tasks/input.rs:91-96`）。
- `/tasks` 不支持指定页签：`TasksEvent::Open`（`tasks/mod.rs:21-27`），
  `Tasks.section` 默认 `Section::Agents`（`tasks/mod.rs:76`），
  参数在 `app/workspace/tasks.rs:18-27`、`commands/tasks.rs:13-17`。
- warnings 无常驻计数：`ChatWidget.warnings`（`chatwidget/mod.rs:112`），
  只在启动时输出一行（`chatwidget/warnings.rs:25-29`），`/warnings` 时
  才合并并剔除已忽略项（`:33-38`、`:47-56`）。
- textarea 有 `scroll`，但 `effective_scroll` 强制光标可见
  （`textarea/layout.rs:240-266`）；`wrapped_lines(width)`（`:214`）
  可把屏幕行映射回文本。
- 鼠标已全局启用（`tui/alternate_screen.rs:143` 与 `:165`）。
- 配置校验：`cargo xtask check-config-reference` 要求每个
  `SettingSpec::new("key", …)` 在 `site/content/docs/config-reference.en.mdx`
  与 `config-reference.mdx` 都有行（`xtask/src/config_reference.rs`）。
  `SettingKind`（`crates/foundation/src/settings/mod.rs:96-131`）支持
  Boolean、Integer、Number、Text、Path、Choice、ChoiceOrText、TextList。

## 阶段 1：warnings 常驻计数与右侧三段数据

- `ChatWidget` 新增可见 warnings 计数缓存，在 `add_startup_warnings`、
  `show_warnings`、`dismiss_warning`、`command_failed` 后重算并下推，
  沿用 `BottomPane::set_status_line`（`bottom_pane/mod.rs:523`）的通路。
- `Appearance` 新增右侧行取数：terminals、agents、warnings，
  全 0 返回 None，否则只列非零项。
- background-tasks 不再进入 `Appearance::status_line()`。
- 测试：计数顺序、三者全 0、单项、忽略后减少。

## 阶段 2：底栏右侧渲染与溢出

- footer 支持第一行/第二行与左/右的位置模型，以及三种溢出行为
  （wrap-left、wrap-all、truncate），默认 wrap-all。
- `footer_height` 与 `render_footer` 一致地计算行数，避免裁切。
- 测试：宽窄两档的左右分配与换行、高度与实际渲染行数一致。

## 阶段 3：点击支持

- footer 新增命中区 `RefCell<Vec<(Rect, FooterTarget)>>`，
  `FooterTarget::{Terminals, Agents, Warnings}`；`BottomPane` 暴露查询，
  接入 `ChatWidget::handle_picker_mouse`。
- `TasksEvent::Open` 增加页签参数，`Section` 提升为 `pub(crate)`，
  同步 `app/workspace/tasks.rs` 与 `commands/tasks.rs`。
- 点击 terminals/agents 打开 `/tasks` 的对应页签；点击 warnings 走
  `AgentCommand::Settings(SettingsCall::Warnings)`，与
  `commands/warnings.rs:32` 同一条路。
- 无会话时沿用 `/tasks` 的错误提示。
- 测试：三处点击各自的路由，以及无会话时点击。

## 阶段 4：状态行布局配置

- 新增 `tui.statusLine.position.<id>`，取 left 或 right，行取 1 或 2。
- 新增 `tui.statusLine.overflow`，取 wrap-left、wrap-all、truncate，
  默认 wrap-all。
- 未知 id 与非法值容错，并按既有方式告警。
- 同步 `crates/config/src/tui.rs` 的 `SettingSpec` 与 config-reference
  中英两行，否则 `cargo xtask check` 失败。
- 测试：解析、默认值、非法值容错、布局生效。

## 阶段 5：`/statusline` 专用设置页

- 新页面承载位置与溢出设置，按 `PageOverride` 接入
  （参考 `commands/statusline.rs:26-32`）。
- 保留 `/statusline` 的参数形式（`commands/statusline.rs:50-75`）。
- 测试：页面渲染与保存。

## 阶段 6：输入框鼠标滚动与点击

- `TextArea` 增加屏幕坐标到字节位置的映射。
- 滚轮移动光标；点击定位光标。
- `ChatComposer::pointer_key`（`chat_composer/mod.rs:140-150`）扩到 textarea，
  命中区来自 `layout_areas` 的 `textarea_rect`。
- 测试：点击定位、滚轮前后移动、边界。

## 阶段 7：文档与快照

- 中英双语更新 `terminal-interface.en.mdx`、`terminal-interface.mdx`、
  `config-reference.en.mdx`、`config-reference.mdx`。
- 更新受影响的快照与断言：`bottom_pane/footer/snapshots`、
  `bottom_pane/chat_composer/snapshots`、`app/tests/appearance.rs`、
  `app/tests/workspace_commands.rs`、`appearance/tests.rs:13-46`、
  `bottom_pane/chat_composer/agents_navigation/tests.rs`。
- 删除本计划文件。

## 验证

每个阶段运行 `cargo fmt --all --check`、
`cargo clippy --workspace --all-targets --all-features -- -D warnings`、
`cargo xtask check` 与相关 crate 的测试。全部完成后运行
`cargo test --workspace --all-features`。

已知与本计划无关的既有失败：`hanafubuki-agent-mcp` 的 stdio 测试需要
`D:\Softwares\Python312\python.exe`，本机不存在。
