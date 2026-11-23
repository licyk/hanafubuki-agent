# 模型流中断的报错可读性

状态：计划（诊断已完成，实现未开始）

影响范围：`crates/model-api`、`crates/kernel`，以及
`site/content/docs/architecture{,.en}.mdx`、`site/content/docs/internals{,.en}.mdx`。

## 背景：真实触发场景

生产故障来自会话 143 的第 3 轮，`2026-11-23T10:55:44.274Z` 失败。用户看到的原文是：

```
agent model failed: model response stream interrupted after the first model event:
model response stream failed after establishment
```

实际发生的事，按字节还原：

1. 请求体 23,963,414 字节，1,468 个 input item，48 个 tools，其中内联
   `data:image` 载荷 21,964,614 字节，占 91.7%（该轮反复 `view_image` 截图回灌）。
2. 请求打到本机网关（`http://127.0.0.1:10600/v1`），转发给上游的
   `agent_chat_generation`；上游体 32,118,468 字节。
3. 上游只发送 632 字节（`response.created` 与 `response.in_progress` 两个 SSE 事件）
   就断连。网关记为 `error_code=internal_error`、`error_message=RequestError`，
   调用轨迹里 `error = "ReadError"`、`response_body.bytes = 0`。
4. Agent 在 `crates/model-api/src/responses/mod.rs:251` 命中 `Some(Err(_))`，把底层
   `reqwest` 错误丢进 `_`，只留一个固定字符串。
5. `crates/kernel/src/service/sampling/stream/events.rs:31` 因 `started_response_id`
   已存在，走「首事件之后中断」分支，包装成 `AgentError::Model`，
   而不是 `AgentError::ModelStreamInterrupted`。
6. `crates/kernel/src/service/sampling/request.rs:379` 的重试只匹配
   `AgentError::ModelStreamInterrupted`，所以这一次**没有任何重试**。
7. `crates/kernel/src/model/error.rs:136` 渲染为 `agent model failed: {message}`，
   经 `service/turn/terminal.rs:25` 的 `bounded_diagnostic`（4 KiB）交给终端界面的
   `chatwidget/turn.rs:356` 的 `new_error_event`。

要解决的问题：界面上只有一句像本地缺陷的英文，既没有「上游断连」这个事实，
也没有请求体规模、图片占比这类可操作线索。

## 必须先接受的既有契约

- `crates/model-api/tests/http_contract/retry.rs` 的
  `stream_failure_after_first_model_event_is_never_replayed`：首个模型事件之后不重放，
  是刻意为之，用来防止重复输出。不要为「让它重试」而改这条测试。
- `site/content/docs/architecture.en.mdx:353` 写明同一件事：一旦推理、文本或工具项
  已经开始，拒绝就以结束本轮来处理，以免产生重复输出。
- 因此本次**不做重试语义变更**，只做可读性。重试是独立议题，见文末。

## 改动计划

### A. 把底层错误带进消息（收益最高）

`crates/model-api/src/responses/mod.rs:251`：

```rust
Some(Err(_)) => {
    return self.fail(ModelError::StreamInterrupted(
        "model response stream failed after establishment".into(),
    ));
}
```

该处已有 `provider: Arc<ModelProvider>`，而
`crates/model-api/src/provider.rs:322` 提供了
`pub(crate) fn bound_diagnostic(&self, value: impl Into<String>) -> String`：
它替换凭据与请求头为 `[redacted]`，并按 `MAX_DIAGNOSTIC_BYTES`（4 KiB）截断。

改为用 `error` 拼装消息，并经 `bound_diagnostic` 落地，例如：

```rust
Some(Err(error)) => {
    let message = self
        .provider
        .bound_diagnostic(format!("the response body failed after the stream was established: {error}"));
    return self.fail(ModelError::StreamInterrupted(message));
}
```

注意：`reqwest::Error` 的显示可能带 URL。凭据与请求头已被处理，URL 查询串要另行确认是否
可复用 `provider.rs:88` 的 `endpoint.set_query(Some("[redacted]"))`；若不确定，就只取
错误分类（`is_timeout`、`is_connect`、`is_body`、`is_decode`）与 `source()` 文案，
不要把整条 URL 拼进去。

同样处理两处：

- `crates/model-api/src/streaming.rs:222`（`RawSseStream::next_frame`，
  Chat Completions、Anthropic Messages、Gemini 共用）
- `crates/model-api/src/model_catalog/mod.rs:283`（`model catalog response body failed`）

### B. 换成语义正确的错误变体

`crates/kernel/src/service/sampling/stream/events.rs:23-33` 目前把「首事件后中断」
降级为 `AgentError::Model`，于是丢掉了可被界面与测试识别的类型。建议保留「不重试」，
但使用专门的变体，例如：

```rust
AgentError::ModelStreamInterruptedAfterOutput { message: String }
```

`request.rs:379` 的重试分支是精确匹配 `ModelStreamInterrupted`，新增变体天然不重试，
无需再加条件。这样界面能拿到结构化的终止原因（`terminal_reason_for_error`），
`agent model failed:` 这个兜底前缀也不再套在这类故障上。

新增变体会波及 `crates/kernel/src/model/error.rs` 的 `Display`、
`terminal_reason_for_error`，以及 `crates/kernel/src/service/turn/mod.rs:30` 的匹配组
（其中已有 `ModelStreamInterrupted(_)`）。`cargo check` 会列出全部位置，逐个补齐，
不要用 `_ =>` 掩盖。

### C. 区分「没重试」与「重试耗尽」

`crates/kernel/src/service/sampling/request.rs:404` 只在重试次数耗尽时给出
`ModelStreamRetriesExhausted`。B 之后本类故障不发生重试，需要在文案上区分这两种情况，
而不是让用户猜测。

### D. 附带可操作线索（需单独确认）

最有用的线索是请求体为什么这么大。相关事实在内核侧已经可得：
`crates/kernel/src/budget.rs` 的 `maximum_tool_output_bytes`、
`record_tool_output_bytes`、`tool_output_bytes`，以及
`crates/kernel/src/service/compression/tools/decompress.rs` 的 `image_budget`。

在同一条错误里附带「本次请求约 N MB，其中图片约 M MB」，用户立刻知道下一步该压缩历史
或另开会话。但这属于新增功能而非改文案，先只做 A、B、C，D 单独征询。

## 验证

从仓库根目录，PowerShell 下执行（MSYS2 的 `link.exe` 会遮蔽 MSVC，POSIX shell 下
Rust 构建会失败）：

```
cargo fmt --all --check
$env:RUSTFLAGS="-D warnings"; cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo xtask check
cargo test --workspace --all-features
```

要点：

- 这些文案没有任何快照断言，改文案不会破快照；但
  `crates/kernel/tests/loop_contract/sampling/retry.rs`、
  `crates/model-api/tests/http_contract/retry.rs`、
  `crates/model-api/tests/provider_protocols/tolerance.rs` 使用了
  `matches!(..., StreamInterrupted(_))`，变体改动必须同步。
- `crates/kernel/tests/loop_contract/sampling.rs:120` 直接构造了
  `"model response stream ended before a terminal event"` 字符串，改文案时留意。
- 回归测试要先看到它在修复前失败。建议在
  `crates/model-api/tests/http_contract/retry.rs` 增加一条：上游发送首个事件后断连，
  断言 `StreamInterrupted` 的消息里包含底层错误信息，能区分「有信息」与「只有固定字符串」。
- 文档双语同步：`site/content/docs/architecture{,.en}.mdx` 第 353 行附近，
  `site/content/docs/internals{,.en}.mdx` 的「Tolerant decoding of provider streams」
  表格。文档改完按需运行 `bun run format:check:site`、`bun run site:check`、
  `bun run site:test`。
- 模块大小：`cargo xtask check-module-size` 限制生产文件 600 代码行。
  `stream/events.rs` 已 24.5 KB，`sampling/request.rs` 已 20 KB，加代码前先看行数；
  压线要报告，不要硬塞，也不要顺手重构无关模块。

## 不要做的事

- 不要为让这类故障重试而改
  `stream_failure_after_first_model_event_is_never_replayed`
  或 `architecture.en.mdx:353` 的语义，那是独立的设计决策。
- 不要把这句话「藏起来」或改得更含糊；目标是信息更多，不是更少。
- 不要往消息里塞请求体内容、图片 base64、凭据或完整 URL 查询串；
  `bound_diagnostic` 的 `[redacted]` 与 4 KiB 截断必须保留。
- 不要在 `crates/model-api` 之外解析 SSE 或重复业务逻辑；错误分类只在该 crate
  的边界做一次。
- 不要为极不可能的情况增加防御分支。

## 未决：首事件后零输出的重试

「首事件后中断」既可能是上游过载或断连，也可能是响应真的坏了，当前代码统一当致命失败。
本次故障恰好零输出（`saw_output_item == false`，`events.rs:18` 已有这个判据）。

若希望「零输出且首事件后立即断连」也能重试一次，那是契约变更，必须：

1. 先向用户确认；
2. 同步修改 `retry.rs` 的那条测试与 `architecture.en.mdx:353`；
3. 单独提交，不与文案改动混在一起。

倾向：先完成 A、B、C（纯可读性，零契约变更），把重试作为后续独立提案。
