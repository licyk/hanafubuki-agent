# Codex 异步提问与侧边栏移植计划

开发分支 `feat/async-input-sidebar`，起点 `cbac9eeb`。行为基准是同级 codex 仓库的 `cf3ff1530d82`。以下路径以各自仓库根目录为基准。

## 源码对照

| 功能 | Codex（codex-rs/ 下） | 当前项目 |
| --- | --- | --- |
| 异步提问 | core/src/tools/handlers/request_user_input_async.rs、core/tests/suite/request_user_input_async.rs、tui/src/bottom_pane/async_questions/、tui/src/chatwidget/questions.rs | kernel 的 core_tools 和 service/interaction/user_input.rs 目前实现阻塞提问；TUI 已有问卷、输入队列和 steering |
| 对话归属 | tui/src/app/side.rs、side_companion.rs、sidebar_close.rs | agent/src/service/children.rs 按父会话返回同一个临时子会话；tui/src/app/workspace/routing.rs 仅识别一个 second 和一个 child |
| 选择器与标签 | tui/src/sidebar_chooser.rs、sidebar_tab_strip.rs、app/sidebar.rs、app/sidebar_subagents.rs | tui/src/app/workspace/panes.rs 已有宽窄布局、分隔线拖动和焦点；agent_bridge 已能订阅子会话 |
| 文件与图片 | tui/src/app/sidebar_files.rs、sidebar_document.rs 及其 watch 子模块、tui/src/sidebar_image.rs、transcript_image | workspace-fs 负责访问边界，session_workspace_roots 提供实际会话的所有根；TUI 已使用 image 解码库，有 transcript_view 和 Markdown 渲染 |

## 实施顺序与验收

1. **异步工具及回答。** 在现有工具计划内登记 typed schema，处理一个或多个有标题的问题及可选建议。成功立即向模型返回 `{"accepted":true}`；沿会话事件把题目交给界面。标识由调用和题号组成，答案作为用户输入送入同一会话。验证模型可继续下一步、多个问题独立作答、默认首选须主动提交、自由文本、草稿、接受失败、已答回放及结束回合。核对上游按会话和前端能力开放的条件，以及无交互 exec 行为。同步模型可见的 request goldens。
2. **侧对话生命周期。** 复用内核现有的子会话、fork、事件订阅和后台终止。区分新建与显示已有会话；保留上游临时和附属侧对话的关闭、恢复、归档差异。路由以实际会话为准，隐藏、切换、关闭一个标签时其他标签的草稿、滚动位置、工具事件和运行状态各自保留。测试父会话切换、并行输出和对子会话的精确关闭。
3. **Open in sidebar。** 接入选择器、标签条、最近访问、键盘导航、窄屏显示、隐藏与重新打开。由已绘制行和标签的位置进行鼠标命中，支持悬停、点击、滚轮、分隔线拖动。滚轮只滚动指针下的视图。保留 fullscreen 和原生 scrollback 的输入方式。
4. **Subagents。** 选择器从当前会话树读取 agents；查看运行状态和记录，使用已有 child bridge 订阅，遵循上游只读观察与可交互会话的区别。验证子代理退出及选择切换后主会话和侧对话仍可使用。
5. **文件与图片。** 从当前及较早的 transcript 分页收集文件链接，最近提及优先；允许浏览会话工作目录。复用工作区访问策略、PathKey 与原有文件搜索。支持文件行定位、文档链接、文本滚动和查找、内容刷新时保留阅读位置。按实际格式解码 PNG、JPEG、GIF、WebP；支持的终端绘制图像并在换标签和 resize 时清理旧画面。文件消失、权限错误、坏图或终端能力不足只影响相应预览。
6. **验证与文档。** 用真实 facade、工具循环及 TUI 输入入口测试，核对上述上游测试和快照。更新涉及行为的中英文开发、嵌入、工具和终端文档及移植署名。运行 cargo fmt、带警告检查的 workspace check、Clippy、cargo xtask check、workspace tests。分别报告本机终端、操作系统与 CI 实际验证范围。

历史沿用本项目的类型和存储契约。沿用已有队列、订阅、渲染器和路径边界；文件 I/O 和图片解码放到绘制循环外。只有实际资源或协议需要才加限制；单个坏条目局部呈现错误，继续处理其他条目。
