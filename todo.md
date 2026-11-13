## W1: a POSIX shell for `exec_command` on Windows (2026-11-13)

- [x] W1: `shell: "posix"` on Windows runs the bash (else sh) of Git for
      Windows or MSYS2 when one is installed, instead of failing with
      `POSIX shell is unavailable on this platform`; the default shell and
      every other platform and shell unchanged. Goldens regenerated on
      purpose (one sentence in `exec_command`'s description).
      Done (2026-11-13):
      - execution: `msys.rs` (Windows, and tests everywhere).
        `PosixShellSearch` looks, bash before sh at each step, (1) when the
        Agent's own process has `MSYSTEM`, on the Agent's `PATH`; (2) on the
        command's `PATH`, skipping hits under `%SYSTEMROOT%\System32`,
        `SysWOW64`, `Sysnative` and any `WindowsApps` directory (WSL's
        launchers), compared by `PathKey`; (3) in `usr\bin` of
        `%ProgramFiles%\Git`, `%ProgramFiles(x86)%\Git`,
        `%LOCALAPPDATA%\Programs\Git`, `%SystemDrive%\msys64`. A hit in a
        `bin` directory with a sibling `usr\bin\<name>` (Git for Windows'
        launcher) is replaced by that shell: the launcher starts the shell as
        its child, and pipe mode puts a command in its Job Object only after
        the spawn. Not found: the error names what was searched and
        `"power_shell"`. `add_posix_shell_environment`: the shell's directory
        first on `PATH` (the clean map's own spelling of the key), and, unless
        the command has them, `CHERE_INVOKING=1`, `MSYS2_PATH_TYPE=inherit`
        and the Agent's `MSYSTEM`. `command_line_script`: the MSYS runtime
        parses its command line itself (`dcrt0.cc` `build_argv`/`globify`:
        splits at `\n` outside quotes, globs `*`, takes `'` as a quote, reads
        `\\` as `\` inside quotes), so argv[2] is the script with a space in
        front when it has no space or tab and each backslash run not before a
        `"` doubled; the approval's `script` stays as written. Scripts over
        8000 UTF-16 units are refused (`InvalidRequest`): the runtime cuts an
        argument past 8186 short silently (measured on both flavours).
        `exec_shell_command` returns a `Result`. Unix, PowerShell and cmd
        unchanged (`unified.rs`, `windows_powershell.rs` untouched).
      - plugin-execution: the sentence in the description; a Windows test
        that `shell: "posix"` through `parse` reaches `usr\bin\bash.exe` and
        an omitted shell PowerShell.
      - tests: `msys/tests.rs` (fakes: WSL launchers skipped, order, the Git
        launcher, the message, the environment, the spelling, the limit);
        `tests/contracts/windows_posix.rs` (both flavours, pipe and PTY:
        workdir with `login` true and false, `uname`/`ls` and `cargo`,
        quoting with and without a space, the longest script and the refusal
        past it, exit codes, Ctrl+C and stop leave neither bash nor its
        `sleep` running); `tests/windows_sandbox.rs` (no harness, the helper
        dispatched first), **ignored**: under the sandbox's restricted token
        both flavours' bash exit `0xC0000142` with `*** fatal error -
        couldn't create signal pipe, Win32 error 5`, pipe and PTY. Not worked
        around; documented (tools, permissions); the user's call.
      - docs: `tools#commands`, `environment#commands`, `permissions` (the
        sandbox limitation), both languages.
      - CI on 981765d: https://github.com/licyk/hanafubuki-agent/actions/runs/43449543844
        (Rust on Linux, Windows and macOS green; the Windows log shows both
        flavours, `C:\Program Files\Git\usr\bin\bash.exe` and
        `C:\msys64\usr\bin\bash.exe`, running the six `windows_posix` checks,
        pipe and PTY, and the sandbox check reported ignored) and Site
        .../runs/43449543923 green. Evidence from scratch runs on branches
        since deleted: the runner's inventory and which variable each
        outcome needs (43444183178), the length limit (43445539086), the
        sandbox failure on both flavours, pipe and PTY (43447300320).

## Flaky Windows tests (2026-11-13)

From CI's failed jobs over the last 60 runs; each passed on a re-run.

- **Durable writes on the runner's system disk** (Windows): `plugins::subagents::tests::messages_steer_…` 2×,
  `…the_depth_and_running_limits_…` 1×, `…roles::a_child_is_its_parent_narrowed_by_its_role` 1×,
  `…roles::a_child_takes_its_roles_model_and_effort_else_its_parents` 1× (history writers past the 10 s shutdown),
  `tests::sessions::children::a_driven_child_runs_a_turn_answers_and_is_interrupted` 1×. Every test's Agent keeps its
  data under `TEMP`, on C:, where a history append (each flushed to disk) took 200 ms at the median and up to 12.4 s
  once four tests wrote at once (the appends' `agent_diag` log, 12 runs of the agent's unit tests right after a full
  build). Turns, a cancellation's durable `Cancelled` (`CONFIRMATION_DEADLINE`, 5 s → `CancellationNotConfirmed`) and
  the tests' 10 s and 20 s waits ran past their deadlines. Fix (`ci.yml`): on Windows `TEMP`/`TMP` are a directory the
  job creates under `runner.temp` (D:, the local disk), with the user's own inheritable full control as a profile's
  temporary directory has (D: grants it only through Administrators, which the sandbox's unelevated token lacks: without
  it `sandbox::windows` could not write a temporary file the test had made). Appends then took 7-110 ms at the median,
  at most 433 ms; the agent's unit tests 55-106 s instead of 178-271 s (47.5 s in CI, from 294.7 s). The 5 s
  confirmation stays as documented (internals, "Cancellation"): on a disk this slow an interrupt is still carried out,
  but reported as not confirmed.
- **PowerShell silent for a whole 30 s yield** (Windows): `plugins::review::tests::a_review_reads_only_and_its_findings_reach_the_parent`
  4× (its `git --no-pager log`), and on the verification PR, with `TEMP` on D:,
  `tests::executions::unified_exec_completes_only_after_manual_exact_approval` 1×. Partly the disk: the first PowerShell
  of a run took 15-21 s with `TEMP` on C:, 1.4-3.2 s on D:, later ones under 0.7 s. **Open**: the D: occurrence is not
  explained; four runs replicating CI (with a 2 s process sampler, and cold launches timed at 300-430 ms) showed none,
  every PowerShell finishing within 634 ms. The exec test's assertions now print the tool's result, so the next one
  says what it returned.
- **`app::tests::queue::esc_interrupts_and_sends_the_waiting_steers_as_one_turn_which_can_be_edited`** (TUI, Windows 1×):
  a product race. A steer in flight when Esc interrupted could reach the Agent after the cancellation, which refuses it;
  when the refusal arrived before the turn's end, the input queue held it for a later turn, and the turn the interrupt
  started lost it. Fix (`chatwidget/input_queue.rs`): after such an Esc no steer goes to the turn being interrupted, and a
  refused one waits with the others, in order; a unit test pins the interleaving.
- **`app::tests::queue::esc_returns_a_scrolled_conversation_to_the_bottom_before_it_interrupts`** (Linux, locally) and
  four more queue tests: their "running turn" was an answer sent after 3 s, which a loaded machine outlived before the
  test's Esc. Fix: the answers those tests interrupt or outlive come after 60 s. On one CPU under load: 7 of 8 runs failed
  before, 0 of 25 after.
- **`plugin_crashes`, `a_plugins_failing_hooks_have_no_effect`** (macOS, on the verification PR): every hook had a 1 s
  timeout, meant for its `sleep 30`; the hooks touching a marker had the same second for the login shell (`$SHELL -lc`,
  which reads the runner's profile) and missed it. Fix: those have 30 s, the hanging one keeps 1 s.
- Already fixed, not recurring: `bottom_pane::tests::tabs_within_a_paste_burst_are_the_pastes_and_send_nothing` (6e36de1),
  `crates/plugin-packages/tests/guests/linked.rs:124` (6d61cc8), and the macOS
  `output::a_poll_behind_the_retained_output_says_exactly_what_it_missed` (BSD `seq` printing `1e+06`, replaced by `awk`
  in 7ada3f2; not a race). The `'checks'` panic at `service/plugins.rs:159/162` in every log is a plugin-fault test's own.

CI on the fixes: [run 43368738460](https://github.com/licyk/hanafubuki-agent/actions/runs/43368738460), attempts 1-3, every job green
with no job re-run (draft PR #23, closed). Before it on the PR: 43356255093 and 43358328046 (the sandbox and D:'s
ACL), 43360809312 (the macOS hooks check), 43362999766 (green, then the open PowerShell silence on attempt 2).
Diagnosis: 43350869210, 43353037350, 43360449754, 43360678802, 43366854195.

## Plan: Codex's shortcuts, image paste and rendering; titles, /effort and /context; three fixes (2026-11-13)

The plan is [docs/plan-shortcuts-rendering-and-context.md](docs/plan-shortcuts-rendering-and-context.md),
read from main `e2981f4`, Codex `57a808a392`, billion-context `4be21b08e`
and Hoshimi `83f7813`.

Findings that changed the request's premises (the plan's §1):

- `/recap`'s 400 is not tool-name normalisation: the structured-output rung
  is chosen per protocol, never per route, with no fallback. Titles fail the
  same way, silently, which is why the first message stays the title.
- The approval reviewer has two rungs (native, text), not three, and keeps
  its strict reading.
- A clipboard image file outside the workspace is readable through
  `prepare_and_stage_images` (only protected roots are refused, and the
  standalone workspace has none): no bytes variant is needed.
- Codex's Scrollback does re-render on resize (it clears and rewrites the
  terminal's scrollback under a row cap): D19 proposes porting it.
- Command output reaches the model, history and presenters as plain text
  from the execution service, so colour needs a display-only styled copy
  there (M7, D21).
- Only four of Hoshimi's window fields are listing-entry fields; Ollama's
  and LM Studio's need other requests (D8). The `max_input_tokens`
  avoidance is Qoder-specific.
- billion-context's panel is already ported (`crates/compression/src/report/panel.rs`);
  no per-request cache usage is recorded anywhere yet.

Each milestone is one commit on main with green CI, strictly in order: B1 →
B2 → B3 → C1 → C2 → E1 → K1 → K2 → K3 → K4 → I1 → M1 → … → M8. Each records,
when done: what landed and any deviation, the validation commands that ran
and those skipped, the CI run's URL (no Windows or macOS claim from a Linux
run), whether goldens changed, what Hanafubuki must change at its next pin,
and any module-size exception.

- [x] Decisions: D3, D4, D6, D8, D10, D14–D22 of the plan's §4 answered
      (D1, D2, D5, D7, D9, D11–D13, D23, D24 came with the request).
      Done (2026-11-13): all confirmed as written, those marked "To
      confirm" included.

**Fixes ([9], [3], [8], [7])**

- [x] B1: structured output on any gateway — a learned per-route rung
      (native → required tool → text, 7-day TTL), the narrow refusal
      classifier, one shared chain for recap, titles, memory and the
      reviewer (two rungs, strict); titles normalised like Codex and
      retried while provisional. Failing test first (the recap's 400).
      Done (2026-11-13):
      - model-api: `ModelStructuredOutputCapability` is `NativeJsonSchema`,
        `RequiredTool`, `Text` (ordered best first; `TextOnly` is gone, the
        default and an unknown port's answer is `Text`); Anthropic answers
        `RequiredTool`. `ModelMetadata.structured_output` (tolerant, only
        `Learned`). `ModelError::{UnsupportedStructuredOutput,
        UnsupportedToolChoice}` from `structured_output_refusal.rs`: 400 or
        422, a name (`text.format`, `response_format`, `json_schema`,
        `structured output`, `structured_output`; `tool_choice` or `tools`)
        and a refusal code or wording. `ModelSession` gains
        `structured_output_capability_for` and `learn_structured_output`
        (no-op defaults). `structured_request.rs`: `request_json_object`
        (the chain under one bound, a refusal of the current rung learns
        the next and retries at once, a forced tool answered with text read
        as text and learning nothing, the text rung's instructions gaining
        one sentence with the schema unless `schema_in_text` is false), and
        `json_object_text` (a text starting with `{` read from there, else
        a fence, else the first balanced `{…}`, else the text) kept apart
        so the reviewer never reads tolerantly.
      - kernel: the registry learns the rung (`learn_structured_output`,
        only ever lowered while live) and answers it
        (`structured_output`); it expires, persists and is forgotten as the
        other learned facts. The registry is an `Arc` in the service, and
        `learning_model_session` (`service/model_facts.rs`) wraps a session
        so it answers and learns from it, holding the registry, not the
        service. `create_private_model_session` takes `&self` and returns
        the wrapped session. The reviewer (`permission.rs`) runs on
        `request_json_object` with no tool (native, then text), reads the
        message exactly with `parse_assessment_text` and keeps its length
        bound; `validate_reviewer_input` went (one user text by
        construction). `turn_user_message` went (no caller left).
      - memory: `generation.rs` uses the chain with all three rungs and
        the tolerant reading; its own native and tool loops are gone.
      - contracts: `AgentNotification::TurnAdmitted.provisional_title`
        became `automatic_title`: the session's title after the admission
        when its source is `Automatic`.
      - agent: `plugins/structured_request.rs` is a thin wrapper (all three
        rungs, tolerant reading) for titles, suggestions and recaps;
        `AgentSessionModel` holds the `AgentService` and asks it for the
        private session. Titles: `normalize_generated_title` ports Codex's
        `parse_thread_title` (quotes, backticks, curly quotes, whitespace,
        trailing `.?!`, cut to 36 instead of refused; control characters
        dropped; NOTICE and `notices-policy.toml` list it and its tests).
        The observer queues every user turn with an `automatic_title`; the
        worker reads the session's first user message from the transcript
        (`AgentConversationReader::first_user_message`, ascending pages:
        a compaction does not lose it), goes on only while the title equals
        its `provisional_agent_title`, generates from the first message and
        commits with the same compare-and-swap; a set of the sessions
        settled in this run (the model titled them, or their title is no
        longer provisional) stops their retries and reads (D6).
      - Deviations: (1) the classifier checks the forced tool's refusal
        last, after the image and reasoning refusals (the plan put both
        before the image one), so a turn's refusal naming its tools never
        loses its image or reasoning fallback; Anthropic's `tool_choice:
        type "tool" and "any" are not supported…`, an "unrelated" case of
        the reasoning suite, is now `UnsupportedToolChoice`. (2) No
        `AgentError` variants: in a turn the new errors fall to
        `AgentError::Model` through `map_model_error`'s default, as the plan
        wants them ordinary. (3) The wrapper is applied where sessions are
        handed out for structured requests (`create_private_model_session`,
        `auto_review.rs`) rather than inside `session/create.rs`'s
        `create_model_session`: it covers a model set after creation too.
        (4) `ModelJsonObjectRequest.schema_in_text` lets the reviewer keep
        its instructions on the text rung (they already ask for exactly the
        object), so its prompt is unchanged. (5) Memory's refusals are tested on a fixture session in
        `hanafubuki-agent-memory`, not the fake server: no end-to-end
        memory pass runs on the fake server. (6) The retry prompts from the
        first user message (it used to be the admitted turn's, the same
        message on the first turn).
      - Tests (failing first): `agent/src/tests/structured_output.rs` on
        the fake server's new `GatewayRefusals` predicate (`FakeServer::
        refusing`, a 400 with `code: invalid_input` and the reported
        message, taking no response): a recap through the forced tool and
        the next recap at the learned rung (one request); a recap through
        text (fenced) with the schema sentence, the next at text, and after
        `forget_learned_model_facts` native again. Run on be56861 with only
        the fake server and these tests copied in, both failed with
        `model remote error status=Some(400) code=Some("invalid_input")
        request_id=None: Structured output (`text.format`) is not supported
        by this stateless gateway.`. `model-api`: the classifier (the
        reported message, illustrative wordings, near misses: an invalid
        schema, a 5xx, no status; structured output over the image
        refusal; reasoning and image refusals naming tools kept), the chain
        on a scripted session (each fall, the starting rung, skipping the
        tool rung, text from a forced tool, failures that end it), and
        `json_object_text`. `kernel`: the registry's rung (lowered only,
        expiry after 7 days, saved and reopened, forgotten); the reviewer
        on a native refusal (text, learned, no tool; a fence or words around
        the object still fail). `memory`: extraction through the tool and
        through text. `agent`: titles normalised as Codex's cases; a failed
        first title retried at the second turn from the first message, then
        no request at the third; a rename stops the retries. Updated: the
        observer contract (`automatic_title` kept at the second turn), the
        titles observer tests, `model_metadata` round trip, the protocol
        capabilities.
      - Not covered by a test: the reviewer's and memory's chains run on
        fixture sessions, so `auto_review.rs`'s wrapped session is not
        exercised through the registry (the registry and the wrapper are,
        through `create_private_model_session` in the recap tests);
        `first_user_message` over more than one transcript page.
      - Goldens: none changed (no golden pins these requests).
      - Docs (both languages): `internals#structured-output-rungs` (new),
        `#titles`, the observers bullet; `architecture#model-metadata` (the
        fact, a **Structured output** paragraph); `providers-models#forget`;
        `sessions` (titles generated, retried, first message as fallback);
        `terminal-interface` (`/recap`); `plugin-rules` (`TurnAdmitted`,
        the `titles` row); `cli-reference` and `data-directory` (what is
        forgotten); the CLI's `providers forget` help.
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features`; `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`. `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` built but could not prerender locally (port 3000
        held by another project's server); CI's Site run covers it.
        CI on Linux, Windows and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43371846978
        (run on fd544aa, which differs from the landed tree only in this
        line; all three passed at once; Site: .../runs/43371846973, success,
        its prerender included).
      - Not known without a live probe: which rung the reported gateway
        takes. The tests prove the chain against the fake's wording; a
        gateway that refuses `tool_choice` in words the classifier misses
        stops the chain at the forced tool.
      - Hanafubuki at its next pin: `ModelStructuredOutputCapability`
        gains `RequiredTool` (`TextOnly` → `Text`); `ModelMetadata` gains
        `structured_output`; `create_private_model_session` takes `&self`;
        `ModelError` gains two variants; `ModelSession` gains two defaulted
        methods; `TurnAdmitted.provisional_title` → `automatic_title`;
        `AgentService::turn_user_message` is gone.
      - Module size: no exception added or raised (`run_memory_pass` grew
        from 252 to 254, under its ceiling of 257).
      - Follow-up (2026-11-13): the chain no longer stops at a rung
        rejected in words the classifier misses (the gap the live-probe
        note above names). A rung above text failing with a 400 or 422
        (`Remote`, or a misread image, reasoning, structured-output or
        tool-choice refusal; never `ContextWindowExceeded`) is tried at the
        next rung (`Attempt::Rejected`), and the rung that then answers is
        learned only on that answer, so a rejection every rung gets teaches
        nothing; the classifier's refusals still learn at once. 401, 403,
        404, 429, a 5xx (retried, then `RequestRetriesExhausted`), an
        overflow, a timeout and a cancellation end the chain as before.
        `classify_remote_model_error` is unchanged, so a turn's sampling is
        too. The reviewer keeps its two rungs and strict reading (it steps
        native → text the same way). An invalid schema (a 400 naming
        `response_format` without refusing it) now tries the next rungs
        too, rather than ending at once.
        - Tests (`model-api`, `structured_request/tests.rs`, failing first:
          4 of the 5 new tests failed on c9bb1b0's chain): an unrecognised
          400 at native reaching the forced tool, learned on its answer; an
          unrecognised 422 at native and 400 at the forced tool reaching
          text, only `Text` learned; a recognised native refusal then an
          unrecognised forced-tool 400 (both learned); rejections at every
          rung, and a rejection followed by a timeout, learning nothing;
          401, 403, 404, 429, 500, a 503 retried out, an overflow, a
          timeout and a status-less error at native and at the forced tool
          ending the chain after one request. `other_failures_end_the_chain`
          lost its invalid-schema case (it now steps down) and gained a
          400 at the text rung.
        - Docs (both languages): `internals#structured-output-rungs` (the
          chain, the refusal's note), `architecture#model-metadata` (what
          the fact is learned from).
        - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
          cargo check --workspace --all-targets --all-features`; `cargo
          clippy --workspace --all-targets --all-features -- -D warnings`;
          `cargo xtask check`; `cargo test --workspace --all-features`; `bun
          install --frozen-lockfile`, `bun run format:check:site`, `bun run
          site:check`, `bun run site:test`; `VITE_BASE_PATH=/hanafubuki-agent/
          bun run site:build` built but could not prerender locally (port
          3000 held by another project's server); CI's Site run covers it.
        - Goldens: none changed. Hanafubuki: no API change. Module size: no
          exception added or raised.
        - CI on Linux, Windows and macOS (99699e5): https://github.com/licyk/hanafubuki-agent/actions/runs/43375640417
          (all three passed at once); Site: .../runs/43375640618, success,
          its prerender included.
- [x] B2: the context window the metadata knows — bound sessions refreshed
      at a listing and at each response commit, `usage` in sync,
      `session_model_metadata` no longer hiding a resolved window, four
      more listing fields. `/status` and the compaction threshold tested
      with a listing after creation and during the first turn.
      Done (2026-11-13):
      - kernel: `list_provider_models` (`service/factory.rs`), after
        `record_listing`, calls `take_listed_context_windows`
        (`service/context_window.rs`): each loaded session whose route is
        on the listed endpoint runs `refresh_context_window` and, when its
        window changed, commits it on its own (revision raised, persisted;
        usage measured against it in the same commit). A failure of one
        session (removed, its commit refused) is logged at debug and the
        others go on. `commit_sampling_response` (`service/sampling/commit.rs`)
        refreshes first too, after its fallible steps, and the response's
        usage takes the window refreshed, so `usage.context_window_tokens`
        always equals the config's. `session_model_metadata`
        (`service/model_facts.rs`) keeps the session's window when it has
        one and otherwise returns the resolved one.
      - model-api: `CONTEXT_WINDOW_FIELDS` (`model_catalog/shapes.rs`)
        gains `inputTokenLimit`, `max_context_length`,
        `model_info.context_window` and `model_spec.availableContextTokens`,
        after the present ones (cited from Hoshimi `83f7813` in the module
        doc).
      - agent: comments only (`model_discovery.rs`, the facade's
        `forget_learned_model_facts`).
      - Deviations: (1) The sweep takes the session's plain lock and skips
        a session with a staged commit (`lock_after_persistence` would wait
        for it, not skip it); that session's next response commit or turn
        takes the window. (2) The sweep raises the revision but not
        `updated_at`: a listing is not activity, and the session list is
        ordered by `updated_at`, so idle sessions are not reordered by a
        background listing (`learn_context_window` keeps bumping it). (3)
        Mistral's documented example of `GET /v1/models`
        (`docs.mistral.ai/openapi.yaml`, `userExample`) shows the cards as
        a bare array; the fixture (`model_listing/fixtures/mistral.json`)
        puts the documented card in the `ModelList` envelope the same
        schema names, since the listing reader needs a `data` array. (4)
        The compaction threshold is asserted as the window compaction reads
        (the session's `config.context_window_tokens`, in memory and
        persisted) before any turn, not by a compaction running: before a
        turn nothing measures against it, and a turn's admission refreshes
        it anyway. (5) The mid-turn test records the listing in the
        registry from inside the model port's answer (the case the sweep
        skips), so the sweep's staged-commit skip itself has no test.
      - Finding: the listing reader reads only a `data` array, so Gemini's
        native listing (`models`) and Alibaba Cloud's (`output.models`)
        never reach their fields; Mistral's and Venice's (`data`) do. The
        fields are harmless there and cover OpenAI-compatible relays that
        forward them.
      - Tests (failing first): `agent/src/tests/model_discovery.rs`, on the
        fake server through the real discovery and `list_provider_models`:
        a listing landing after creation and before any turn (the
        session's revision advances with no model request, `/status`'s
        window is the listed one, and the next run has it with
        `model-metadata.json` deleted); the existing late-listing test now
        takes the window (usage included) as the listing lands, before the
        second turn. Both timed out on 99699e5 (their first form, waiting
        on the window). Kernel
        (`service/context_window/tests/listed.rs`): the sweep before any
        turn (status window, config window, persisted, revision +1,
        `updated_at` kept; a configured session and an unlisted model not
        committed); a session with usage kept in step and recovered; a
        listing landing during the first turn taken at the response commit
        (config, usage and persisted usage equal, recovered without
        quarantine); a session with no route showing the resolved
        (built-in) window. The last two failed with the commit and
        `session_model_metadata` changes reverted. `model-api`: each new
        field and its order (`shapes/tests.rs`), Mistral's documented
        listing (`model_listing`). `selecting_another_providers_model_lists_that_provider`
        re-reads the snapshot before selecting a model: the listing may
        have raised the revision since creation. The discovery suite ran 20
        times in a row without a failure.
      - Goldens: none changed.
      - Docs (both languages): `providers-models#compaction` (when a
        session takes a window: at a listing, a turn's start and after each
        answer), `#forget`; `config-reference` (discovery: the window at
        once); `architecture` (the context window, model metadata, late
        resolution, the known limit); `internals#model-metadata` (the four
        fields).
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features`; `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`; `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` built but could not prerender locally (port
        3000 held by another project's server); CI's Site run covers it.
      - Hanafubuki at its next pin: no API change. Behaviour: a session's
        revision can advance with no event when a listing of its Provider
        is recorded (a host that caches a revision must re-read the
        snapshot before a mutation, as `StaleRevision` asks); a response's
        commit can change the session's window; `session_model_metadata`
        can return a window the session's config does not have (a session
        with none).
      - Module size: no exception added or raised.
      - CI on Linux, Windows and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43377613175
        (run on f488307, which differs from the landed tree only in this
        line; all three passed at once); Site: .../runs/43377613201,
        success, its prerender included.
- [x] B3: thinking stops when the answer starts — the kernel closes an
      open reasoning item at the first non-reasoning item, with a frozen
      duration; the late `output_item.done` persists its payload without a
      second completion. No TUI guard. Failing test first.
      Done (2026-11-13):
      - Cause confirmed: the Responses decoder forwards each
        `output_item.done` as it comes, and the kernel ended a reasoning
        item (its `ReasoningCompleted` and the turn's thinking interval)
        only at that done or at the response's end, so a done sent after
        the answer's text or a tool call kept the entry's and the status
        line's clocks running. In Scrollback the answer's lines also waited
        behind the live thinking entry in the tail.
      - kernel (`service/sampling/stream/`): `PendingReasoning`
        (`reasoning.rs`) gains `closed: Option<u64>`; `close_open_reasoning`
        closes every open item, freezing its duration, emitting
        `ReasoningCompleted` (no tail) and calling `reasoning_item_completed`
        once. `apply_model_event` (`events.rs`) calls it first for every
        `OutputItemAdded` or `OutputItemCompleted` of a non-reasoning item
        and every `AssistantTextDelta` (`ends_reasoning`), so the
        completion precedes that output's events. A done for a closed item
        completes it (summary, content, replay) with the frozen duration
        and emits nothing; a second done is still a protocol error. A
        reasoning delta for a closed item is kept in its text, not shown,
        and logged at debug with its byte count only.
        `complete_unfinished_reasoning` skips closed items; `into_item`
        (`finalize.rs` passes the reasoning bound) completes a closed item
        with no done from its streamed text; an item neither closed nor
        completed still fails as "unfinished reasoning item". The doc of
        `AgentEventKind::ReasoningCompleted` says when it comes.
      - agent (test support only): `FakeServer` holds an event stream at
        each `HOLD` (an SSE comment) until `FakeServer::hold` is notified;
        `late_reasoning_answer_events` is a Responses stream whose
        reasoning done follows the answer's text, held after the summary
        and after the text.
      - Tests (failing first; all six failed with the kernel change
        reverted): `kernel/tests/loop_contract/sampling/late_reasoning.rs`,
        with a model that pauses 300 ms before the late done: the answer
        case (one `ReasoningCompleted`, before `AssistantMessageStarted`
        and the first text, as is the `ReasoningUpdated` Completed; the
        shown text stops at what streamed; the persisted item carries the
        done's summary and replay and the event's duration; one Completed
        update, and `turn_completed_millis` equals its `last_completed_at`
        minus the Thinking update's `active_since` and is under the pause);
        the same for a tool call seen only at its completion (before
        `ToolStarted`) and for reasoning started after the answer item
        (closed by the text delta); a closed item with no done commits its
        streamed text with the event's duration (before: "unfinished
        reasoning item"); a late delta is kept, not shown. TUI
        (`app/tests/thinking.rs`), both presentations, on the fake server:
        a tool call, then the late-done stream held after the answer's
        first line; while held the entry reads `• Thought · ` with no
        `• Thinking` left, the status line's `(Thinking X)` equals the
        rule's `thought for X` and is unchanged 300 ms later, and the
        finished turn has one thought. Before the fix it showed
        `• Thinking 0.7s` with the status line still counting.
      - Deviations: (1) The TUI test asserts strings, not an insta
        snapshot: its durations are measured, so a snapshot would differ
        between runs. (2) Closing happens in one place, `apply_model_event`,
        before the event is applied, rather than in each handler; the
        unannounced-item path re-entering `output_item_added` finds nothing
        open. (3) The plan's "OutputItemAdded for any other item" includes
        `Other` items (a hosted call): they end the thinking too.
      - Goldens: none changed.
      - Docs (both languages): `terminal-interface#thinking` (a stretch
        ends at the answer or a tool call at the latest; what comes after is
        kept, shown on resume; the status line and the rule stop there);
        `internals#thinking-details` (closing, the late done and deltas);
        `architecture#thinking` (the kernel bullet); `json-events` (the
        order guarantee of `reasoning_completed`).
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features`; `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`; `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` built but could not prerender locally (port
        3000 held by another project's server); CI's Site run covers it.
      - Hanafubuki at its next pin: no API change. Behaviour: a reasoning
        item's `ReasoningCompleted` (and the `ReasoningUpdated` that ends
        the turn's thinking) can now come before the provider's done, at
        the first answer text, answer item or tool call; no
        `ReasoningDelta` of that item follows it; text the late done adds
        (a summary that arrives whole) is only in the committed
        `ConversationItem::Reasoning`, so a live view shows less than a
        replay.
      - Module size: no exception added or raised.
      - CI on Linux, Windows and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43382410935
        (run on 1e69139; all three passed at once); Site:
        .../runs/43382410976, success, its prerender included.

**Context and cache ([6])**

- [x] C1: cache usage — `ModelUsage` and `AgentModelUsage` gain cache
      read/write, last and cumulative; prompt total = uncached + read +
      write; `cached: 0` replaced. Context consumed becomes the prompt
      total: **the figure rises for Anthropic and its automatic compaction
      starts earlier** (a change of meaning in `history_committed.usage`).
      Goals count the prompt total (D10).
      Done (2026-11-13):
      - **Change of meaning, Anthropic only.** `lastResponseInputTokens`,
        `cumulativeInputTokens` and `contextConsumedTokens` now count
        `input_tokens + cache_read_input_tokens +
        cache_creation_input_tokens`, where they counted `input_tokens`
        (the uncached part) alone. A session on Anthropic that the cache
        mostly serves therefore shows a higher context share and **compacts
        automatically earlier than it did** (at the threshold of its real
        prompt, as the other protocols always did); its turn input-token
        budget (`maximumCumulativeModelInputTokens`) and a goal's token
        budget fill faster. Responses and Chat Completions are unchanged:
        their input counts already included the cached part. Per the
        json-events stability rule the next release must be a new minor
        version (0.3); the docs say "before 0.3".
      - model-api: `ModelUsage` gains `cache_read_tokens` and
        `cache_write_tokens` (`Option<u64>`, `None` not reported,
        `#[serde(default)]`); `input_tokens` is documented as the prompt
        total. `tolerant_decode.rs`: `token_usage` takes a `UsageFields`
        (input, output, cache-read JSON pointers, the first that decodes
        winning) and `cache_token_count` reads one cache count, `None`
        (logged at debug) when it does not decode, never failing the usage.
        Responses reads `input_tokens_details.cached_tokens`; Chat
        Completions `prompt_tokens_details.cached_tokens`, else DeepSeek's
        `prompt_cache_hit_tokens`; neither reports a write. Anthropic's
        decoder keeps the uncached input and both cache counts per field
        (a later count replaces, an absent or null one keeps, a malformed
        one is `None` without `usage_invalid`) and sums them once in
        `finish`, so a `message_delta` repeating the input side does not
        double count.
      - kernel: `AgentModelUsage` gains `last_response_cache_read_tokens`
        and `last_response_cache_write_tokens` (`Option<u64>`),
        `cumulative_cache_read_tokens`, `cumulative_cache_write_tokens` and
        `cache_reporting_responses` (`u64`), each `#[serde(default,
        deserialize_with = "tolerant")]` (a figure that does not decode is
        its default alone, as persisted data falls back per field).
        `next_model_usage` fills them with the same source-filtered,
        checked accumulation as input and output; `validate_persisted_usage`
        adds "cumulative ≥ last" for read and write. `cache_report` uses the
        last response's cache read as `cached` (a write is not a hit).
        Context consumed needed no code change: it is the reported
        `input_tokens`, now the prompt total. Nothing subtracts an estimate
        from the reported input; the budget ledger, `prefill_input_tokens`
        and memory's `AgentMemoryModelUsage` take the prompt total too.
      - No code change in `agent/src/goals.rs` or
        `tui/src/agent_bridge/status.rs` (in the plan's file list): goals
        count `cumulative_input_tokens` and the status line
        `context_consumed_tokens`, both now the prompt total (D10).
      - Tests: model-api — `tolerant_decode/tests.rs` (the first decodable
        cache pointer wins, null details fall through, a malformed count is
        `None` with the counts kept); Anthropic (`anthropic_messages/tests.rs`:
        the prompt total adds read and write once, with `message_delta`
        repeating or nulling the input side; no cache fields; a malformed
        read with the write kept); Chat Completions (`chat_completions/tests.rs`:
        `cached_tokens`, DeepSeek's field with null details, a non-integer,
        none); Responses (`http_contract/streaming.rs`: cached tokens kept
        in, not added to, the input; a malformed one). Kernel —
        `loop_contract/sampling/usage.rs`: three responses (read and write,
        read only, none) give the per-response, cumulative and counting
        figures, context consumed the prompt total; `service/validation/tests.rs`:
        usage persisted without the cache fields decodes with none and
        passes recovery, a malformed figure falls back alone, a cumulative
        read under the last fails; `service/compression/tools/tests.rs`:
        `cache_report` counts the reads as hits (1500 of 2000, 75%), none
        when unreported. Agent — `tests/sessions/compaction.rs`: an
        Anthropic session over the fake server whose answer reports 1,000
        uncached + 79,000 read + 500 written of a 100,000 window has 80,500
        consumed and compacts before its next turn (failed with the
        decoder's sum removed: 1,500 consumed). The decoder tests were
        written with the change (they assert fields the old type lacked).
      - Deviations: (1) The plan's "Anthropic's threshold test moves
        accordingly": no existing threshold test is per protocol (the
        kernel sees none), so the Anthropic case is the new agent test
        above. (2) The plan says `#[serde(default)]` for the new
        `AgentModelUsage` fields; they also take a per-field tolerant
        reader (AGENTS.md, persisted data). (3) The new
        `compression/tools/tests.rs` is listed as `[[original]]` in
        `architecture/notices-policy.toml` (its directory is derived from
        billion-context; the test is the Agent's own).
      - Goldens: none changed.
      - Docs (both languages): `json-events` (`usage`'s new fields; input
        tokens and `contextConsumedTokens` the prompt total, Anthropic's
        before 0.3); `providers-models#compaction` (what counts as the
        context, Anthropic compacting earlier, the budgets);
        `sessions#goals` (the prompt total); `internals` (the tolerant
        decoding table: the cache counts, Anthropic's sum; `acp_cache`);
        `terminal-interface#compression` (`/compression cache`).
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features` (passed with
        `--no-fail-fast` on the second run; the first stopped at an
        unrelated flake, `plugin-packages` `guests`
        `a_linked_guest_starts_anew_once_its_directory_changes`:
        "signalled after 25.38µs" against its 400 ms minimum, likely the
        first change's signal arriving just after a retry's write under
        load; it passed alone); `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`; `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` not run locally (port 3000 held by another
        project's server, so the prerender cannot run); CI's Site run
        covers it.
      - Hanafubuki at its next pin: `ModelUsage` gains `cache_read_tokens`
        and `cache_write_tokens`, `AgentModelUsage` gains five fields
        (struct literals must name them or use `..`). Behaviour: on
        Anthropic the usage's input figures and context consumed are the
        prompt total, so its context share and compaction rise and come
        earlier.
      - Module size: no exception added or raised.
      - CI on Linux, Windows and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43386494898
        (run on fe37c60, which differs from the landed tree only in this
        line; all three passed at once); Site: .../runs/43386495361,
        success, its prerender included.
- [x] C2: `/context` — the last request's composition captured at build
      time (or an estimate of the next), grouped by instructions, tools,
      context sections, conversation, summaries, images and free space,
      with billion-context's bars, then cached, uncached and the hit rate
      (last and session).
      Done (2026-11-13):
      - compression: `bar(value, total, width)` is public (re-exported at
        the crate root) and takes billion-context's `width` again (the TS
        has it, default 20; the port had fixed 20), so a narrow card can
        shrink it; the status panel passes 20.
      - kernel: `model/context_composition.rs` — `AgentContextComposition`
        (instructions; tools per provider id, `AgentContextTools`; sections
        per kind in prompt order, `AgentContextSection`; user and assistant
        messages, reasoning, tool calls and outputs, continuation context,
        compaction summaries; image count and tokens; `other_tokens`, the
        rest; `total_tokens`), `AgentSessionContext { request, window,
        usage }`, `AgentContextRequest::{Sent(AgentSentRequest), Next,
        Unavailable(reason)}`, `AgentReportedPrompt`. `sampling/composition.rs`
        (new): `compose_request(request, tool_plan)` groups the request's
        serialized parts (each group's bytes summed, its estimate rounded
        once, images by `estimate.rs`'s rule, the rest what no group holds,
        so the groups add up to `estimate_model_request_tokens`);
        `estimate.rs` shares `without_media` and `serialized_len` with it.
        `run_sampling_request_once` composes the request it is about to
        stream (its estimate replaces that call's own) and keeps it in
        `SessionRuntime::sent_request` (`Option<Arc<_>>`, never persisted,
        `None` at every construction); `commit_sampling_response` records
        the response's reported prompt and cache read/write beside it; a
        compaction's install drops it. `AgentService::session_context`
        returns it, or builds the next request as a standalone compaction's
        check does (`prepare_compaction_transport`, now `pub(in
        crate::service)`, `load_history_images`, `step_model_request`), or
        says why not; the window is `/status`'s (the session's, else the
        one `session_model_metadata` resolves). A Provider that sends no
        usage leaves `reported` `None`, which the card shows as "no count
        from the Provider". `AgentToolPlan::model_spec_providers` (new, public)
        pairs `model_specs` with each spec's provider. `kernel::panel`
        re-exports `bar`, `cache_hit_stats`, `format_compact_tokens`,
        `format_hit_rate`, `CacheHitSummary`, `CacheUsageSample`.
      - agent (facade): `session_context(session_id)` (async; starts the
        plugin host, activates the session, reads); `hanafubuki_agent::panel`
        re-exports the kernel's; the new types, `AgentModelUsage` and
        `AgentUsageSource` are re-exported.
      - tui: `/context` (`commands/context.rs`, after `/status`; during a
        task, needs a session) → `AgentCommand::Context` →
        `TurnUpdate::ContextReport` → `history_cell/context.rs`: the heading
        (the estimate's label with the Provider's count beside it, or the
        next request's label and what it leaves out, or why there is none),
        the window, a row per non-empty group (label, bar, percent of the
        window, tokens; sections indented under their total), `Free`, then
        `Cached tokens`, `Uncached tokens` and `Cache hit rate` for the
        last request and the session. At a narrow width labels are cut to
        22 columns first, then the bar narrows, then labels are cut further;
        a bar under 4 cells is dropped. With no window, shares are of the
        request and there is no `Free` row.
      - Tests: kernel `sampling/tests.rs`
        `a_request_is_composed_of_its_groups_and_they_add_up_to_its_estimate`
        (every input kind, two providers' tools, a section kind twice, an
        image: each group equals the estimator on its part, the sum equals
        `estimate_model_request_tokens`). Agent `tests/context.rs`: a real
        session on the fake server — before any request `Next` with no
        sections, tools, no message, the sum exact and no model request;
        while the response is held `Sent` with sections and the message and
        no Provider count; after it the same composition with `{1200, read
        900, write none}` and the session's cumulative read. TUI
        `history_cell/context/tests.rs`: insta snapshots at 100 and 50
        columns with and without cache reporting, before any request with
        no window, plus the response pending, a reported zero read (0%,
        not "not reported"), no estimate possible and a 28-column card with
        no bars; every line within the width. `app/tests/context.rs`: in
        Scrollback and Fullscreen, `/context` before any request, during
        the held turn and after it (`Cache hit rate: last 75.0% · session
        75.0% — 900 of 1.2k prompt tokens from cache (1 request)`).
        `commands/tests/mod.rs`: the pinned order, `during_task` and
        `needs_session` lists. The kernel and TUI tests were written with the
        change (the types and the command did not exist before).
      - Deviations: (1) Captured when the request is streamed
        (`stream/mod.rs`), not in `request.rs` at build: the composition is
        then the request actually sent, after an image or reasoning
        fallback. (2) The plan's sum "equals the request's estimate" holds
        through an explicit rest (`Other request fields`: the model, the
        settings, the JSON framing); with very many small groups their
        rounding could exceed it, and it is then 0 (documented on the
        field). (3) The Provider's count and the last request's cache
        figures are kept with the captured request at its response's
        commit, not read from `runtime.usage` (a later step or a
        compaction overwrites that); the session's come from
        `AgentModelUsage`'s cumulative figures. (4) C1 keeps no per-request
        samples, so `cache_hit_stats` is called on one sample (the last
        request's, or the session's cumulative), and a reported zero read
        is shown as 0.0% where its statistics would skip it; the session's
        rate is over all its prompt tokens (responses that reported no
        cache figure included), with `cache_reporting_responses` as the
        request count. (5) The kernel splits tools per provider id; the TUI
        folds the `mcp.` and `plugin.` providers into `MCP/plugin tools` and
        the rest (the kernel, built-in plugins, memory's source, the native
        tool search) into `Tools`, so the kernel learns no plugin naming.
        (6) Outside a turn the next-request estimate has no sections at all
        (not merely unprepared ones); the card says so. The facade starts
        the plugin host but does not bind the Provider for it
        (`activate_provider_session` would start a discovery). (7) The
        cache lines read label first (`last X · session Y`), so `not
        reported` and `(no request yet)` read; groups with nothing are left
        out, as billion-context's panel leaves them. (8) `/context` is not
        among a side conversation's commands (Codex's list).
      - Goldens: none changed.
      - Docs (both languages): `terminal-interface` (the commands table, a
        command that needs a session, the new `#context` section with an
        example), `embedding` (the facade read, `panel`),
        `architecture#the-context-window-and-the-token-estimate` (the
        capture).
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features
        --no-fail-fast` (two failures: the command popup's pinned list,
        which `/context` changed, fixed; and the known flake
        `plugin-packages` `guests`
        `a_linked_guest_starts_anew_once_its_directory_changes`,
        `linked.rs:33` "signalled after 259.308769ms", which passed alone;
        the whole matrix then ran again on the final tree);
        `bun install --frozen-lockfile`, `bun run
        format:check:site`, `bun run site:check`, `bun run site:test`;
        `VITE_BASE_PATH=/hanafubuki-agent/ bun run site:build` not run
        locally (port 3000 held by another project's server, so the
        prerender cannot run); CI's Site run covers it.
      - Hanafubuki at its next pin: additive only. New facade call
        `session_context` and types `AgentSessionContext`,
        `AgentContextRequest`, `AgentSentRequest`, `AgentReportedPrompt`,
        `AgentContextComposition`, `AgentContextTools`,
        `AgentContextSection`; the facade now also re-exports
        `AgentModelUsage` and `AgentUsageSource`, and `panel`. In
        `hanafubuki-agent-compression`, `bar` is new public API with a
        width; in the kernel, `AgentToolPlan::model_spec_providers`.
      - Module size: no exception added or raised.
      - CI on Linux, Windows and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43395635357
        (run on 2be309d, which differs from the landed tree only in this
        line). Linux and macOS passed at once. Windows failed its first
        attempt on an unrelated timing test, `agent`
        `memory::supervisor::tests::operation_heartbeats_until_completion`
        (`crates\agent\src\memory\supervisor\tests.rs:51`: "assertion
        failed: renewals.load(Ordering::SeqCst) >= 2"; it expects two 10 ms
        heartbeats within a 45 ms operation, which Windows' timer
        granularity can miss), with every `/context` test passing there;
        its job run again passed (attempt 2). Site: .../runs/43395635400,
        success, its prerender included.

**Effort ([5])**

- [x] E1: `/effort` and `SettingKind::ChoiceOrText` — options from the
      reported or configured levels only, else Default and Custom; refused
      mid-turn; the Settings Center's `reasoningEffort` row on the new
      kind, every match site and the configuration reference updated.
      Done (2026-11-13):
      - foundation: `SettingKind::ChoiceOrText { values, maximum_chars,
        nullable }` (`"type": "choiceOrText"`); `read` and `check` take any
        text within the bound (and `null` when nullable), like `Text`: the
        values are what is offered, the owner checks a typed one at its
        write.
      - model-api: `ModelMetadata::known_reasoning() -> KnownReasoning
        { Levels { efforts, default }, Unsupported { learned }, Unknown }`,
        the rule in one place (the O1 plan's D9 accessor, taken now):
        levels only from a `Configured` or `Reported` fact with levels;
        "no reasoning" from `Configured`, `Reported` or `Learned`;
        models.dev, the built-in table and levels not reported are
        `Unknown`. Configured `reasoningEfforts` replace the reported
        levels (the resolved fact is one winner); nothing unites them.
        Re-exported by the kernel and the facade.
      - agent: `reasoning_effort_choices(&KnownReasoning) ->
        Vec<SettingChoice>` (facade export), the levels with the model's
        default labelled `LEVEL (model default)`. The model policy's
        `reasoningEffort` row is `ChoiceOrText` (64 characters, nullable),
        its values filled in `describe_known` from the default model's
        metadata; its summary follows `known_reasoning` (a learned refusal
        says `The Provider refused the reasoning parameter`; models.dev
        levels are no longer listed there). The write still refuses what
        `ModelReasoningEffort::parse` refuses (`none`).
      - tui:
        - `model_facts::effort_options(levels, current)`: the levels, then
          `Default` (`unset: the Provider's default`) and `Custom…`
          (`type an effort`); a current effort off the levels is
          `Custom: VALUE`, marked current. `/effort` and the settings
          center's row both build their lists with it from the same
          `reasoning_effort_choices`.
        - `/effort` (`commands/effort.rs`, after `/model`): bare, the
          picker (`Select Reasoning Effort`, a subtitle for levels, none
          known, no reasoning or a learned refusal); `/effort LEVEL` and
          `/effort default` set at once; an unparsable value says the
          parse error. Custom opens the review picker's `PromptView`,
          which gained `with_check`: a refused value stays in the field
          with its reason in red. `AgentCommand::Effort(EffortCall::{List,
          Set})` → `agent_bridge/effort.rs` (`session_model_metadata`,
          `set_session_reasoning_effort`) → `TurnUpdate::{EffortChoices,
          EffortChanged}`; a change refreshes the status line and says
          `Reasoning effort set to LEVEL` (or `to the Provider's default`).
        - The settings center: a `ChoiceOrText` row (`enter choose ›`)
          opens the choice page with `effort_options`' rows; Custom closes
          it and types the value in the row's own editor
          (`Effect::Type`, `type_selected` on the main page and a form),
          so the write's refusal stays there with why, as a text's does.
          `value_text` shows a value's label. `typed` takes an empty
          nullable `ChoiceOrText` as `null`.
        - Plugin options (`commands/plugins/options.rs`): a
          `ChoiceOrText` is a text field (a guest schema never lowers to
          it).
      - Tests: model-api `tests/model_metadata.rs` (reported and
        configured levels with their default; models.dev, built-in, not
        reported and none are unknown; no reasoning configured, reported
        or learned, not from models.dev); foundation `tests/settings.rs`
        (a typed text, a choice and null taken, the bound refused; the
        serialized shape); tui `model_facts/tests.rs` (levels then Default
        and Custom, current marked; none known → Default and Custom only;
        a current effort off the list is `Custom: max`) — these three were
        written before their code; the foundation one was seen failing
        (`no variant named ChoiceOrText`), the other two first ran after it. `commands/effort/tests.rs`
        (written before the command): dispatch (list, typed, default,
        `none` refused), refused during a turn with the hint and no call,
        picker snapshots with reported levels and with none, models.dev
        levels and a learned refusal offer none, Custom typed with `none`
        kept in the field with its reason then `ultra` sent, the change
        said. Agent `tests/settings_catalog.rs`: the row's kind with no
        level and with configured levels, `max` typed round-trips, `none`
        refused, `null` unsets; `reasoning_effort_choices` marks the
        default. Settings center `tests/interaction.rs`: the page lists
        exactly `effort_options`' rows (`Custom: max (current)`), a level
        saved; with no level Default and Custom only, Default saves
        `null`; Custom typed in the row, the write's refusal kept with
        why, then saved. App, end to end (`app/tests/settings.rs`):
        `/effort` over the standalone Agent (Default and Custom, `max`
        typed, `/effort default`); the policy's row with no level (Default
        and Custom, `high` typed — this existing test failed with the new
        kind until it chose Custom) and with a reported listing (`1. low`,
        `2. high`, Default, Custom). The pinned command lists (table
        order, `during_task`, `needs_session`, `inline_args`, the popup)
        gained `effort`.
      - Deviations: (1) `/effort` is `during_task: true` and refuses in its
        dispatch (as `/review` does), since the generic refusal is printed
        before a command's dispatch runs; it says `'/effort' is disabled
        while a task is in progress. Change the effort after this turn, or
        interrupt it.` (`effort::DURING_TASK`, for K1's stepping too).
        (2) The rule is the model-api accessor `known_reasoning` (the O1
        plan's D9 shape), not code in `model_facts.rs`, because the agent's
        settings row needs it too; `model_facts::effort_options` builds the
        list from it. (3) Alt+,/. are K1's: the acceptance's third surface
        lands there, on `effort_options`. (4) `packages/src/options/lower.rs`
        and `config/src/settings/*` needed no arm (they build kinds, never
        match them). (5) The bridge's calls are `agent_bridge/effort.rs`
        (`AgentCommand::Effort(EffortCall)`) so `run_command` stays under
        the 200-line function limit. (6) `/effort` is not a side
        conversation's command. (7) `/effort`'s own refusal checks a
        running task only, not the queued input the generic guard also
        checks; the kernel refuses a change during a turn regardless
        (`TurnAlreadyRunning`), and `/effort` during a turn still sends
        `EnsureSession` first, which does nothing with a session. (8) The
        choice page of a `ChoiceOrText` uses `effort_options`' wording
        (`the Provider's default`, `type an effort`): the effort is the
        kind's only setting. (9) The settings center's Custom types in the
        row's editor, which starts from the current value even when it is
        a listed level; `/effort`'s Custom prompt starts from a custom
        current value only. (10) The effort stepping (Alt+,/.) is K1's
        (plan §2.7), not E1's.
      - Existing tests that failed with the change and were updated: the
        command table's pinned order and availability lists, the command
        popup's unfiltered list, and the end-to-end
        `the_model_policy_saves_reasoning_context_and_a_custom_budget`
        (the row now opens a choice page).
      - Goldens: none changed.
      - Docs (both languages): `terminal-interface` (the commands table, the commands that need a session,
        the bullets, a new `#effort` section with the picker, the settings
        center's choice-or-text row), `providers-models#reasoning` (the
        options and where they come from, `#interface`'s policy bullet,
        `reasoningEfforts`' row), `config-reference` (`reasoningEffort`'s
        row), `embedding` (`known_reasoning`, `reasoning_effort_choices`,
        `ChoiceOrText`), `architecture#model-metadata` (what may be
        chosen). `architecture/notices-policy.toml`: two `[[original]]`
        entries (`agent_bridge/effort.rs`, `commands/effort/tests.rs`) and
        `model_facts.rs`' reason.
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check` (its module-size check first failed on
        `AgentBridge::run_command` at 213 lines with the effort arms in
        it, hence `agent_bridge/effort.rs`); `cargo test --workspace
        --all-features --no-fail-fast` (all passed, no flake seen); `bun
        install --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`; `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` not run locally (port 3000 held by another
        project's server); CI's Site run covers it.
      - Hanafubuki at its next pin: `SettingKind::ChoiceOrText { values,
        maximumChars, nullable }` in the catalog's JSON (`type:
        "choiceOrText"`): the model policy's `reasoningEffort` row is one
        (its values the known levels; any text up to 64 characters, or
        `null`), so a frontend offers its values, a typed text and unset,
        and a Rust match on `SettingKind` needs the arm. New facade API:
        `KnownReasoning`, `ModelMetadata::known_reasoning`,
        `reasoning_effort_choices`. The row's summary no longer lists
        models.dev levels.
      - Module size: no exception added or raised.
      - CI on Linux, Windows and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43405239912
        (run on fa006c8, which differs from the landed tree only in this
        line; all three passed at once); Site: .../runs/43405239916,
        success, its prerender included.

**Keys ([1])**

- [x] K1: Codex's three-group `?` overlay from the keymap's hints (3/2/1
      columns, `/keymap customize`), and the actions `show_shortcuts`,
      `cycle_mode` (Shift+Tab), `reasoning_down`/`reasoning_up` (Alt+,/.),
      `previous_agent`/`next_agent` (Alt+←/→ on an empty composer), all
      rebindable (`empty_composer_only`, D18).
      Done (2026-11-13):
      - keymap: 66 → 72 actions. `chat.cycle_mode` (`shift-tab`),
        `chat.reasoning_down`/`reasoning_up` (`alt-,`/`alt-.`),
        `composer.show_shortcuts` (`?`), `composer.previous_agent`/
        `next_agent` (`alt-left`/`alt-right`). `ActionSpec` gains
        `empty_composer_only` (the last three): `check_binding`'s text-key
        rule lets such an action take one key that types text, never a
        chord starting with one. `SHARED_KEYS` gains `(previous_agent,
        move_word_left)` and `(next_agent, move_word_right)`. The table
        moved to `keymap/catalogue.rs` (the specs), `actions.rs` keeping
        the vocabulary and the rules on keys: `actions.rs` was at 594 of
        600 code lines. `KeyStroke::matches` no longer folds Shift away
        for unshifted punctuation (US layout: `` `-=[]\;',./ ``) in a
        binding with Ctrl or Alt (a bare `/` still matches `/` with
        Shift, as a layout typing it with Shift needs): before,
        `alt-,` matched `(',', ALT|SHIFT)`, which is `alt-shift-,`
        (`transcript.jump_top`) from a terminal that reports no shifted
        key; `alt-<` still matches `('<', ALT|SHIFT)`. No voice action,
        no `f8` (pinned by a test).
      - The overlay: `shortcut_help.rs` (Codex's) and
        `bottom_pane/footer/shortcut_overlay.rs` (Codex's
        `bottom_pane/shortcut_overlay.rs`); `footer.rs` is now
        `footer/mod.rs`, its old two-column list gone. Compose (`/`, `@`,
        the new line, `esc esc` Edit last message), Session (Tab
        Send/Queue message, Shift+Tab Change mode, Alt+, Less / Alt+. More
        reasoning, ← Agents (empty prompt), Alt+← / Alt+→ Subagents (empty
        prompt), F4 Inspect activity, Ctrl+C Quit/Interrupt), Transcript
        (open first) (Ctrl+T, F3, `pgup / pgdn`, Ctrl+Space, Ctrl+Home /
        Ctrl+End, Shift+Alt+, / Shift+Alt+. when both are bound); then
        `/keymap customize`. Keys from `keymap::hint`: rebound shows the
        new key, unbound or chord-only is left out. `footer_height` takes
        the width; cut to the rows there are, the list keeps its footer
        and says `… more: /keymap`. A second conversation leaves out the
        mode, the effort and Edit last message (`FooterProps::
        second_conversation`). The empty composer's `? for shortcuts`
        names `show_shortcuts`' key. `key_hint` says Shift+Tab (`BackTab`)
        as `shift + tab`.
      - The actions: `?` toggles through the keymap
        (`handle_shortcut_overlay_key`). Alt+←/→
        (`chat_composer/agents_navigation.rs`, on an empty, focused
        composer with no popup or paste burst, in a second conversation
        too) send `WorkspaceEvent::StepAgent`; the app lists the main
        session's subagents off the UI thread through the main
        conversation's bridge (`list_subagents_to_step`) and, once listed
        (`StepAgentListed`, dropped if the main session changed), shows
        the next or previous of the session then its subagents in
        `/subagents`' order, round (`app/workspace/keys.rs`,
        `step_target`): a subagent through `open_child`, the session
        through `return_to_main`; none: `This session has no subagents.`
        Shift+Tab and Alt+,/. (`chatwidget/shortcuts.rs`, while no popup
        or view is open and their command runs in this conversation):
        Shift+Tab sends `SetMode` with the other mode after `/mode`'s
        guard (`refuses_during_task`, factored out of `dispatch_command`)
        and `EnsureSession`; Alt+,/. say `effort::DURING_TASK` during a
        turn, else `EnsureSession` then `EffortCall::Step { raise }`. The
        bridge (`agent_bridge/effort.rs`) reads `session_model_metadata`
        and steps with `model_facts::step_effort`, which walks the `Level`
        rows of `effort_options(reasoning_effort_choices(known))` (E1's one
        list): the current effort, else the model's default, else the
        first level, one up or down; past an end `Reasoning is already at
        the highest level (High).` (`TurnUpdate::EffortUnchanged`), with no
        level `No reasoning levels are known for this model; use /effort
        to type one.` A step sets the effort as `/effort` does (`Reasoning
        effort set to LEVEL`, the status line refreshed). E1's acceptance
        (Alt+,/. offer the same options as `/effort` and the settings row)
        is met with this.
      - Tests: keymap `spec/tests.rs`
        `shift_on_unshifted_punctuation_is_another_key` (`alt-,`, `alt-<`,
        `alt-shift-,` against crossterm's four reports of Alt+, and
        Alt+Shift+,; seen failing first: `alt-,` took `(',', ALT|SHIFT)`;
        `?` and a bare `/` with or without Shift);
        `tests/resolution.rs` (the six actions' defaults and paths, the
        `empty_composer_only` set, Alt+Left both agent and word motion, no
        voice/`f8`; a text key taken by `show_shortcuts`/`next_agent`, a
        text chord and `chat.cycle_mode = "m"` refused); the existing
        catalogue test covers `ACTIONS`' order and no conflict among the
        defaults. `footer/tests.rs`: insta snapshots at 140 (three
        columns), 90 (two) and 50 (one) columns and while a turn runs; the
        cut list at 8 rows; `cycle_mode` rebound to `f6` shows `f6`,
        `reasoning_up` and `previous_agent` unbound are absent; no Voice,
        `f8`, Shell command, Paste image, External editor or Search
        history; a second conversation's list. `agents_navigation/tests.rs`:
        the composer at 50×12 keeps its input row with the list cut;
        Alt+←/→ step on an empty composer, move by word in a draft, step
        in a second conversation, not without focus; `?` toggles and,
        rebound to `h`, `h` does (`?` in a draft types `?`:
        `question_mark_only_toggles_on_first_char`). `model_facts/tests.rs`
        (seen failing first against a stub): a step from a set effort,
        from unset (the default, else the first level) and from one off
        the list, both ends, no level (unknown, unsupported, empty). App,
        end to end (`app/tests/shortcuts.rs`): `?` then Shift+Tab and
        Shift+Tab-as-`Tab+SHIFT` switch the mode in Scrollback and
        Fullscreen; Alt+./, through a reported listing's `low`/`high` and
        both ends; two Alt+. before the first is set take two levels;
        Alt+. with no level, and Alt+, during a turn (`DURING_TASK`,
        nothing changed); Alt+→/→/←/← through a parent and its subagent
        and back, round, and Alt+← in a draft moving by word, in Scrollback
        and Fullscreen; `This session has no subagents.` before a session and in
        one without. The keymap, overlay and app tests were written with
        the code (the actions did not exist before).
      - Deviations: (1) `!` Shell command is left out with Ctrl+V, Ctrl+G
        and Ctrl+R: §2.7 lists `!` as a fixed key, but the acceptance asks
        every listed key to work and `!command` is K2's; K2 adds the row.
        (2) The "remapped `history_search`" test uses `cycle_mode` (and
        unbound `reasoning_up`, `previous_agent`): `history_search` is
        K3's. (3) Shift+Tab, Alt+,/. and Edit last message are left out of
        a second conversation's list (its commands have no `/mode` or
        `/effort`, and Esc returns from it), so the list says only what
        works there. (4) The overlay stays in the footer under the
        composer (as ours was), not above it as Codex's now is; there is
        no close-hint row: `?` or a key the composer takes closes it (the
        session and agent keys, taken before the composer, leave it open).
        (5) Codex's
        macOS fallback (Option+b/f as agent steps on an empty composer) is
        not ported; `alt-left`/`alt-right` only, as the plan's table. (6)
        `pgup / pgdn` stays fixed as §2.7 says (not read from
        `transcript.page_up`). (7) A chord-only binding is left out, as
        `keymap::hint` gives keys only. (8) Effort levels are labelled with
        a capital (`(High)`) as the plan's message. (9) `model_facts.rs`
        now carries the Codex marker (the step is ported from
        `reasoning_shortcuts.rs`); its `[[original]]` entry is gone, its
        tests' and the bridge's reasons say the step. (10) The doc table's
        mangled `alt-shift-,` cell (`` `alt-shift-`, `` ``) is fixed in
        both languages. (11) A step reads the session when it runs, and one
        refused (another step set the effort first: two quick presses, a
        held key) steps again from the new effort, up to 8 times; seen
        failing first with `stale agent session revision`. Two quick
        Shift+Tab presses both ask for the other mode than the one the chat
        last heard of (harmless: the same mode twice).
      - Goldens: none changed.
      - Docs (both languages): `terminal-interface#keys` (three rows, the
        rebindable list gains `?`, Shift+Tab, Alt+,/. and Alt+←/→, so
        only Ctrl-C, Ctrl-D and the lists' keys stay as they are; a new
        `### The shortcuts [#shortcuts]` with the overlay), `#effort` (the
        steps), `#subagents` (the round), `#keymap` (shifted and unshifted
        punctuation, the empty-composer exception, the two shared pairs,
        six table rows).
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check` (its notices check first failed on the new
        `app/tests/shortcuts.rs`, now an `[[original]]`); `cargo test
        --workspace --all-features --no-fail-fast` (all passed, no flake
        seen); `bun install --frozen-lockfile`, `bun run format:site`, `bun
        run format:check:site`, `bun run site:check`, `bun run site:test`;
        `VITE_BASE_PATH=/hanafubuki-agent/ bun run site:build` not run
        locally (port 3000 held by another project's server); CI's Site
        run covers it.
      - Hanafubuki at its next pin: nothing; the change is the terminal
        interface's (no facade, contract or format change).
      - Module size: no exception added or raised. `agent_bridge/mod.rs`
        is now at the 600-line limit (one `TurnUpdate` variant).
      - CI on Linux, Windows and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43425879541
        (run on b652100, which differs from the landed tree in this line
        and the docs fix below; all three passed at once). Site on
        b652100: .../runs/43425879329, failed: its build (MDX) read the
        keymap sentence's bare `Alt+<` as the start of a tag in both
        languages (`681:370: Unexpected character ','`), which the local
        checks (`format:check`, `site:check`, `site:test`) do not parse
        and the local build was not run. Fixed in 1cc7f7f (`Alt+<` as
        code; both pages then compiled with the store's `@mdx-js/mdx`,
        which reproduced the error on b652100's page); its Site run,
        .../runs/43428280881, passed, its build and prerender included.
- [x] K2: `!command` — a no-model turn running the planned `exec_command`
      handler, approved by the person, in the sandbox, recorded as a
      `UserShellCommand` item the model reads (D14, D15). Goldens
      regenerated on purpose.
      Done (2026-11-13):
      - kernel: `ConversationItem::UserShellCommand { call_id, command,
        cwd, exit_code, duration_millis, output, truncated }` (optional
        fields default on read), lowered as a user-role context fragment
        (`user_shell_command`) with Codex's text
        (`model/user_shell_command.rs`: `<user_shell_command>`, command,
        `Exit code: N` (`none` without an exit), `Duration: S.ssss
        seconds`, output). `AgentService::submit_user_shell_command_with_sink`
        admits a turn of `TurnKind::UserShellCommand` (on the turn record,
        through `TurnAdmission::UserShellCommand`): refused while a turn
        runs and in Plan mode; its admission commits `TurnStart` and the
        item saying `The command did not finish.` (it opens the turn as a
        user message does: the boundary validation takes it); the turn
        samples no model (`run_turn` → `run_user_shell_command`,
        `service/tool_execution/user_shell.rs`): it plans the session's
        tools as a step does (`tool_catalog_context`, factored out of
        `prepare_sampling_step`), emits `ToolStarted` for `exec_command`
        with call id `user-shell-<turn>`, runs the planned handler through
        `execute_handler_tool_call` (no `ToolCall`/`ToolOutput` pair, no
        tool-lifecycle hooks), then `ToolExecutionFinished`, the item
        completed in place from the presenter's `AgentToolDisplay::Command`
        (an item snapshot, as a permission review's; an error output gives
        no exit code and the error as output), `ToolCompleted`, and
        `TurnCompleted { final_answer_message_id: None }` (now an
        `Option`; `finish_completed` takes an optional answer and
        `execute_admitted_turn` returns `Option<AgentTurnOutcome>`). The
        turn reads no steering (`InvalidInput`). `ToolCallContext` gains
        `origin: ToolCallOrigin { Model, User }`, set from the turn (also on
        `AgentToolInvocationContext`); `route_approval` authorizes a
        `User` call with no rule verdict at once as the new
        `ApprovalDecisionSource::UserCommand`, in every mode, while
        `Deny`/`RequireUser` verdicts stay as they are. `tool_display`
        answers the call id from the item. Matches: the reviewer's evidence
        (a tool result named `user_shell_command`), hierarchical compression
        (left whole), `/context` (the fragment counts with the calls and
        outputs), history validation (bounded call id; the turn's opener),
        tool pairs.
      - execution and plugin-execution: `AgentExecutionService::stop_confirmed`
        (terminate unless ended, then dispose once the end is confirmed;
        `stop_background_terminals_confirmed` uses it). A `User` call of
        `exec_command` is recorded as `ApprovalOrigin::User`, runs until
        its command exits (`run_to_exit`: waits of 30 s, then one read of
        everything it printed, its wall time the whole run), not the
        model's yield; cancelled (Esc, the tool timeout) it is stopped
        through `stop_confirmed` and reported disposed, never left as a
        background terminal.
      - approval: `ApprovalDecisionSource::UserCommand`, accepted by
        `authorize` (the contract test lists it with the other direct
        sources); `User` still cannot bypass the prompt.
      - facade: `AgentApplicationService::submit_user_shell_command_with_sink
        (session_id, command, sink)` (trims; refuses an empty command): the
        session activated, its turn permission mode and turn extensions (the
        sandbox) as for a message, the working directory
        (`ResolvedPath::native_text` of the execution service's) as
        `workdir`. History store (item type `user_shell_command`, memory
        evidence as a call of that name), export (`!COMMAND`, `· exit N` or
        `· failed`, the output's first lines), `sessions show` (`user ran:
        COMMAND`), `exec`'s final message with no answer id.
      - TUI: `Op::UserShellCommand`; the chat (`chatwidget/shell.rs`,
        Codex's `submit_shell_command`): a sent message starting with `!`
        runs the rest; `!` alone shows Codex's help cell; refused with
        `A turn is running; wait for it or interrupt it first.` while a turn
        runs or a message waits (the draft stays); a message sent while a
        `!` command runs is queued (it reads no steering). The bridge starts
        both kinds in `agent_bridge/start.rs` (`start_turn` moved there:
        `mod.rs` 600 → 596). The footer shows `! shell command · runs in the
        session's sandbox` while the draft starts with `!`
        (`FooterProps::shell_command`). The entry says `You ran` (live: the
        chat knows its `!` turn runs; replayed: `ReplayedItem::UserShellCommand`
        → `new_user_shell_command`), a failed one adds `` `!` commands run
        in the session's sandbox (/permissions) ``; an interrupted one ends
        as it stood with no "tell the model" line. The `?` overlay gains
        `!  Shell command` under Compose (K1 left it out).
      - Tests (written after the code; their first run failed on two bugs
        it then fixed: a turn committing no item could not reach its
        terminal commit, which needs a durable item of the turn, so the
        item moved to the admission; and `authorize` refused `User`):
        agent `tests/user_shell.rs`: no model request, no approval under
        Manual, the events, `[TurnStart, UserShellCommand]` with exit 0, the
        output and the cwd, `tool_display`, and the next request carrying
        the fragment; `exit 3` recorded; refused during a model turn and in
        Plan mode, no steering, Esc terminating `sleep 30` with no
        background terminal left and the item saying it did not finish
        (seen failing with the stop removed: a background terminal was
        left); a test plugin's rule requiring a person asks for `!echo
        needs-a-person`, denied → no exit code (and with the person's
        approval removed, the plain command waited for an approval: seen
        failing). Golden `user_shell_command_request` (new). Kernel: the
        record's text (Codex's two cases and no exit), the item's tolerant
        decoding and lowering. Export test. TUI: `app/tests/shell.rs` end to
        end (the hint, `You ran`, the output, no model request, Up recalls
        `!echo …`, the next request carries it, `/resume` replays it; `!`
        alone; refused during a turn with the draft kept; a message waiting
        for `!sleep 2`; Esc stopping `!sleep 30`); the entry
        (`history_cell/tool/tests/user_shell.rs`: finished and failed
        snapshots, running, interrupted, replayed); agent: a session of a
        finished and a stopped command reopened by a new Agent over its
        storage, both items as committed and the next request carrying
        both; editing the message after a command forks with the command
        kept; footer snapshots (the
        overlay at 140/90/50 columns and during a turn now with `!`, the
        new shell hint) and the K1 test's absent list without "Shell
        command".
      - Deviations: (1) the approval is `ApprovalDecisionSource::UserCommand`,
        not `User`: the approval crate pins that `authorize` refuses `User`
        and `System` (`user_and_system_sources_cannot_bypass_manual_review`),
        and the prompt's answer keeps `User`. (2) The plan's "the
        `execution.git-clone` rule still asks" does not hold for `!`: that
        rule reads a command's program and argv, never a shell script (its
        own test pins `bash -c "git clone …"` as no verdict), and
        `exec_command` always runs a shell; so the test proves that the
        rules still judge a `!` command with a test rule requiring a person
        instead. The model's `exec_command git clone` is not matched either
        (before K2 too). (3) No public `AgentTurnKind`: the kind is the
        kernel's turn record's. (4) The item has a `call_id` the plan did
        not list, which names its live entry and its display. (5) The item
        is committed at admission and completed in place: a turn's terminal
        commit and the restart repair settle working time on a durable item
        of the turn, and a turn boundary needs its opener in the admission;
        an interrupted, failed or restarted command leaves `The command did
        not finish.` (6) Runs to its exit as Codex's, bounded by the tool
        timeout (10 minutes unless configured) rather than Codex's hour;
        a login shell as Codex's (so a profile's messages show in the
        output; the golden redacts them). (7) Refused in Plan mode, whose
        tools only read. (8) The sandbox hint shows on any failed `!`
        command: the chat does not know the sandbox mode. (9) The shell-mode
        hint is a footer row; Codex's red `!` prompt and its bash mode
        (the `!` taken out of the draft) are not ported. (10) A message sent
        during a `!` command is queued, as Codex queues it, since the turn
        reads no steering. (11) `TurnCompleted`'s id is optional (a
        contract change, documented in `json-events`). (12) The user's
        PreToolUse/PostToolUse hooks do not run for a `!` command (no model
        call). (13) A compaction keeps the person's messages, not `!`
        items: an older command is summarized with the rest. (14) A running
        `!` command holds one blocking thread until it exits (a model's call
        holds one up to its 30 s yield); one runs per session at most.
      - Goldens: one new, `user_shell_command_request.json` (the request
        after `!printf user-shell-golden` and a message; the duration and
        the login shell's own output redacted); every other golden
        unchanged.
      - Docs (both languages): `terminal-interface#composer` (shell
        commands), `#shortcuts` (the overlay with `!`), `#tools` (`You
        ran`); `permissions#what-asks` (a typed command is the person's
        approval); `sessions#show` and `#export`; `json-events#turn`
        (`finalAnswerMessageId` `null`); `internals#turn-details` (the
        turn); `embedding#events-and-history` (the facade call);
        `plugin-rules#tools` (`ToolCallContext.origin`).
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check` (its notices check first failed on the three new
        original files, now listed); `cargo test --workspace --all-features
        --no-fail-fast` (4115 passed, none failed; no flake seen); `bun install --frozen-lockfile`, `bun
        run format:site`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`, and each changed page compiled
        with the store's `@mdx-js/mdx` 3.1.1 (all 14 compiled);
        `VITE_BASE_PATH=/hanafubuki-agent/ bun run site:build` not run
        locally (port 3000 held by another project's server); CI's Site run
        covers it.
      - Hanafubuki at its next pin: the new `ConversationItem::UserShellCommand`
        (its transcript and history readers); `AgentEventKind::TurnCompleted`'s
        `final_answer_message_id: Option<AgentMessageId>`;
        `ToolCallContext::origin` (a field to fill where it builds one, e.g.
        tests: `ToolCallOrigin::Model`); `AgentToolInvocationContext::origin`;
        `ApprovalDecisionSource::UserCommand`; optionally
        `submit_user_shell_command_with_sink` for its own `!`. The item is
        committed at the turn's admission and completed in place with no
        event of its own: a reader re-reads it at `ToolCompleted`.
      - Module size: no exception added or raised. `agent_bridge/mod.rs`
        596, `chat_composer/mod.rs` 591, `stage_turn_admission` 196 lines.
      - CI on Linux, Windows and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43439254210
        (run on c930bcc; all three passed at once). Site on c930bcc:
        .../runs/43439254219, passed, its build and prerender included.
- [x] K3: Ctrl+R history search over this run's messages and the history
      store's user messages (D16).
      Done (2026-11-13):
      - history store (kernel trait, file store, router): a new
        `AgentHistoryStore::recent_user_messages(limit, before)` with a
        default that returns none (a host's store compiles unchanged; the
        ephemeral store keeps it); the file store's
        (`store/query/recent_messages.rs`) reads the items of the top-level
        sessions (`parent_session_id IS NULL`: a subagent's messages are
        its parent model's) of the types `user_message`,
        `steering_message` and `user_shell_command` (as `!` then the
        command), newest first (`created_at`, then session and item order),
        each text once, an image-only message and an item that does not
        decode skipped, at most `limit` and `RECENT_USER_MESSAGES_MAX`
        (1,000, new in the kernel and the facade); `before` (Unix
        milliseconds) keeps what was recorded earlier. No index or schema
        change: the type filter cannot use the `user_message` partial
        index, so the read scans the items table and sorts what matches,
        holding the store's database gate, once per composer's first
        Ctrl+R, off the interface's thread (an additive index can come if
        a measurement asks for it). Kernel `AgentService::recent_user_messages`, facade
        `AgentApplicationService::recent_user_messages` (`service/history.rs`).
      - TUI: keymap 72 → 73 actions, `composer.history_search` (`ctrl-r`).
        `ChatComposerHistory` gains the search (Codex's unique-match
        traversal, newest first, staying at an end: `AtBoundary`) over
        this run's messages then the stored ones, read once on a
        composer's first Ctrl+R (`AppEvent::ReadRecentMessages` → the
        bridge's `read_recent_messages`, a blocking task of the Agent's
        runtime whose reply, `RecentMessagesRead`, always comes, empty on
        failure, routed to its conversation like a reply); a step past this
        run's matches before the read lands is `Pending` (`searching`) and
        goes on when it lands. `chat_composer/history_search/` (Codex's
        `history_search.rs` and `history_search_draft.rs`): the footer row
        becomes `reverse-i-search: QUERY` with `searching`, `enter accept ·
        esc cancel` or `no match`, the cursor at the query's end; the
        match previews in the draft's place, its matched text reversed and
        bold (`TextArea::set_highlights`, Codex's case-insensitive ranges);
        the draft (text area with cursor and placeholders, and its pending
        pastes) is put aside whole and put back on Esc, Ctrl+C (through
        `BottomPane::on_ctrl_c`, no quit hint) or raw `\u{3}`; Enter takes
        the match as a draft (cursor at the end, not sent); typing or a
        paste edits the query (`↵`/`⇥` shown for a line break and a tab),
        Backspace/Ctrl+H and Ctrl+U delete; the search's key and Up older,
        Ctrl+S and Down newer. The search counts as a popup
        (`popup_active`), so Esc does not interrupt a running turn nor
        prime editing an earlier message, Up/Down stay with it, and
        `sync_popups` opens no popup over a previewed `/command` or `@file`.
        A message given back meanwhile (`restore_draft`, an interrupted
        turn's) goes into the draft put aside (`edit_draft`, Codex's
        `edit_stored_draft`); `set_composer_text` (editing an earlier
        message) closes the search first. The `?` overlay gains `ctrl + r
        Search history` under Compose, after New line, in a second
        conversation too. `chat_composer/render.rs`: the composer's drawing
        (layout, footer props and mode, `Renderable`) moved out of
        `chat_composer/mod.rs` (591 → 494 code lines), which had no room
        for the search's hooks.
      - Tests: history store `tests/recent_messages.rs` (order across
        sessions, dedupe, the three types, a child session and an
        image-only message left out, `limit`, the cap, `before`, a row that
        does not decode skipped); facade `user_shell.rs`
        `the_persons_recent_messages_include_commands_newest_first`;
        keymap resolution (`composer.history_search` = `ctrl-r`);
        `chat_composer_history/tests.rs` (traversal, dedupe with the stored
        ones, the ends, waiting for the read and going on, a newer step
        before any match waiting too (a review found it ended on `no match`
        though the read then held one; fixed), no match after it, a sent
        message resetting the search);
        `chat_composer/history_search/tests.rs`, Codex's ported (opens
        without a preview and reads once; accept; the ends; this run's then
        stored matches and `!ec` finding `!echo …`; waiting; Esc restoring
        the draft and its cursor and resetting Up; Ctrl+C; no match keeping
        the search open; a pending first char flushed first; a draft with a
        large paste surviving a match and Esc and still expanding; a paste
        into the query; a message given back during the search; no popup
        over a preview; highlights until accepted; the match ranges; a
        remapped key (`f2`) opening and stepping while Ctrl+R does not) and
        insta snapshots of searching, a match and no match at 70 and 30
        columns; footer: the overlay snapshots (140/90/50 and during a turn)
        now with `ctrl + r  Search history`, K1's deferred test (a remapped
        `history_search` shows `f7`, unbound it is absent). App, end to end
        (`app/tests/history_search.rs`, Scrollback and Fullscreen): a run
        over an earlier run's home finds its message (`PARSERS`) and its
        `!echo` and takes it with Enter; Esc and Ctrl+C close the search
        during a turn, which goes on, with no quit hint and the draft back.
        Written with the code; seen failing by mutation: with the read's
        reply emptied, the earlier run's message is not found, and with the
        search not counted as a popup, Esc and Ctrl+C reach the turn.
      - Deviations: (1) the store read includes steering messages and `!`
        commands besides user messages (Codex's history file holds `!cmd`
        as typed, and a steer is what the person typed), and leaves out
        subagent sessions. (2) One action as the plan's table says, not
        Codex's two (`history_search_previous`/`_next`): Ctrl+S, Up, Down,
        Enter, Esc, Backspace, Ctrl+U inside the search are fixed, as a
        popup's keys are. (3) Codex fetches its history entry by entry; here
        one bounded read on a composer's first Ctrl+R (each conversation's
        composer reads its own), so `Unavailable` and Codex's offset
        machinery are not ported: a failed read is an empty one. (4) Codex's
        Vim states, sparkles and frozen fallback preview are not ported; a
        draft change during an idle search shows at once. (5) `before` is
        implemented as the plan names it; the interface passes none.
      - Goldens: none changed.
      - Docs (both languages): `terminal-interface#keys` (Ctrl-R row, the
        rebindable list), `#shortcuts` (the overlay with Search history),
        `#composer` (the search described; `!` commands found by it),
        `#keymap` (the row, the search's fixed keys); `embedding#events-and-history`
        (`recent_user_messages`, the trait's default).
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check` (its notices check first failed on the two new
        original files, now listed); `cargo test --workspace --all-features
        --no-fail-fast` (4149 passed, none failed, on the final tree; no
        flake seen); `bun install --frozen-lockfile`,
        `bun run format:site`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`, and the four changed pages
        compiled with the store's `@mdx-js/mdx` 3.1.1;
        `VITE_BASE_PATH=/hanafubuki-agent/ bun run site:build` not run
        locally (port 3000 held by another project's server); CI's Site run
        covers it.
      - Hanafubuki at its next pin: `AgentHistoryStore::recent_user_messages`
        (a default method: nothing to change unless its store has an index
        to answer from); optionally `AgentApplicationService::recent_user_messages`
        and `RECENT_USER_MESSAGES_MAX` for its own prompt history.
      - Module size: no exception added or raised. `chat_composer/mod.rs`
        494 (the drawing moved to `render.rs`), `agent_bridge/mod.rs` 597.
      - CI on Linux, Windows and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43460170435
        (run on ed2828f; all three passed at once). Site on ed2828f:
        .../runs/43460170282, passed, its build and prerender included.
- [x] K4: Ctrl+G external editor (`VISUAL`/`EDITOR`), a foreground child
      as `/diff`'s `git` (D17).
      Done (2026-11-13):
      - TUI: keymap 73 → 74 actions, `composer.external_editor` (`ctrl-g`),
        before `history_search`. `external_editor.rs` (Codex's): the
        command is `VISUAL`'s, else `EDITOR`'s, read when the interface
        starts (`EditorVariables`, an `App` field); a variable set to blanks
        is unset. Split with `shlex` on Unix and `winsplit` on Windows (the
        command line's rules: a backslash is part of a path), and on Windows
        the program looked up with `which` (`PATHEXT`: `code` → `code.cmd`).
        The draft goes to a `tempfile` `.md` (a path, no open handle;
        removed once read), the editor runs with inherited stdio, on the
        app's thread, and its file is read back on success. Failures
        (`EditorError`), each leaving the draft and shown as an error in the
        transcript: `Set VISUAL or EDITOR to edit the message in an
        editor.`; `The editor in VISUAL is not a command line (a quote is
        not closed).` (Unix); ``The editor `X` was not found; set VISUAL or
        EDITOR to an installed one.``; `The editor could not be started:
        …`; `The draft could not be written to a temporary file for the
        editor: …`; `The editor exited with exit status: N; the draft is
        unchanged.`; `The edited message could not be read (…); the draft is
        unchanged.` (not UTF-8, or gone). The composer
        (`chat_composer/external_editor.rs`): the key, before any popup's,
        flushes a paste burst into the draft and sends
        `AppEvent::ExternalEditor` when the composer has the keys and no
        popup or history search is open, and does nothing otherwise (as
        Codex); a bottom-pane view or an overlay takes the key first. The
        event is dropped when another conversation than its own has the
        focus (`route_event`). The app (`app/external_editor.rs`): the
        editor resolved first (no handoff without one); the seed is the
        draft with its large pastes written out; the footer row reads
        `Save and close external editor to continue.` (Codex's) and a frame
        is drawn; `Tui::release` (`tui/handoff.rs`): the input thread
        stopped and joined (`InputReader::pause`), the alternate screen
        left when entered, the pointer off (unconditionally, whatever the
        shared record says), the keyboard enhancement popped, bracketed
        paste off, the cursor shown, raw mode off; the editor runs;
        `Tui::reclaim`: raw mode, bracketed paste and the enhancement back,
        the input drained of what was typed meanwhile and its thread
        started again (`resume`), the alternate screen entered again with
        the pointer it had (Fullscreen), else the viewport cleared and
        Scrollback's click capture as it was; every step taken even when
        one fails. The edited text is the draft (`\r\n` → `\n`, trailing
        blanks dropped, cursor at the end), and a large paste whose text
        comes back unchanged is its placeholder again, in the draft's
        order; an edited one stays text, a deleted one is gone. The
        `InputReader` now lives in `Tui` (`set_input`, `stop_input`): a
        `Tui` without one (the tests) leaves raw mode and the terminal's
        input alone. The `?` overlay gains `ctrl + g  External editor`
        under Compose, after New line, before Search history (Codex's
        order).
      - Ctrl+G with a popup (`/` commands, `@` files) or the history search
        open does nothing and leaves them open; during a turn it works
        (the turn goes on; its events wait in the app's channel until the
        editor exits); in a side conversation or a subagent shown it edits
        that composer's draft.
      - Deviations: (1) `winsplit` is a third new dependency (Windows only):
        the plan named `shlex` and `which`, but Codex at 57a808a392 splits
        with `winsplit` on Windows, and `shlex` reads `C:\tools\vim.exe` as
        escapes. `shlex` is a Unix-only and `which` a Windows-only
        dependency; all three are MIT or MIT/Apache-2.0, which `about.toml`
        accepts (no change). (2) "Its placeholders kept": Codex writes the
        pastes out and drops them; here the editor gets them written out
        (as Codex) and an unchanged one is collapsed back into its
        placeholder, which keeps the placeholders of pastes the person did
        not touch. Image placeholders are I1's. (3) The terminal is released
        whole, as the plan says (raw mode and the alternate screen left);
        Codex now keeps raw mode and repaints its last frame on the
        alternate screen for an editor in another window. So in Fullscreen
        a GUI editor (`code --wait`) shows the shell's screen until it
        closes, the footer's row showing in Scrollback only; and the
        terminal being in its normal mode, Ctrl+C typed there while such an
        editor waits stops the interface (SIGINT, as for `git commit`),
        documented. (4) Codex's temporary directory that the sandbox cannot
        write (`<codex_home>/editor`) is not ported: the file is in the
        system's temporary directory, so a command the model runs with
        write access there could change the draft while it is open (not
        guarded against; say if it should be). (5) A blank variable counts
        as unset (Codex errors on an empty `VISUAL`), as `config edit`
        does. (6) What was typed into the terminal while the editor had it
        is dropped by reading crossterm's events (Codex flushes the tty
        with `tcflush`/`FlushConsoleInputBuffer`); a resize meanwhile is
        dropped with it, and the full redraw after covers it. (7) A
        non-alternate-screen editor that scrolls Scrollback's screen is not
        accounted for (the inline viewport is cleared and drawn where it
        was).
      - Keymap tests and docs that used `ctrl-g` as a free key moved to
        `ctrl-q` (`keymap/tests/{resolution,chords}.rs`,
        `app/tests/preferences.rs`; the `[tui.keymap]` examples in
        `terminal-interface`, `configuration`, `config-reference`). A
        person's own `ctrl-g` binding beside the composer (the docs' old
        example, `global.open_transcript = "ctrl-g"`) now conflicts with
        `composer.external_editor`: it is refused with a warning and that
        action keeps its default keys; unbinding `composer.external_editor`
        (`[]`) frees Ctrl+G.
      - Tests (written with the code; seen failing by mutation): unit
        `external_editor/tests.rs` (VISUAL over EDITOR, a blank one unset,
        neither; a shell's words with quotes and an unclosed quote (Unix);
        a quoted and an unquoted Windows path (Windows); `code`-style `.cmd`
        found on a PATH and a missing one left as named (Windows); a round
        trip through `FakeEditor`; exit 3, not found, not UTF-8).
        `test_support::FakeEditor`: a `sh` script on Unix, a `.cmd` on
        Windows, that keeps what it was given and saves a prepared result
        with an exit code. `tui/handoff/tests.rs` (vt100): Scrollback and
        Fullscreen with and without the pointer, released (no alternate
        screen, bracketed paste and pointer off, cursor shown) and taken
        back (screen, paste, pointer as before; the viewport drawn whole).
        `chat_composer/external_editor/tests.rs`: asks on a draft and an
        empty composer; nothing with a `/` popup, the history search
        (both stay open, the draft back on cancel) or without the keys; a
        rebound key (`f9`) works and Ctrl+G then does not; the footer row;
        the edit's line endings and trailing blanks; two large pastes
        written out, one back unchanged as its placeholder and one edited
        as text, a deleted one gone; a paste ending the draft with a line
        break, saved back with the editor's own line break after it, its
        placeholder again (a review found the text trimmed before the
        pastes were looked for; seen failing with that order). Footer: the overlay snapshots (140,
        90, 50 columns, during a turn, and the composer's) with `ctrl + g
        External editor`; a remapped (`f9`) and unbound
        `external_editor`. Keymap: the default. App, end to end
        (`app/tests/external_editor.rs`, Scrollback and Fullscreen): a draft
        typed as fast as a paste, Ctrl+G, the editor got it, the edited
        two-line text is the draft, the footer row gone, the screen
        (alternate in Fullscreen only), bracketed paste and Fullscreen's
        pointer back, and Enter sends it to the model; no editor, a missing
        one and one exiting 3 each say why and keep the draft. The pointer
        is checked off Windows only (4ba431a): there crossterm sets it
        through the console's mode, which vt100 never sees. Mutations:
        without `reclaim` three app tests fail (screen and modes); without
        the burst flush the editor gets an empty draft.
      - Not verified without a real terminal: raw mode, the keyboard
        enhancement and the input thread's pause and drain (a `Tui` in
        tests has no input and leaves raw mode alone; vt100 does not read
        the enhancement); a real interactive editor (vim, nano, `code
        --wait`) in either presentation; Windows' console and macOS
        terminals beyond CI's runs.
      - Goldens: none changed.
      - Docs (both languages): `terminal-interface#keys` (Ctrl-G row, the
        rebindable list), `#shortcuts` (the overlay and its note),
        `#composer` (Editing in your editor), `#keymap` (the row; the
        example's and the chord's key now `ctrl-q`); `environment` (the
        variables are Ctrl-G's too); `configuration`, `config-reference`
        (their `ctrl-g` examples).
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check` (its notices check first failed on the two new
        original test files, now listed); `cargo test --workspace
        --all-features --no-fail-fast` (4166 passed, none failed; no flake
        seen; after the review's fix, `cargo fmt`, the workspace's clippy,
        `cargo xtask check` and the TUI's 948 tests again); `cargo clippy -p
        hanafubuki-agent-tui --target x86_64-pc-windows-gnu --all-targets
        --all-features` clean for the TUI (the `cfg(windows)` code and
        tests compile; the MSVC target cannot be cross-built here, Oniguruma
        needs MSVC's C compiler); `bun install --frozen-lockfile`, `bun run
        format:site`, `bun run format:check:site`, `bun run site:check`,
        `bun run site:test`, and the eight changed pages compiled with the
        store's `@mdx-js/mdx` 3.1.1; `VITE_BASE_PATH=/hanafubuki-agent/ bun
        run site:build` not run locally (port 3000 held by another
        project's server); CI's Site run covers it.
      - Hanafubuki at its next pin: nothing (the terminal interface only;
        no facade, contract or format change).
      - Module size: no exception added or raised.
      - CI on 9406703 (the feature):
        https://github.com/licyk/hanafubuki-agent/actions/runs/43470112620,
        Linux and macOS passed, Windows failed on three pointer assertions
        alone (`tui::handoff::tests::{scrollback_…, fullscreen_…}`,
        `app::tests::external_editor::ctrl_g_edits_the_draft_in_the_editor_in_fullscreen`:
        `left: None`, `right: PressRelease`/`AnyMotion`); its `.cmd` editor
        round trips, `which`'s `.cmd` lookup and `winsplit`'s paths passed.
        Site on 9406703: .../runs/43470112493, passed, its build and
        prerender included. CI on 4ba431a (the tests' fix):
        https://github.com/licyk/hanafubuki-agent/actions/runs/43474467831,
        all three passed at once, the 19 editor and handoff tests among
        Windows' passes.

**Images ([2])**

- [ ] I1: image paste — Ctrl+V and empty-paste clipboard images (WSL
      fallback), pasted image paths, `[Image #N]` placeholders with
      renumbering, image-only messages, steers, backtrack restore, the
      capability gate. Windows, macOS and WSL clipboards covered by unit
      tests of the non-clipboard logic only.

**Rendering ([4]; every Codex format in the plan's §2.12 table)**

- [ ] M1: the Markdown renderer — tables (key/value fallback, ```md
      unwrap, literal tables), task lists, list spacing, web and local
      links, `tui.rendering.*`; pulldown-cmark 0.13 adaptations.
- [ ] M2: source-backed answer cells and reflow on resize, in Fullscreen
      and in Scrollback (D19).
- [ ] M3: streaming — incremental blocks, stable/tail regions, chunking,
      table holdback, open-fence highlighting, prose previews.
- [ ] M4: OSC 8 hyperlinks and URL-aware wrapping.
- [ ] M5: math (D20).
- [ ] M6: mermaid, as `crates/tui/src/mermaid/` (D22).
- [ ] M7: SGR colour in command output, live and on replay (D21).
- [ ] M8: JSON pretty-printing, MCP content summaries, the diff
      improvements.

## Plan: a sub-agent's model, Provider and reasoning effort, chosen at each spawn (2026-11-13)

The plan is [docs/plan-subagent-model-overrides.md](docs/plan-subagent-model-overrides.md),
read from main `be56861` and Codex `57a808a392`.

Findings that changed the request's premises (the plan's §1):

- `select_role_model` skips the model selection when the model id equals
  the parent's, so a `provider` with the parent's model id would keep the
  parent's Provider: the pair is compared instead.
- The child is created and renamed before its model is selected, so a
  failed selection leaves an idle child holding the task name: everything
  is checked before `create_child`.
- A model change applies the **global** model policy (effort, input
  capabilities, compaction, turn budget); there is no per-Provider policy.
  The window and the image capability are refreshed inside
  `set_session_model_configuration`, so the spawn is covered whenever it
  selects a model.
- The kernel keeps each Provider's recorded listing but exposes no read of
  its ids: a new `listed_models`.
- `exec --json` reports no sub-agent; nothing changes there.
- Codex lets a role override the requested model, and gives an explicit
  model its own default effort; ours keeps explicit > role > parent and the
  parent's effort (D2, flagged).

Each milestone is one commit on main with green CI, strictly in order,
after M8 above: O1 → O2 → O3. Each records, when done: what landed and any
deviation, the validation commands that ran and those skipped, the CI
run's URL (no Windows or macOS claim from a Linux run), whether goldens
changed, what Hanafubuki must change at its next pin, and any module-size
exception.

- [ ] Decisions: D2 (flagged), D7–D19 of the plan's §4 answered, and the
      open questions Q1–Q6 of §5 (D1, D3–D6 came with the request).
- [ ] O1: `spawn_agent`'s `model`, `reasoning_effort` and `provider`, with
      or without `agent_type`. Precedence explicit > role > parent per
      field, the pair compared. Field-named checks before the child
      exists: an enabled Provider (id or name); a model against the
      recorded listing ∪ `models."<id>"` (passed through with no listing);
      an effort against the known levels (`ModelMetadata::known_reasoning`,
      E1's rule, shared). One commit for pair and effort. Spawn's result
      names them. The 13 goldens with the spec regenerated on purpose.
- [ ] O2: the guidance lists the enabled Providers with their known models
      and levels, bounded (8 × 8), stable for the prompt cache, built in
      `prepare`. The 11 goldens with the guidance regenerated on purpose.
- [ ] O3: a child's Provider, model and effort in `list_agents`,
      `AgentSubagentStatus` and `/subagents` (status cell and picker).

## Plan: wake events, nested sub-agents and the background task manager (2026-11-13)

The plan is [docs/plan-agent-wake-and-background-tasks.md](docs/plan-agent-wake-and-background-tasks.md),
read from main `147fdaa`. What the request calls the "Hook" mechanism is
called **wake** everywhere: user-configured hooks already exist.

Findings that changed the request's premises (the plan's §1 and §1.1):

- A parent hears of a child only through `wait_agent`: an in-memory
  `unreported` flag that a restart loses. The kernel queue replaces it, so
  even with the option off, final statuses become durable.
- Session events of an idle session reach the sink of its last managed
  turn, not nowhere. Even so, a root's woken turn is submitted by its host
  (`AgentBackgroundEvent::Woken`), never started on a retained sink.
- Child turns have no sink: the terminal interface finds a child's
  approval by reading journals every 400 ms. Two levels and `exec` need a
  forwarding sink (`Child` events).
- "Every background terminal wakes its owner" makes a development server
  wake it at the command timeout, and holds `exec` open that long. The
  timeout becomes a setting (Q4, R4b), 10 minutes by default.
- Counting running turns against the cap would let woken turns exceed it:
  the cap counts live agents across the tree instead.
- `stage_turn_admission` is at 196 of 200 lines and `agent_bridge/mod.rs`
  at 596 of 600: both are split first. `activity.rs` is at its ceiling and
  takes nothing.

Each milestone is one commit on main with green CI, strictly in order,
after O3 (and after K4, I1 and M7, whose files R5 and R8 share; W1, which
R4b follows in `crates/execution/src/shell.rs`, is on main): R1 → R2 → R3
→ R4 → R4b → R5 → R6 → R7 → R8. R4b is its own small milestone before R5:
the setting needs no wake, and R5's fragment states its value. Each
records, when done: what landed and any deviation, the validation commands
that ran and those skipped, the CI run's URL (no Windows or macOS claim
from a Linux run), whether goldens changed, what Hanafubuki must change at
its next pin, and any module-size exception.

- [x] Decisions: D1–D20 of the plan's §2, and the questions Q1–Q12 of
      §5, decided by the user on 2026-11-13: each as recommended, except
      Q4, where the command timeout becomes the setting
      `pluginOptions.execution.commandTimeoutMinutes` (D12, §3.13, R4b).
- [ ] R1: the kernel's durable wake queue (`AgentWakeEvent`, keyed per
      sub-agent and terminal, bounded), `ConversationItem::WakeEvents`
      rendered as `<background_events>`, drained at every admission and
      every step, `TurnAdmission::Wake` and `submit_wake_turn_with_sink`.
      Inert: only test support queues events. New golden
      `wake_events_request.json`.
- [ ] R2: the main agent suspends and wakes on its sub-agents. Adds
      `wakeOnBackgroundTasks` (default on), the dispatcher, the person's
      pause and the 100-turn runaway bound. `wait_agent` reads the queue
      (`unreported` goes). Adds `send_message` to `parent`, the guidance
      ("end your turn; you are resumed"), Goal's pass while background
      work runs, the host's `Woken`, the TUI's submit and waiting line, and
      `exec` waiting for the tree. The 12 guidance and 14 spec goldens are
      regenerated.
- [ ] R3: the background feed: `background_tasks(root)`, `Changed`, and
      `Child` events from a forwarding sink on child turns. The TUI's
      400 ms watch goes, and `exec` answers children's approvals.
- [ ] R4: two levels of sub-agents (depth from lineage; depth 2, side
      conversations and reviews refused) and `maximumRunningAgents`
      default 8, counting live agents across the tree; woken turns are
      never refused. A child's completion is held until its subtree is
      quiet. Adds the `Waiting` and `Closed` states, and splits
      `subagents/service.rs` by responsibility. Goldens regenerated.
- [ ] R4b: the command timeout becomes a setting,
      `pluginOptions.execution.commandTimeoutMinutes` (1–1440, default
      10). The `execution` row takes options and a change in place,
      without a restart; every `exec_command` execution, a `!command`'s
      included, runs with the value at its call
      (`ExecCommandRequest.timeout_ms`, in its plan and its approval's
      canonical bytes); running commands keep theirs. No goldens; the
      `config-reference` row is hand-written in both languages.
- [ ] R5: background terminals wake their owner. The kernel queues the
      end on a terminal status of a call not in flight and withdraws it on
      disposal; `!command` and `Cancelled` are excluded. Adds the execution
      plugin's `background_terminals` fragment, which states the
      configured timeout, and `exec` waiting for terminals. The 14
      `exec_command` goldens are regenerated.
- [ ] R6: `close_subagent` (cascade), `close_background_terminal`, and
      `/stop` as the person's close. Each wakes the owner; archive, delete
      and side close do not.
- [ ] R7: restart. Work a stop of the Agent ended is reported as `Lost`
      to its owner (transitively), at a clean shutdown before turns are
      cancelled and at start for a crash. It is idempotent by key, and
      nothing is woken while starting.
- [ ] R8: the terminal interface. `/tasks` (the manager: a tree of agents,
      the terminals, inspect, close, resume), the `background-tasks`
      status item in the default status line, the waiting and paused
      lines, `open_tasks` on `leader t` with its `?` row, and the `/ps`
      and `/subagents` hints.

## Plan: a registry-driven settings center, the command center, and the conversation timeline (2026-11-12)

**Complete (2026-11-13):** every item below is done; T7 was the last.

The plan is [docs/plan-settings-and-timeline.md](docs/plan-settings-and-timeline.md),
read from Codex `57a808a392`, KohakuTerrarium `c1d17b45` and main `6d61cc8`.
It supersedes the hand-written forms of the configuration-center plan (below)
and the thinking presentation of the 2026-11-09 plan (one write into
scrollback, a sticky Ctrl-O, `Working` alone in the status line); their other
results stay.

Findings that changed the request's premises (the plan's §1):

- Codex's `/config` is a hard-coded list of 19 `[tui]` rows without MCP or
  a mouse: its presentation is ported, its data model is not.
- KohakuTerrarium's schemas cover module options only, and neither
  reference has a collection editor (Providers, MCP): ours is designed.
- Codex has no `Thought · Ns` entry on any ref: the thinking timeline is new.
- Codex's `exec_command` decodes with lossy UTF-8 only, and sends Windows
  PowerShell exactly our prefix; porting it would not fix garbling. The
  reported case is Windows PowerShell 5.1 (D10): mixed encodings in one
  stream, 5.1's own decoding and `$OutputEncoding`, progress bars under
  ConPTY. Ours is fixed in place, after reproducing it on `windows-latest`.
- Scrollback must work as Fullscreen does (D13): recent entries stay in a
  repainted tail where they change and expand in place (the plan's §4.11).
- Our TUI drops `ExecutionProgress`, so no tool output streams today, and
  `read_file` and the other structured tools show only their header.

Each milestone records, when done: what landed, the validation commands that
ran, whether goldens changed, what Hanafubuki must change at its next pin,
and any module-size exception. Order: S1 → … → S7; A1 → A2 after S1; T1 →
T1b → T2, T3 → T4 → T5 → T6, T7 alongside T1, T1b and T5; E1 → E2 → E3,
independent.

- [x] Decisions: D1–D17 of the plan's §8 answered. Done (2026-11-12): all
      accepted as recommended. D10: Windows PowerShell 5.1; D7 and D13:
      Scrollback must work as Fullscreen does.

**Settings ([1], [2])**

- [x] S1: one vocabulary, `foundation::settings` (`SettingSpec`,
      `SettingKind`, choices with labels, aliases and disabled reasons,
      `apply`, `advanced`, `visible_if`, `commit_group`);
      `PluginOptionSchema` and the guest `configSchema` lowering on it. Not
      model-visible.
      Done (2026-11-12):
      - `hanafubuki-agent-foundation::settings`: `SettingSpec` (key, label,
        summary, help, kind, default, apply, advanced, `visible_if`,
        `commit_group`, keywords; `SettingSpec::new(key, label, kind)` with
        the rest by struct update), `SettingKind` (Boolean, Integer and
        Number with `Option` bounds, Text with `maximum_chars`, `secret`,
        `nullable`, Path with `must_be_absolute`, Choice of
        `SettingChoice { value, label, aliases, disabled }`, TextList),
        `SettingValue` (with `Null`, a nullable text unset),
        `SettingApply { Live, NextSession, NextLaunch }`,
        `SettingCondition { key, equals }`. Two reads: `read`, tolerant (an
        alias is its choice, a disabled choice is still read, a number out
        of range is `SettingProblem::OutOfRange` with its clamped value),
        and `check`, strict (refuses out of range, an alias, naming its
        value, and a disabled choice with its reason). `Serialize` only,
        camelCase, the kind tagged `type` with camelCase fields.
      - `hanafubuki-agent-packages::options` keeps `PluginOptionSchema
        { options: Vec<SettingSpec> }`, `ResolvedPluginOption(s)` (values
        are `SettingValue`s), `resolve` and `check_settings` on
        `SettingSpec::read`, `MAX_PLUGIN_OPTIONS` and the `configSchema`
        lowering: the same keywords, labelled by key, each choice by its
        value; an absent bound or length is `None`, no longer an
        `i64::MIN`/`f64::MAX`/`usize::MAX` sentinel. A guest's settings set
        to a former name reach it as the current value.
      - `subagents`' four options gain labels; the TUI's options form shows
        the same text as before (Choice rows show labels, select by value;
        Path reads as a text).
      - Deviations from §4.1: the key is a `String`, not a `SettingKey`
        newtype; bounds are `minimum`/`maximum`, as before; the schema and
        resolved types stay in packages (they are plugin-specific) and only
        the vocabulary moved, so `PluginOptionSchema`'s path is unchanged
        and its `options` field kept; `title`, labelled choices, aliases and
        disabled reasons are not read from a `configSchema` (no standard
        keyword for the last two; `title` is S6's, with package settings in
        the center). `agent_settings_fields` stays until S3, as planned.
        For S2: §4.3's tolerant decoder must call `SettingSpec::read`, not
        `check`, or a stored alias or disabled choice is repaired away.
      - Tests: `foundation/tests/settings.rs` (clamped read and strict
        refusal, one-sided and absent bounds, an alias read but never
        written, a disabled choice read but refused with its reason, `null`
        only for a nullable text, an absolute path, the serialized shape);
        `packages/tests/packages/options.rs` (an alias resolved and
        delivered to the guest as its value, the lowering's `None` bound and
        its choices).
      - Docs (both languages): `plugin-rules#host-configuration-and-status` (Descriptors and
        options: a list of `SettingSpec`s, the kinds, aliases and disabled
        reasons), `architecture` (the crate table: foundation holds the
        settings vocabulary), `contributing#twins-shared-with-hanafubuki`
        (`settings` is Agent only).
      - No golden changed (nothing model-visible); no TUI snapshot changed.
      - Hanafubuki at its next pin: `PluginOption` → `SettingSpec` (new
        `label`, `help`, `apply`, `advanced`, `visible_if`, `commit_group`,
        `keywords`), `PluginOptionKind` → `SettingKind` (`Option` bounds and
        lengths as `u32`, `Text.nullable`, new `Path`, `Choice.values` of
        `SettingChoice`), `PluginOptionValue` → `SettingValue` (new `Null`),
        all re-exported from `hanafubuki_agent` (also `SettingChoice`,
        `SettingApply`, `SettingCondition`, `SettingProblem`);
        `PluginOptionSchema.options` and `ResolvedPluginOption.value` take
        the new types; `PluginOption::check` is `SettingSpec::check`. The
        JSON of `AgentPluginStatus.options` and `AgentPackageGuest.options`
        changes with them (kind fields camelCase, choices as objects, new
        fields). Hanafubuki-dev `685e18b9` names none of these types and
        its TypeScript `AgentPluginStatus` has no `options`, so nothing
        breaks there.
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features`; `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`, `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` (all 35 pages prerendered). CI on Linux, Windows
        and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43212925497
        (run on 6ef1205, which differs from the landed commit only in this
        line; Site: .../runs/43212925447).
      - Module size: no exception added or raised.
- [x] S2: a section beside each config domain, the decoders reading keys and
      bounds from it; per-key provenance in `layers::resolve`;
      `AgentConfig::write_values` (set, reset, commit groups, outcomes) under
      the center and `config get/set`; the leaf-to-spec coverage test;
      `cargo xtask check-config-reference`.
      Done (2026-11-12):
      - Specs beside their structs: `settings/mod.rs` (`general`:
        `permissionMode`, `disabledPlugins`), the new `settings/sandbox.rs`
        (`sandboxMode`, `sandboxWorkspaceWrite.*`, `windows.sandbox`, with
        `AgentSandboxWorkspaceWrite` and `AgentWindowsSettings`),
        `settings/memory.rs` (with `AgentMemorySettings`), `settings/web.rs`
        (with `AgentWebSearchProvider`; provider and URL in the commit group
        `webSearch`), `settings/compaction.rs` (`visible_if` the strategy),
        `settings/model_metadata.rs`, `standalone.rs` (`network`:
        `network.proxy`, `logLevel`) and `tui.rs` (the eight `[tui]`
        scalars). `hanafubuki-agent-config::sections` lists them
        (`config_sections()`, `SettingsSection { id, title, settings }`),
        each default filled from the serialized struct `Default`; no spec
        states one. `apply`: `logLevel`, `tui.fullscreenTranscript` and
        `tui.alternateScreen` are next launch, the rest live.
      - The decoders read by the specs (`sections::SettingsRead`): each key
        under a struct is read with `SettingSpec::read` (tolerant, as S1
        asked), overlaid on the serialized layer below, deserialized and
        checked by the section's rule on its own, so one bad key falls back
        alone, as `read_member` did; a table on the way that is not one is
        one repair. `read_member` calls and the per-field validators of
        these domains are gone; the patch path checks the same specs
        (`check_specs`). Every existing decode test passed with its
        expectations unchanged (only `decode_settings`' report argument
        changed in `settings/tests.rs`).
      - Provenance: `layers::resolve` records per key the highest layer
        whose value the settings use (`SettingSource::{Default, User,
        Profile { name }, Project { path }, Override}`), so an unusable
        value above does not claim a key; `AgentConfig::setting_sources()`.
      - `AgentConfig::write_values(&SettingsWrite)`:
        `SettingsWrite { changes: [{ key, op: {"set": value} | "reset" }] }`
        (strict, `ipc-request` in the policy with `SettingChange`); each
        value `SettingSpec::check`ed, then `check_changes` (the old
        `check_setting` generalized to a batch, so a commit group is one
        check) refuses any new repair, then one atomic in-place write
        (comments and unknown keys kept; a reset removes the key); returns
        `SettingsWriteOutcome { written, shadowed, apply }`. `config set` of
        a key with a spec goes through it; a profile's key is checked
        against its spec first; other keys (`plugins.*`, a table) keep the
        re-decode path. `config get` is unchanged.
      - `cargo xtask check-config-reference` (in `cargo xtask check`): every
        `SettingSpec::new("key", …)` of `crates/config/src` (the key must be
        a string literal; macro bodies are read) has a row in the
        `## config.toml` tables of `config-reference` in both languages.
      - Deviations: the decoder overlays the layer below rather than
        `#[serde(default)]` (layers decode over the one below, not the
        defaults), and an out-of-range value still falls back to the layer
        below rather than to its clamp; the hand-written fallbacks stay
        (`sandboxMode` → read-only, the SearXNG provider and URL pair and
        URL normalization, `disabledPlugins` per name, absolute
        `writableRoots`, non-blank model names of at most 256 bytes, the
        summary-length pair); `SettingsSection` has only id, title and
        settings (S3 adds group, order, file and values); the collections
        `plugins`, `marketplaces`, `pluginOptions`, `tui.keymap` and
        `tui.presets` have no spec and are a named exclusion of the
        coverage test; `Set` carries raw JSON checked by the spec
        (`SettingValue` has no `Deserialize`); the patches
        (`AgentSettingsPatch`, the standalone and `[tui]` patches) are not
        yet re-expressed on `write_values` (S3–S5 remove or move their
        callers); keys are string literals because xtask may depend on no
        workspace crate.
      - Tests: `config/tests/settings` (every leaf of the defaults has
        exactly one spec and every spec a leaf; every default is the
        struct's and passes its spec; every choice value written and read
        back; set and reset; refusals leave the file unchanged; a commit
        group; provenance per layer and the shadowed outcome; a host's
        configuration; `config set` and `write_values` write the same file;
        `SettingsWrite` is strict); xtask's spec-key and row parsers.
      - Docs (both languages): `configuration#write` (what `config set`
        checks), `cli-reference#config`, `building-testing` (the new check);
        AGENTS.md's check list. The rest is S7's.
      - No golden changed (nothing model-visible); no TUI snapshot changed.
      - Hanafubuki at its next pin: additive only. New in `hanafubuki_agent`:
        `config_sections`, `SettingsSection`, `SettingSource`,
        `SettingsWrite`, `SettingChange`, `SettingWriteOp`,
        `SettingsWriteOutcome`, `AgentConfig::write_values` and
        `setting_sources`; `AgentTuiPreferences` and `AgentNetworkSettings`
        gain `Deserialize`. `agent_settings_write` refuses out-of-bounds
        memory and compaction values with the spec's message
        (`` `memory.maxSessionsPerPass` is between 1 and 64 ``) instead of
        the former generic one.
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features`; `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`, `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` (all 35 pages prerendered). CI on Linux, Windows
        and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43220395547
        (run on 4fdebb5, which differs from the landed commit only in this
        line; Site: .../runs/43220395691).
      - Module size: no exception added or raised.
      - For S3: build the catalog's sections from `config_sections()`, their
        values from `resolved_document()` and `setting_sources()`; the
        facade's `write_setting_values` is `write_values` then
        `apply_settings_to_plugins`; `disabledPlugins` has a spec (general,
        advanced) though the plugins collection also writes it; the model
        policy is still decoded by hand (`read_member` remains for it);
        `write_settings`, `set_plugin_disabled` and a repair still write the
        whole settings object, after which every key's source is `User`
        until reset; `check-config-reference` reads a macro body only when
        it is comma-separated expressions, so keep specs directly in
        `vec![…]`.
- [x] S3: the facade's `settings_catalog`, `write_setting_values`,
      `run_settings_action`; collections for Providers, the model policy,
      MCP, plugins, marketplaces, hooks, roles, profiles and project trust;
      `agent_settings_fields` / `web_search_fields` removed (Hanafubuki:
      its settings page reads the `web` section).
      Done (2026-11-12):
      - Config: `SettingsSection` gains `group` (`SettingsGroup`: General,
        Model, Tools, Memory, Context, Network, Interface, Plugins,
        Advanced), `order` and `values` (`SettingState { key, value, source,
        issue }`); `AgentConfig::settings_sections()` fills them from
        `resolved_document()` and `setting_sources()`, the issue being the
        first layer warning about the key (`Resolved.issues`, recorded in
        `layers::report` from the repair's key); a host's configuration
        leaves out `network` and `tui`. `write_values_with_options(write,
        schemas)` also writes `pluginOptions.<row>.<key>` against the row's
        schema in the same atomic write (`write_values` is it with none).
        Plugin option keys now have provenance (`setting_sources()` lists
        `pluginOptions.<row>.<key>` with the layer that set it).
        `set_plugin_disabled` writes `disabledPlugins` alone, so a toggle no
        longer makes every key's source `User`. New readers/writers for the
        facade: `user_document()`, `is_layered()`, `set_user_json(key,
        value)` (`set_user_value` without the TOML parse).
        `agent_settings_fields` and `settings/schema.rs` are gone.
      - Facade (`crates/agent/src/service/settings/`, one module per
        collection): `settings_catalog()` → `SettingsCatalog { sections,
        collections }`: the config's sections, then a Plugins-group section
        per row whose descriptor has options (`pluginOptions.<row>`, full
        keys, values from the row's resolved options); collections
        `providers`, `modelPolicy`, `mcp`, `plugins`, `marketplaces`,
        `hooks`, `roles`, and standalone `profiles`, `projects`
        (`SettingsCollection { id, title, group, record: Vec<SettingSpec>,
        records: Vec<SettingsRecord>, actions: Vec<SettingsAction>,
        revision, issues }`; `SettingsRecord { id, label, summary, state,
        issue, values, secrets_set }`; `SettingsAction { id, label, scope:
        Collection | Record, confirm }`). An owner that cannot be read
        (hooks while the row is off) gives an empty collection with the
        reason. `write_setting_values(SettingsWrite)` =
        `write_values_with_options` with the host's option schemas, then
        `apply_settings_to_plugins` and a memory scan signal.
        `run_settings_action(SettingsActionRequest { collection, record,
        action, expectedRevision, values })` (strict, `ipc-request` in the
        policy; `Debug` shows the value keys only) → `SettingsActionOutcome
        { record, message }`, dispatched to the owner's calls: Providers
        `add`/`save`/`delete`/`set_default`/`list_models`
        (`write_provider` etc.); model policy `save` (`write_model_policy`,
        one record `policy` with `defaultProviderId`, whose choices are the
        Providers, a disabled one with why); MCP `add` (record = new id)
        /`save`/`delete`/`connect`/`disconnect`/`refresh`/`logout`; plugins
        `save` (`enabled` → `set_plugin_enabled`)/`retry`; marketplaces
        `add`/`upgrade`/`delete`; hooks `save` (`enabled`)/`trust`; profiles
        `add`/`save` (`set_user_json` per key); projects `add`/`trust`/
        `untrust`; roles none. A save merges its values over the stored
        record by the specs (`record.rs`), so what no spec names is kept; a
        secret (`apiToken`) is never returned (`secrets_set` lists it) and a
        save without it keeps it; `null` unsets an optional field.
      - `AgentSettingsView.web_search_fields` removed. The TUI never read it:
        its configuration center is unchanged (S4 replaces it).
      - Deviations from the plan: the calls are on `AgentApplicationService`
        (reached through `AgentHandle`, as Hanafubuki and the TUI reach the
        service), not `AgentRuntime`. No contracts registry for settings:
        plugin rows contribute through their descriptors, as §4.2 says
        (no global registry), so a stopped or failed row's options stay
        editable. `SettingsSection` has no `file`: every section is
        `config.toml`, and the model policy is a revision-gated collection
        record rather than a `providers.toml` section. Record writes go
        through `run_settings_action` (`add`/`save`/`delete` with `values`
        and `expectedRevision`), the plan naming no record write. Record
        specs (Provider, policy, MCP server, …) live beside their collection
        in the facade, not in the config crate, so `check-config-reference`
        (config.toml's keys) does not scan them and `specs()` stays
        config.toml's. Not fields, kept as they are on save: a Provider's
        `requestHeaders` and `models`, the policy's `turnBudget`, an MCP
        server's environment, headers, credential maps and per-tool maps.
        Not actions: MCP OAuth login (`mcp_oauth_login` takes closures and
        waits for the browser), removing a profile's key or a profile,
        editing a role (S6). Installed packages are not a collection (the
        plugin browser page, S4/S6). `SettingState.issue` covers values a
        layer above the user's file could not use; the user's own file is
        repaired at load (a recovery, not an issue).
      - Tests: `config/tests/settings/catalog.rs` (values, layers and an
        issue per key; a host has no standalone sections; plugin options
        written with their schema, refused out of range or unknown, reset;
        a plugin toggle writes `disabledPlugins` alone);
        `agent/src/tests/settings_catalog.rs` (every owner read and no
        secret in the serialized catalog; settings and plugin options in one
        write, a refusal writing nothing; a Provider's add, stale and
        missing revision, unknown field, save keeping its token, delete;
        the model policy's save, `null` unset and refusal; an MCP server's
        add, transport switch and delete; a plugin toggle; profiles and
        projects in a layered configuration; the strict request).
      - Docs (both languages, same ids): `architecture#settings-catalog`
        (new), `embedding#configuration-in-a-host` (the three calls),
        `internals` (the settings API).
      - No golden changed (nothing model-visible); no TUI snapshot changed.
      - Hanafubuki at its next pin:
        - `AgentSettingsView.web_search_fields` and
          `hanafubuki_agent_config::agent_settings_fields` are gone.
          `src-tauri/crates/hanafubuki-agent-integration/src/settings_form.rs`
          (`AgentSettingsPage::from`, `agent_form_fields`, `AgentFormField`)
          builds `web_search_fields` from the `web` section instead:
          `hanafubuki_agent::config_sections()` (static, no running Agent)
          or `settings_catalog().await?.sections`, id `"web"`. Mapping:
          `SettingKind::Choice` → `FieldKind::Select` (options' `value` and
          `label`), `Text { nullable }` → `FieldKind::Text` with
          `nullable`, `Boolean` → `FieldKind::Boolean`; `label`, `summary`
          and `help` are plain English text, not i18n keys (the
          `settings.fields.webSearchProvider.*` keys go unused);
          `commit_group` (`"webSearch"`) → `patch_group`; the commit mode is
          no longer sent (change for a choice or a boolean, blur for a
          text); `searxngBaseUrl` has no `disabledWhen` (no `visible_if` in
          the spec). The keys are unchanged, so `AgentSettingsPatch` writes
          as before. `settings_form/tests.rs` (which calls
          `agent_settings_fields()`) and `docs/backend-spec.md` (§ settings,
          which names it) follow; the frontend's `webSearchFields` is
          unchanged if the integration crate keeps producing it.
        - Additive: `settings_catalog`, `write_setting_values`,
          `run_settings_action` and their types (`SettingsCatalog`,
          `SettingsCollection`, `SettingsRecord`, `SettingsAction`,
          `SettingsActionScope`, `SettingsActionRequest`,
          `SettingsActionOutcome`, `SettingsGroup`, `SettingState`);
          `SettingsSection` gains `group`, `order`, `values`;
          `AgentConfig::settings_sections`, `write_values_with_options`,
          `user_document`, `is_layered`, `set_user_json`.
        - Behaviour: `set_plugin_enabled` writes `disabledPlugins` alone
          (other keys keep their provenance).
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features`; `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`, `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` (all 35 pages prerendered). CI on Linux, Windows
        and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43227750990
        (run on a83c226, which differs from the landed commit only in this
        line; Site: .../runs/43227751089).
      - Module size: no exception added or raised.
      - For S4: render `SettingsCatalog` generically: `SettingsGroup`'s
        order is the tabs'; sections by `order` within a group; a
        section's keys are full keys for `write_setting_values` (plugin
        options included); a collection's `record` specs are keys relative
        to the record (`transport.url`, with `visible_if` on
        `transport.type`), `save` merges only the values sent (send one
        field to save it at once) and a revision-gated collection's write
        names `revision` (a stale one is refused: read the catalog again
        and keep the edit); `confirm` actions ask first; `secrets_set` says
        which secrets show as `••••`; a collection without `save` is
        read-only; `issues` on a collection are shown, not fatal. The MCP
        OAuth login and the credential values stay direct calls
        (`mcp_oauth_login`, `mcp_set_credential`).
- [x] S4: `settings_center/` replaces `config_center/`: Codex's tabs,
      label · value · description rows, search across tabs, saving at once,
      inline editors, reset, per-row hints, narrow fallback, mouse; a page
      stack with asynchronous writes; the `PageRegistry` hosting the theme,
      status line, title, keymap, presets, plugin browser and hook-trust
      pages.
      Done (2026-11-12):
      - `crates/tui/src/settings_center/`: `mod.rs` (the page stack, the
        requests and replies, the notice), `main_page.rs` (the tabs:
        `SettingsGroup`'s order, sections by `order`, a heading for a
        section of more than one setting, `visible_if` evaluated, then the
        group's collections and registry rows; `/` search over every tab's
        settings, collections, records and pages, a result described
        `Group · Section`), `collection.rs` (records with state and
        summary, issues above them, `a` add, `d` delete, collection
        actions), `form.rs` (a record's fields saved one at a time with
        the collection's `revision`; a refused field stays `unsaved` and
        goes with the next change; an add, or a commit group, is a draft
        saved by its Add or Save; record actions, `confirm` ones asked
        first; read-only without `save`), `editor.rs` (the row's one-line
        editor checked with `SettingSpec::check` before it is sent and
        kept with the reason when refused; Codex's child picker for a
        choice, `(current)`, disabled values with their reasons, and for
        confirmations; a list of texts edited item by item), `render.rs`
        (the title, `Changes save automatically` or the last outcome:
        `Saved`, `Saved · the project layer overrides it`, `Saved ·
        applies at next launch`, `Could not save: …`; tabs with issue
        counts, scrolled to the selected one; label · value · description
        columns, `[x] On`, `••••`, `(unset)`, a dim `· profile NAME` /
        `· project` / `· -c`; `›` on the selected row; the selected row's
        hint; under 45 columns of description the selected row's
        description below the list), `values.rs`, `registry.rs`.
        Booleans save at once, choices on selection, texts on Enter; `r`
        resets (`SettingWriteOp::Reset`); a singleton collection without
        `add` (the model policy) opens as its form. Mouse: wheel, a click
        on a tab, a click selects a row and a second click activates it.
      - The `PageRegistry` (`registry.rs`): a page is a command's views,
        declared beside the command (`commands::theme::PAGE`, …, listed in
        `commands::SETTINGS_PAGES`), attached to a setting (`tui.theme`,
        `tui.statusLine`, `tui.terminalTitle`: the row opens the page in
        place of its editor) or a row of its own (Keymap and Presets in
        Interface, Installed plugins in Plugins, Hook review in Tools).
        The command runs as typed; its views stay in the bottom pane's
        stack (so `send_step`'s next views and `CommandReply`s reach them
        as before) and the center draws the command's top view in its body
        and hands it the keys (Ctrl-C as Esc) and pastes. After each key a
        `WorkspaceEvent::SettingsPageCheck` is queued behind any step the
        key sent; when the command has no view left the page pops to the
        row that opened it and the catalog is read again.
      - Asynchronous: `agent_bridge/catalog.rs` runs each read, write
        (`write_setting_values`) or action (`run_settings_action`) as a
        task of the Agent's runtime that reads the catalog again after it
        and answers with both (and `[tui]` re-read after a `tui.*` write)
        as `WorkspaceEvent::Settings`, which reaches the center whatever
        conversation has the focus. One change at a time ("Wait for the
        last change to be saved."); a read answered after a later call is
        ignored. `App::save` (the pages' `[tui]` writes, `/raw`, `/tui`)
        is asynchronous too (`AgentBridge::save_tui_preferences` →
        `AppCommand::PreferencesSaved`); `write_tui_preferences`, the
        bridge's `config_state` and `write_{agent,standalone}_settings`,
        `write_plugin_options` and `ConfigState` (all `block_on`) are gone.
        After a `tui.*` write the app takes `[tui]` whole
        (`adopt_tui_preferences`: keymap, raw output, theme, status line,
        title, fullscreen preference).
      - App: `/config [TARGET]` opens the center at a tab, a section id, a
        collection id or a page id (`/config quick` unchanged); the center
        takes the alternate screen with the mouse in both presentations;
        `alt_screen_open` counts it (Scrollback's tail no longer draws
        under it); keys, the pointer and pastes go to it first.
      - Facade (additive): `SettingsAction.names_record` (`namesRecord`),
        the label an `add` asks the new record's id by (MCP `Server id`,
        profiles `Profile name`, projects `Project path`); `None` where the
        owner names it (Providers, marketplaces).
      - Removed: `config_center/` (and its two module-size exceptions),
        `app/workspace/config.rs`. Moved: the plugin options form
        (`options_form`, `changed_options` and their test) into
        `commands/plugins/options.rs`, which `/plugins`' `o` uses.
      - Deviations from the plan: pages are an enum on the stack, not
        `trait SettingsPage` (the overrides are all one kind, a command's
        views). Replies come through a workspace event, not
        `CommandReply` (which reaches bottom-pane views by command name).
        Page overrides attach in the TUI's registry (the catalog has no
        `page` field). The layers page and the Session-role domain are
        gone: each row names the layer that sets it, profiles and projects
        are collections, `/warnings` keeps the layers' warnings and the
        role is `/role`'s. Advanced settings are shown with the rest (no
        advanced toggle).
      - Tests: `settings_center/tests` (snapshots `memory_140`,
        `memory_50`, `interface_140`, `providers_140`,
        `provider_record_50`, `search_50`; every spec of `config_sections()`
        has a row, a conditional one while its condition holds; an
        untouched center writes nothing; a boolean written at once; an
        edit checked first and kept when refused; reset; a record field at
        the revision, a refused one sent with the next; a confirmed delete;
        an add naming its record; a search opening another tab's choice; a
        commit group saved together; clicks); `app/tests/settings_center.rs`
        through `App` (a toggle saved and taken in Scrollback and
        Fullscreen; a search across tabs and an edit refused then saved;
        Esc from the theme and keymap pages back to their rows; a write a
        profile overrides says so; a click on a tab and on a row). The old
        `/config <section>` tests are gone; `/config modelPolicy` replaces
        `/config providers` in `the_configuration_center_opens_the_model_policy_directly`;
        the presets test waits for the asynchronous save's message.
      - Docs (both languages, same ids): `terminal-interface#config`
        rewritten for the center, the commands table's `/config` and the
        role's `/config role` line; `architecture` (the settings center,
        the catalog's `namesRecord`); `config-reference` and
        `providers-models` (`/config modelMetadata`); `plugin-rules`.
      - No golden changed (nothing model-visible). New snapshots only.
      - Hanafubuki at its next pin: additive `SettingsAction.names_record`
        (serialized `namesRecord`, `null` when the owner names the record);
        nothing else (the TUI is not Hanafubuki's).
      - Module size: the two `config_center/forms/mod.rs` exceptions
        removed; none added or raised.
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features`; `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`, `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` (all 35 pages prerendered). CI on Linux, Windows
        and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43237517798
        (Windows passed on a rerun of its job: the first attempt failed only
        `tests::sessions::children::a_driven_child_runs_a_turn_answers_and_is_interrupted`
        with `CancellationNotConfirmed`, a timing test this change does not
        touch; the run is on the landed commit but for this line; Site:
        .../runs/43237517782).
      - For S5: `/config TARGET` takes the ids the commands map to
        (`/mcp` = `/config mcp`, `/providers` = `/config providers`,
        `/plugins` = `/config packages`, `/hooks` = `/config hooks` or
        `hook-review`, `/theme` = `/config theme`, `/statusline`,
        `/title`, `/keymap`, `/presets` likewise, `/memories` =
        `/config memory`, `/tui` = `/config tui`). The page overrides are
        still the commands' bottom-pane views, hosted (`/theme`, `/keymap`,
        … run alone still open them over the composer); `/mcp`,
        `/providers`, `/memories`, `/tui` and `/config quick` still have
        their own views and writers (`SettingsCall::SetMemory`,
        `SetCompactionStrategy`, `app/preferences.rs`' `shadowed`).
        `[tui]`'s section is the config crate's (`tui`), already in the
        catalog; keymap and presets have no spec (pages only).
- [x] S5: the separate editor views removed; `/mcp`, `/providers`,
      `/plugins`, `/hooks`, `/theme`, `/statusline`, `/title`, `/keymap`,
      `/presets`, `/tui`, `/memories` open the center at their page;
      `/config quick` and the second memory and compaction writers removed.
      Done (2026-11-12):
      - Commands: `/mcp` → `/config mcp`, `/providers` → `providers`
        (`policy` → `modelPolicy`), `/plugins` → `packages`, `/hooks` →
        `hook-review`, `/memories` → `memory` (the tab), `/tui` → `tui`,
        `/theme`, `/statusline`, `/title`, `/keymap`, `/presets` → their
        page (`commands::open_settings`); `/mcp login ID` opens the center
        at `mcp-login` on that record (`WorkspaceEvent::OpenSettings {
        target, record }`). Kept as they were (not settings interfaces):
        `/mcp verbose`, `/statusline ITEMS`, `/title ITEMS|--clear`,
        `/keymap debug`, `/raw`. `/config quick` is gone (`ConfigPanel`).
        `/providers` runs during a turn now, as `/config` does.
      - Pages: `PageOverride` gains `open: fn(&mut CommandContext,
        Option<&str>)`, the views a page shows, apart from the command's
        `dispatch` (which now opens the center); `Attach::Record {
        collection, label }` is a page that is an action of each record of
        a collection (listed in the record's form after its actions). New
        pages: `memory-store` (row in Memory), `provider-model` (record of
        `providers`: Choose the model), `mcp-login` (record of `mcp`: Log
        in). A page asks the center to run a collection action as its own
        (`WorkspaceEvent::SettingsAction`, the center filling the revision).
        Page checks are numbered: a check queued by an earlier key came
        before a later key's step and closed the page under it (the
        keymap's and presets' multi-step views); only the latest counts.
        A preset run (`AppCommand::Replay`) closes the center first.
      - One writer for settings: the center, `/raw` and the theme, status
        line and title pages write through `write_setting_values` /
        `run_settings_action` only. Removed:
        `SettingsCall::{Config, SetCompactionStrategy, SetMemory}`,
        `SettingsReply::Config`, the `/providers` screen and every
        `ProviderCall` (`agent_bridge/providers.rs` keeps only the model
        listing for the model page), the `/mcp` manager and `McpCall::{Load,
        Write, Delete, Connect, Refresh, Logout}` (kept: `Report`, `Login`,
        `CancelLogin`), `AppEvent::SetFullscreenTranscript`. `/raw` and the
        theme, status line and title pages write by key
        (`AgentBridge::write_tui_settings` → `AppCommand::TuiSettingsWritten`,
        the shadow notice from `SettingsWriteOutcome.shadowed`).
        `AgentTuiPreferencesPatch` keeps only `keymap` and `presets` (no
        spec, a named exclusion since S2), and `app/preferences.rs`'
        `shadowed` compares only those.
      - Facade (catalog), so the center has what the views had: MCP
        record fields `transport.publicEnvironment`/`publicHeaders` and
        `toolApprovalPolicies`/`toolExposurePolicies`/
        `toolMaximumConcurrentCalls` as `NAME=VALUE` text lists
        (`record::{maps_as_lists, lists_as_maps}`, an item that is not one
        or a name given twice refused); MCP record summary `· N tools`
        when connected; a save that drops a credential reference and a
        delete remove the credentials no server references. New collection
        `mcpSecrets` (`service/settings/mcp_secrets.rs`): a record per
        credential-backed variable or header, `SERVER/NAME`, its `value` a
        secret spec (masked while typed, never returned); add writes the
        reference at the revision then `mcp_set_credential`, save sets the
        credential, delete drops the reference and the unreferenced
        credential. Provider `requestHeaders` (written `NAME=VALUE`, listed
        by name only since the registry checks their values as
        credentials, a name alone keeping its value). Model
        policy `customTurnBudget` + the eleven `turnBudget.*` fields
        (starting from the kernel's `AgentTurnBudgetPolicy::default()`;
        off writes `turnBudget = null`), and the default model's known
        reasoning levels and window said in the summaries of
        `reasoningEffort` and `contextWindowTokens`.
      - Removed view → center (feature by feature):
        - `/mcp` list → MCP servers collection (state = phase, issue = last
          error, `· N tools`); `a` add, `d`/Delete with confirmation, Enter
          edit; `e` enable → Enabled field; `c` connect/reconnect →
          Connect / Refresh; `o` → Log out; `l` / `/mcp login ID` → Log in
          page (address shown, `c` copies it, the transcript keeps it, Esc
          and Ctrl-C cancel, a failure shown until Esc). Form: transport,
          program, argv (a list now, not a quoted line), cwd, URL, OAuth,
          enabled, required, protocol, approval, exposure, enabled/disabled
          tools, timeouts, calls → fields (S3); environment/headers public
          rows → `NAME=VALUE` lists; secret rows → MCP secrets; per-tool
          limits/approval/exposure → lists. Plugin servers (read-only rows)
          → `/mcp verbose` (S6 brings plugin MCP servers as settings).
        - `/providers` list → Providers collection (`default`/`off`
          states); `a`, `d`, Enter; `e` → Enabled; `*` → Make default; `p`
          → `/providers policy`/Model policy. Form: name, protocol, base
          URL, token (masked), enabled → fields; header rows →
          `requestHeaders`; Test (a draft) → List models on the record once
          added (an add is one write); model picker and manual id → Choose
          the model page (one modelPolicy save of default Provider + model).
          Policy form: every field (S3) plus the turn budget → custom
          budget; reasoning levels as a choice → listed in the summary
          (free text); known window placeholder → summary. First-run setup
          → the center at Providers (`open_setup`); `setup_needed` is
          recomputed from the registry the bridge reads after an action on
          `providers`/`modelPolicy` (`CatalogReply.registry`).
        - `/memories` → Memory tab (Memory, Use memories, Generate memories
          … as settings; Turn memory on → Memory) + Memory store page
          (status, latest error, this session contributes, Reset with
          confirmation).
        - `/config quick` → rows of the center: Raw output, Status line
          colors, Fullscreen, Alternate screen (Interface), Compaction
          strategy (Context); its editor entries → the commands/pages.
        - `/tui` → Fullscreen row; `/theme`, `/statusline`, `/title`,
          `/keymap`, `/presets`, `/plugins`, `/hooks` → their unchanged
          views, now only as pages of the center.
      - `[tui]` section: already registered (the config crate's `tui`
        section, S1/S2; checked here); keymap and presets stay pages
        without specs.
      - Form widget (`bottom_pane/form`): rows, the secret column,
        Advanced and conditional items removed (no user left); `FormView`
        keeps text, choice, toggle and action. `model_facts::sourced_window`
        removed.
      - Deviations: the plan's "maps are collections" holds for the MCP
        secrets (masked values need a secret spec); a record's other maps
        are `NAME=VALUE` lists, since the catalog has no nested collection
        and these are fields of one record. The browsers that are pages
        keep their owners' calls, as S4 made them: the plugin browser
        (install, on/off, options, settings: `PluginsCall`, the `plugins`
        collection's calls), the hooks browser (`set_hook_enabled`,
        `trust_hooks`: the `hooks` collection's), the memory store
        (session contribution and reset, no settings); keymap and presets
        through the `[tui]` patch (no spec). Routing the two browsers'
        writes through `run_settings_action` is left to S6, which reworks
        package settings and hooks. A Provider's model chosen while
        another change is still being saved is refused with the center's
        "Wait for the last change to be saved." and has to be chosen
        again. A page whose last view ends on a reply (a login done) is
        checked after each reply as after each key. Test-before-save became List
        models after the add. Reasoning levels are a summary, not a choice
        (the spec is the field's for every model). The center's notice says
        `Added`/`Deleted`/`Saved` for those actions.
      - Tests: facade `settings_catalog` (MCP maps round trip and refusals,
        MCP secrets add/save/delete and credential cleanup on a transport
        change, Provider headers (values never returned, kept by name), custom
        turn budget); config `tui` tests
        on the presets patch; TUI: every settings command opens the center
        at its page (chat), hooks browser as a page, `app/tests/settings.rs`
        rewritten through the center (first-run setup to a session, model
        page, model policy with budget, the listed levels and the reported
        window, an MCP server with a secret added and deleted, `/mcp login`
        over its record, a login ended by its reply closing its page), keymap/presets/appearance/memory tests through
        the hosted pages (`Harness::leave_settings`, `select_setting`,
        `edit_setting` shared). Removed: the MCP form/argument tests, the
        Provider draft test, the `/config quick` test, the form rows test.
      - Snapshots (why): `memory_140`, `memory_50` (the Memory store
        row), `provider_record_50` (the Choose the model row widens the
        label column). No golden changed (nothing model-visible).
      - Docs (both languages, same ids): `terminal-interface` (modes,
        layout, commands table, `#setup`, `#forms`, `#providers`,
        `#plugins`, `#mcp`, `#hooks`, highlighting, status line, keymap,
        presets, `#config`), `mcp#interface`/credentials/oauth,
        `providers-models#interface`, `memory`, `config-reference`,
        `troubleshooting`, `quick-start`, `architecture` (catalog, center),
        `internals` (bridge calls).
      - Hanafubuki at its next pin: the catalog gains collection
        `mcpSecrets` (after `mcp`), MCP fields (`transport.publicEnvironment`,
        `transport.publicHeaders`, `toolApprovalPolicies`,
        `toolExposurePolicies`, `toolMaximumConcurrentCalls`), Provider
        `requestHeaders` (names only, values never returned; a name alone
        keeps its value), model policy `customTurnBudget` and
        `turnBudget.*`, additive; MCP save/delete now also remove
        unreferenced credentials. Breaking: `AgentTuiPreferencesPatch` has
        only `keymap` and `presets` (the `[tui]` scalars are written by key
        with `write_setting_values`).
      - Module size: no exception added or raised.
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features`; `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`, `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` (all 35 pages prerendered). One full run failed
        `app::tests::queue::esc_returns_a_scrolled_conversation_to_the_bottom_before_it_interrupts`
        (a 60 s wait under load); it passes alone, on this change and on
        a0cc070, and in the next full run. CI on Linux, Windows and macOS:
        https://github.com/licyk/hanafubuki-agent/actions/runs/43260265539
        (run on eaa55da, which differs from the landed commit only in this
        line; macOS passed on a rerun of its job: the first attempt got no
        runner and was cancelled after 15 minutes in the queue, with no
        step run; Site: .../runs/43260265771).
      - For S6: the add form of a collection shows `(unset)` for every
        field (a new MCP server shows no transport until one is chosen, an
        unset boolean toggles to on); record specs carry no defaults. Saves
        made by hosted pages (theme, keymap, presets) report in the
        transcript, hidden while the center is open. Plugin MCP servers are
        only in `/mcp verbose`.
- [x] S6: the settings missing today: profiles, project trust, roles, hook
      definitions, per-model overrides, plugin MCP server and tool filters,
      package settings from `configSchema`, marketplaces.
      Done (2026-11-12):
      - Per item (§4.4's list):
        - Profiles: done now. They were a collection (S3); a save's `null`
          now removes a key from the profile and a confirmed `delete`
          removes the profile, refused for the one the Agent runs with
          (`-p`). A profile's fields carry no defaults (a new profile sets
          nothing).
        - Project trust: already done (S3: `projects`, add/trust/untrust).
        - Roles: done now. The `roles` collection adds, edits (key by key,
          `null` removing one) and deletes the user's `[agents.<name>]`
          entries of `config.toml`: `description`, `instructions`,
          `instructionsMode`, `model`, `reasoningEffort` (checked as a
          level), `spawnable`; the catalog is rebuilt after each write. A
          role of a role file (or an entry with `configFile`), a profile, a
          project, a plugin or a built-in one is shown with
          `SettingsRecord.read_only` (why and where it is written) and a
          save or delete of it is refused with that reason.
        - Hook definitions: done now, shown. A hook's record summary is its
          whole definition (command, timeout, status message, the file or
          plugin it is in) beside its trust (state) and `enabled`.
        - Per-model overrides: done now. New collection `providerModels`
          (Model group): a record per `PROVIDER/MODEL` (split at the first
          `/`), fields `contextWindowTokens`, `maxOutputTokens`,
          `imageInput`, `reasoningEfforts`, add/save/delete at the
          `providers.toml` revision through a new crate-private
          `write_provider_model` (the registry's mutation; the model id
          checked as the reader takes it: 1 to 256 bytes, not blank).
        - Plugin MCP servers and tool filters: done now. New collection
          `pluginMcp` (Tools group): a record per server an enabled
          installed plugin runs (`PLUGIN/SERVER`, from
          `mcp_snapshot().plugin_servers`), its connection phase, tools and
          last error; `save` writes `enabled`, `enabledTools`,
          `disabledTools` as leaf keys under `[plugins."<id>".mcpServers.
          <server>]` (`null` removes one, back to the plugin's), the server
          with them `validate()`d first, then the packages reload.
        - Package settings from `configSchema`: done now. A section per
          installed code plugin whose schema lowers to options
          (`plugins."<id>".settings`, Plugins group, after the rows'
          option sections), its keys `plugins."<id>".settings.<option>`
          (`package_setting_key`, public); `write_setting_values` writes
          them through `set_package_settings` (options and JSON Schema
          checked, guest restarted): the user file's settings with the
          changes made, a reset removing the key. The lowering now labels
          an option by its `title` (S1's leftover).
        - Marketplaces: already done (S3: `marketplaces`, add/upgrade/
          remove).
      - S5's leftovers: an add form starts from the record specs' defaults
        (filled in the facade from the template the add writes over:
        Provider `enabled`/`protocol`, MCP `new_server(server_defaults)`
        with both transports, the model override's default), so a new MCP
        server shows its stdio fields and Enabled shows `[x] On`; only
        changed fields are sent. A hosted page's save (keymap, presets,
        theme, status line, title, `/raw`) is said under the center's title
        too (`SettingsCenter::report`). Plugin MCP servers are the
        `pluginMcp` collection. The browsers write through the catalog:
        hooks (`save`, `trust`), the plugin browser (new `packages`
        collection: `add` = install from `PLUGIN@MARKETPLACE` or a
        directory, `save` (enabled), `update`, `retry`, `trust` (its hooks),
        `delete`, `purge`; `marketplaces`; `plugins`; built-in and package
        options as writes by key).
      - Catalog vocabulary: `SettingsRecord.read_only` (`readOnly`);
        record specs carry `default`; `r` on a record's field sends `null`
        for any field (the owner refuses where a field is always set).
        Config: `UserEntry { Profile, Role, PluginMcpServer }`,
        `AgentConfig::write_entry_values` and `remove_entry` (leaf writes,
        `null` removes, comments kept).
      - Deviations: hook definitions are shown, not written (they live in
        `hooks.json`/`[hooks]` arrays of several files and plugins; editing
        them is not in §4.4's "only the latter is editable" scope); roles
        edit only the user's `config.toml` entries and not role files, and
        "open the file" is the record's read-only reason naming the path
        (the TUI has no editor launcher); the narrowing role keys are not
        fields. The grant of a code plugin's permissions stays a direct
        call (`grant_package_permissions`, a consent for what the browser
        showed, as the OAuth login), and so do the raw JSON settings of a
        code plugin without a `configSchema` (no spec; the plan's carve-out).
        Package settings carry no per-layer provenance (`[plugins]` is not
        read key by key): set is `User`, else `Default`; a write of them is
        one plugin's alone ("written on their own"), not mixed with other
        keys. Installing a linked directory is not an action (the CLI's
        `plugins link`). The plugin browser's page is now `plugin-browser`
        ("Plugin browser"; `/plugins` opens it), since `packages` is the
        collection's id. `SettingsActionOutcome` carries no
        `AgentPackageInstall`: the bridge reads the package and its hooks to
        trust after an install or update.
      - Tests: facade `tests/settings_collections.rs` (an add's defaults
        equal what it writes; a profile's key and a profile removed, the
        selected one refused; the user's roles added, saved, unset and
        deleted, a built-in role's read-only reason, a `none` effort
        refused; a model's overrides added, stale revision and zero window
        refused, unset, deleted; a hook's whole definition; an installed
        plugin installed, turned off and uninstalled; a code plugin's
        settings section, written, refused out of range, refused mixed,
        reset), `tests/packages/mcp.rs` (unix: a plugin's server
        restricted and returned to the plugin's through its record),
        `tests/settings_catalog.rs` (the collection list); packages
        `options.rs` (`title` as label); TUI `settings_center/tests/
        interaction.rs` (an add starting from its defaults with a
        conditional field shown, a read-only record, `r` unsetting a record
        field), `app/tests/preferences.rs` (the keymap page's save said in
        the center), `app/tests/settings.rs` (a new MCP server starts on),
        `commands/plugins/tests.rs` (a code plugin's options saved by key).
      - Snapshots and goldens: no `.snap` changed, no golden changed
        (nothing model-visible).
      - Docs (both languages, same ids): `architecture#settings-catalog`,
        `terminal-interface#config`, `#plugins`, `#hooks`, `internals`
        (bridge calls), `embedding` (`write_setting_values`), `mcp#interface`,
        `providers-models#per-model` and `#interface`, `roles#inline`,
        `config-layers#profiles`, `writing-plugins#config-schema`.
      - Hanafubuki at its next pin: additive: collections `providerModels`
        (after `providers`), `pluginMcp` (after `mcpSecrets`), `packages`
        (after `plugins`); package settings sections
        (`plugins."<id>".settings`) and `package_setting_key`;
        `SettingsRecord.read_only` (`readOnly`); record specs' `default`;
        `roles` gains `add`/`save`/`delete` and fields `instructions`,
        `instructionsMode`; `profiles` gains `delete`; hooks' summary is
        the definition; `UserEntry`, `AgentConfig::write_entry_values`,
        `remove_entry`; a lowered `configSchema` option's `label` is its
        `title`. A struct literal of `SettingsRecord` needs `read_only`.
      - Module size: no exception added or raised.
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features`; `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`, `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` (all 35 pages prerendered). CI on Linux, Windows
        and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43272937692
        (run on 53687a8, which differs from the landed commit only in this
        line; Site: .../runs/43272937763).
      - For S7: the pages above say what S6 changed; the full pass still
        owes `configuration`, `config-reference` (the new collections and
        the plugin MCP/package settings keys under `#plugins`),
        `plugin-rules`, `host-plugins` and `writing-plugins` (declaring
        settings), and `plugins.en.mdx` (the `/hooks` line).
- [x] S7: documentation in both languages: `terminal-interface`,
      `configuration`, `config-reference`, `architecture`, `embedding`,
      `plugin-rules`, `writing-plugins`, `host-plugins`.
      Done (2026-11-12), every page checked against the code, both
      languages, same `[#id]`s:
      - `terminal-interface`: the commands table (`/compression` had no
        "during a turn" cell; `/config [target]`, `/providers [policy]`),
        the commands that take arguments (`/compression`, `/config`,
        `/providers` were missing) and their usage lines; `#mcp` (the
        plugins' MCP servers are the `pluginMcp` collection, not "not
        settings"); `#plugins` (where the options form writes, the same
        options as the plugin's section); `#config` (`(empty)`, `(none)`,
        `· applies from the next session`, the profile and `-c` layers in a
        save's note, an unknown target); the front matter.
      - `configuration#write`: the configuration center's typed forms,
        layers page and plugins domain replaced by the settings center;
        `r` removes a key where `config set` cannot.
      - `config-reference`: rows checked by hand against the specs, the
        structs' defaults, the decoders and validators (all `config.toml`,
        `providers.toml` and `mcp.toml` tables). Fixed: `disabledPlugins`
        lacked `hooks`; `minSummaryLength > maxSummaryLength` takes the
        layer below, not the defaults; `reasoningEffort`'s identifier
        characters; `baseUrl` refuses a query; an MCP HTTP `url` refuses
        credentials and a fragment; the `mcpServers` keys default to the
        package's and replace its lists; `mcp.toml`'s `revision`; an MCP
        server's keys are required (below). Added the collections that
        show or edit each table (`#profiles-projects`, `#hooks`,
        `#plugins`: `packages`, `pluginMcp` and the package settings keys
        `plugins."ID".settings.OPTION`; `#marketplaces`; `#plugin-options`
        and its writers, the dead "network form"; `#agents`: `roles`;
        `#providers-toml` and `#provider-models`: `providerModels`). No row
        named a key that no longer exists.
      - `architecture#settings-catalog`: the owners' calls
        (`write_provider_model`, `set_hook_enabled`, `remove_entry`).
      - `embedding`: the `settings_catalog()` row (every collection, the
        package settings sections, `readOnly`, `namesRecord`, `issues`,
        defaults), `#plugin-rows` (options as a catalog section, written
        with `write_setting_values` or `set_plugin_options`).
      - `plugin-rules`: options as settings, both writers, a guest's
        `configSchema` options as a catalog section; the dead "network
        form".
      - `writing-plugins`: declaring settings with `configSchema` alone (the
        catalog section, its keys, how a write of them goes), the lowering's
        rules the page left out (empty `enum`, `minimum` above `maximum`, a
        default that does not fit, no secret, choices labelled by value).
      - `host-plugins`: new `#settings`: a row's options declared in its
        descriptor, with an example, read, shown and written as settings.
      - `plugins`: the `/hooks` line (the hook review, or a hook's record).
      - Code found: an MCP server in `mcp.toml` without any of its keys but
        `protocolMode` and a stdio `cwd` (`toolMaximumConcurrentCalls`,
        `toolExposurePolicies`, a timeout…) is dropped whole by
        `repair_settings` (`McpServerConfig` has no serde defaults, and
        `normalize_current_json` supplies none), which conflicts with
        AGENTS.md's rule that our persisted data falls back per field;
        documented as it is, for the owner to rule on. `config set
        plugins."ID".settings.KEY` takes the generic path and writes the
        value unchecked against the `configSchema` (the guest still reads it
        checked); documented. A stale doc comment:
        `AgentPluginDescriptor::options` still says "the configuration
        center's generic form".
      - Validation: `bun install --frozen-lockfile`, `bun run
        format:check:site`, `bun run site:check`, `bun run site:test`,
        `VITE_BASE_PATH=/hanafubuki-agent/ bun run site:build` (all 35 pages
        prerendered as HTML (the zh-CN paths); the English pages are client
        bundles (`*.en.mdx` assets) and the search index, which hold the new
        text); `cargo xtask check`. No code
        changed, so no Rust build or test; no golden or snapshot changed.
      - Hanafubuki at its next pin: nothing (documentation only).
      - [x] The code findings fixed (2026-11-12): an `mcp.toml` server that
        leaves out a key takes a new server's value (`McpServerDefaults`,
        empty lists and tables) in `repair_settings`, without a repair; only
        an id, a transport, its `type` and its program or URL have none.
        `config set plugins."ID".settings.OPTION` (`PackageSettingKey`)
        writes through `write_setting_values` on the started Agent, checked
        as the settings center checks it; the generic writer refuses any
        other key under a plugin's `settings`. The descriptor comment names
        the settings catalog. Docs: `config-reference`, `internals`.
        Validation: fmt, check, clippy, `cargo xtask check`, `cargo test
        --workspace --all-features`, the site steps (35 pages); CI
        https://github.com/licyk/hanafubuki-agent/actions/runs/43281775547
        (Windows rerun once for the known subagents/review flakes).
        Hanafubuki at its next pin: servers missing keys now load; the new
        `PackageSettingKey` export; `set_user_value`/`set_user_json` refuse
        `plugins."ID".settings…` keys.

**Command center ([3])**

- [x] A1: Left on an empty composer under Codex's conditions,
      `Action::OpenAgents`, `← for agents`; root rows with child rollup;
      Needs you / Working / Ready / Inactive / Error; grouping
      (`tui.agentsGrouping`); Codex's keys (Tab filters, Right opens, `x`
      stops, `r` renames, Backspace deletes), confirmations, search,
      worktree, `?` help.
      Done (2026-11-12):
      - Entry: plain Left on an empty composer
        (`chat_composer/agents_navigation.rs`) while it has the keys, no
        popup or paste burst, `/agents` runs in that conversation (the
        command scope, a disabled command) and Left is still
        `editor.move_left`'s; the footer's empty row says `← for agents`
        (after the status line, or before `? for shortcuts`). The keymap's
        `global.open_agents` (no default) opens it too, in
        `handle_workspace_key`. Both presentations (the center keeps the
        alternate screen).
      - Model (`agents_center/`: `mod.rs` loading and rows, `status.rs`,
        `grouping.rs`, `input.rs`, `render.rs`): the sessions without a
        parent; each root's status is the most urgent of its own and its
        children's (`Status::of`): Needs you (a question, or an approval
        read from `snapshot(id).active_activities` for running sessions
        only, `AgentBridge::waits_for_approval`), Working, Ready, Inactive,
        and Error for a root whose own turn failed. Filters All, Needs you,
        Working, Ready, Inactive (with counts) and Archived; Tab,
        Shift-Tab, `1`–`6`. Headings with counts (`2 of 5`) by project,
        directory, status or model; `g` cycles and writes
        `tui.agentsGrouping` quietly (a failure is the center's notice). A
        project is the directory within the main checkout: a worktree's
        `.git` file and its Git directory's `commondir` are read once per
        directory, compared with `PathKey`. Selection kept by session id,
        wrapping at the ends; Enter or Right opens (restores under
        Archived); `/` or F3 searches titles through
        `AgentSessionListRequest.search` (children never searched); `r`
        renames inline (`rename_listed`); `x` stops a running session;
        `a` and Backspace ask in a modal starting on Cancel; `n`, `w` (the
        `/worktree` "start a new conversation" call, or why none can be
        made), `f`; `?` lists every key; pastes reach a search or a rename.
      - Keys: a new keymap context, `agents` (its own input path, letters
        allowed), with `search`, `new_task`, `new_worktree`, `fork`,
        `rename`, `stop`, `archive`, `delete`, `toggle_grouping`; the
        center's chords are read in it, none while text is typed.
      - Config: `tui.agentsGrouping` (`AgentAgentsGrouping`: project,
        directory, status, model; spec in `[tui]`, a row in both
        `config-reference`s).
      - Removed: child rows in the center, `d`/`i`, Left/Right as
        filters, `SubagentsCall::Interrupt` (only the center used it).
      - Deviations from the plan, the smallest change meeting its intent:
        - Six tabs and `1`–`6` (All, the four groups, Archived), not `1`–`5`.
        - Ready is a last turn completed, Inactive one interrupted or
          cancelled (or a child's failed); Codex's loaded-and-idle vs not
          loaded has no backend here.
        - `n` and `w` start in the current working directory, not the
          selected row's (a new worktree is made from its repository).
        - The search matches titles only (the store's search); Codex also
          matches the preview and the directory. Search and rename are
          line edits with no cursor movement, as the resume picker's.
        - All four groupings persist (Codex keeps Directory view-local for
          older clients).
        - The Left check is `move_left` holding Left: the keymap already
          refuses a key or chord prefix another action beside the composer
          would share, so Codex's further scan cannot fire.
        - The session shown is refused before the archive or delete
          confirmation (the check moved from the app into the center).
      - Docs (both languages): `terminal-interface` (Layout's footer, the
        Keys table, the `/agents` row, `#agents` rewritten for what ships,
        the keymap's contexts and its `global.open_agents` and `agents.*`
        rows), `config-reference#tui`, `sessions` (subagents).
      - Snapshots: new `agents_center` (wide, narrow, help, confirm) and
        footer (`← for agents` with and without a status line); changed
        for the footer hint: the composer's `empty`, `status_only`,
        `status_and_composer_fill_height…`, the owned transcript's scrolled
        view, the side pane's two; the settings center's `interface_140`
        (the new row). Not model-visible; no golden changed.
      - Validation: `cargo fmt --all --check`, `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`, `cargo
        clippy --workspace --all-targets --all-features -- -D warnings`,
        `cargo xtask check`, `cargo test --workspace --all-features`;
        `bun install --frozen-lockfile`, `bun run format:check:site`,
        `bun run site:check`, `bun run site:test`. The local
        `site:build` could not prerender: another project's dev server
        holds port 3000, which nitro's preview takes for prerendering
        whatever the config says; CI's `site.yml` built it and prerendered
        210 pages (every page, both languages).
      - Hanafubuki at its next pin: the facade exports
        `AgentAgentsGrouping`, `AgentActivityPhase`, `PathKey`,
        `normalize_path` and `relative_below`; `AgentTuiPreferences` gains
        `agents_grouping`. Nothing to change.
      - Module size: no exception added or raised.
      - Notes for A2: children are not openable from the center until the
        details pane lists them (`open_listed` still opens a child beside
        its parent); stopping a child needs `interrupt_subagent` again;
        the approval read (`waits_for_approval`) and the children pages
        are synchronous on the UI thread, at open, on a key and after an
        action; the main pane shows `← for agents` while a side pane has
        the focus (its composer keeps `has_focus`).
      - CI: https://github.com/licyk/hanafubuki-agent/actions/runs/43295489283
        (Linux, macOS, Windows, all green on the first run) and the site's
        https://github.com/licyk/hanafubuki-agent/actions/runs/43295489407.
- [x] A2: the details pane; a live session watch with background re-reads;
      pin, hide and unread in a new `tui-state.json` (D11); mouse. Docs:
      `terminal-interface#agents`, `data-directory`.
      Done (2026-11-12):
      - Reads off the loop (`agent_bridge/center_reads.rs`): while the
        center is open a thread of its own answers its `CenterJob`s (the
        list from the first page until the sessions wanted are read; a
        session's children; the selected
        session's latest prompt and answer from its last 64 items) and,
        between jobs, watches the sessions the Agent holds every 250 ms
        (`AgentApplicationService::session_snapshots`, new): each one's
        revision, an approval or a question it waits on, its latest
        activity, sent as `CenterRead::Live` only when it changed. The
        center (`agents_center/loading.rs`) asks for one read of each kind
        at a time, drops answers of an older epoch (a search or a filter),
        and from the watch reads again only what changed: the list for a
        session without a parent, the parent's children for a child, the
        exchange when the selected session's revision moved. A read again
        replaces the rows in place (no `Loading…`); an action refreshes
        instead of reloading. Nothing reads on the UI thread any more
        (`waits_for_approval` and the synchronous page loop are gone).
      - Details pane (`agents_center/details.rs`, Codex's 38 columns from a
        body of 90): title, status, model, reasoning, branch, tokens,
        directory; the children with their status; Needs you (an approval's
        subject or the question, `Subagent:` when a child's) and the latest
        activity of the most urgent of the root and its children; `Last
        message` and `Prompt` (two lines). `c` (`agents.children`) gives the
        keys to the children: arrows, Enter or Right opens one beside its
        parent (`open_listed`), `x` stops it (`stop_listed`: a subagent's
        through `interrupt_subagent`, else the turn's cancel), Esc, Left or
        `c` returns. Below 90 columns the details take the list's place while
        a child has the keys.
      - `tui-state.json` (`tui_state.rs`, in the data directory beside
        `themes/`): `agents.pinned`, `agents.hidden`, `agents.seen` (session
        id → milliseconds). Unknown keys are kept as they were; an entry not
        understood is left out and the file moved into `.recovery/`
        (`quarantine_file`) before the next write; a file that is not a JSON
        object is moved there at once; one that exists but cannot be read is
        never written over (its saves say so). Writes run on a thread, the
        latest last; the exit writes synchronously. At most 2000 showings.
      - Pin (`p`, `agents.toggle_pin`): pinned rows first under `Pinned`
        (`HeadingKey::Pinned`) in every grouping. Hide (`h`, `agents.hide`):
        out of every tab and count; a search lists it marked `(hidden)`, and
        opening it un-hides it. Unread: a root whose `updated_at`, or a
        child's, passed its last showing has a bold title and, when Ready or
        Inactive, a blue `●`; a session never shown is not unread, the
        session shown never is. A showing is stamped when the main
        conversation's session changes (`App::note_shown`, from
        `show_update`), when the center opens, after the center's own rename
        or restore, and at exit.
      - Mouse (both presentations; the center now captures it in
        Scrollback): a click on a tab filters, on a row or a child selects,
        on the selected one opens; the wheel moves three rows and stops at
        the ends; nothing while searching, renaming, confirming or in help.
      - The side pane's focus fix: the parked conversation's composer loses
        `has_focus` on a swap, so `← for agents` shows in the focused pane
        only.
      - Deviations from the plan, the smallest change meeting its intent:
        - The facade has no event stream for every session, so "subscribes
          to the facade's session events" is a 4 Hz watch over
          `session_snapshots()` (in memory, no store reads), as the
          subagents' watch polls; the list's reads go to the store only
          when something changed.
        - A click on the selected row opens (as the settings center), not a
          double click.
        - Pinned rows come first among the sessions read; Codex lists its
          pinned threads with a read of their own, which the history store
          has no counterpart for (a read by id), and chasing them page by
          page would re-read pages on every change for a pin that is
          archived or gone.
        - Pinned rows are under `Pinned` in the status grouping too (Codex
          keeps them under their status with a pin glyph).
        - Hidden sessions come back by a search and opening one (a search
          takes every letter, so `h` there types); Codex's hide is not
          persisted, ours is (D11).
        - The prompt is the selected session's latest message, read with its
          latest answer; Codex shows the thread's first prompt (its
          preview).
        - Usage is tokens in and out; Codex's credits are dropped.
      - Docs (both languages, heading ids unchanged): `terminal-interface`
        (`#agents`: pinned, unread, hidden, the details pane and `c`, the
        mouse row and `p`/`h`/`c` rows, the reads and watch replacing "on
        the interface's thread"; the `/agents` row; the keymap's
        `agents.toggle_pin`, `agents.hide`, `agents.children`),
        `data-directory#contents` (`tui-state.json`). `architecture` and
        `internals` say nothing of the center's reads; unchanged.
      - Snapshots: new `agents_center` `center_details` (with the pane, a
        pinned and an unread row, children, an approval, activity, last
        message and prompt) and `center_children_narrow`; changed
        `center_wide` and `center_confirm` (the details pane at 100
        columns, so the list drops its status columns as Codex's does),
        `center_help` (`p`, `h`, `c`, the mouse); the side pane's two
        (`side_pane_beside_the_transcript`, `leader_keys_over_the_side_pane`)
        lose the main pane's `← for agents`. Not model-visible; no golden
        changed.
      - Validation: `cargo fmt --all --check`, `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`, `cargo
        clippy --workspace --all-targets --all-features -- -D warnings`,
        `cargo xtask check`, `cargo test --workspace --all-features`;
        `bun install --frozen-lockfile`, `bun run format:check:site`,
        `bun run site:check`, `bun run site:test`. The local `site:build`
        built but could not prerender (another project's dev server holds
        port 3000); CI's `site.yml` built and prerendered it (run below).
      - Hanafubuki at its next pin: the facade gains
        `AgentApplicationService::session_snapshots()` and re-exports
        `quarantine_file`, `write_atomic` and `FileAccess`. Nothing to
        change.
      - Module size: no exception added or raised.
      - CI: https://github.com/licyk/hanafubuki-agent/actions/runs/43305181559
        (Linux, macOS, Windows, all green on the first run) and the site's
        https://github.com/licyk/hanafubuki-agent/actions/runs/43305181458.

**Timeline ([4], [5], [6], [8], [9])**

- [x] T1: entries that change in place (shared state and revisions), a
      per-entry layout cache, the joined live tail and Fullscreen's deferral
      removed, the status row kept for the whole turn.
      Done (2026-11-12):
      - `history_cell::LiveCell<C>`: a thinking or a tool call is one `Arc`
        from its start, its cell behind a `parking_lot::RwLock` with a
        revision raised by every `update` (raised first, so a change a panic
        leaves halfway is laid out anew). The chat widget keeps the typed
        handle; the app's cells hold the same `Arc`, so its `EntryKey`,
        activity ids, a reader's anchor and a pinned revision survive
        completion. `HistoryCell::revision()` replaces
        `has_stable_transcript_height`; `copy_source` returns an owned
        `String` (it cannot borrow through the lock).
      - Two app events: `InsertLiveEntry` (the transcript takes the entry
        at its start, in both presentations, and the Ctrl-T overlay with
        it) and `WriteLiveEntry` (Scrollback writes it as shown when it
        completes; Fullscreen sends none). Both are routed to a parked
        conversation like `InsertHistoryCell`.
      - The transcript view's layout cache is keyed by (entry, revision,
        width, presentation with `expanded`), an animation tick refreshing
        it at most once a frame. The joined live tail is gone:
        `ActiveCellTranscriptKey`, `chatwidget/live_tail.rs`,
        `sync_live_cells`/`sync_live_tail`, `EntryKey::Live` (now `End`,
        past the last entry), the combined disclosure, `live_ids`, search's
        live layout and the snapshot's `search_live`. A scan whose entry
        changes in place restarts, as a live-tail scan did; a change of the
        last entry below a reader counts as unseen activity; a selection's
        or a reader's snapshot pins every live entry, in view or not, as it
        pinned the live tail.
      - Fullscreen no longer defers: an update applies at once in its place,
        and one that adds an entry while a message streams (`adds_entry`:
        reasoning or tool start, a consumed steer, the compaction cells)
        first commits the message's lines so far. Scrollback keeps its
        deferral until T1b.
      - The status row stays for the whole turn: the commit ticks no longer
        hide it.
      - Tests: the view's live-tail tests rewritten against a `LiveCell` in
        the cells (a reader keeps its revision through completion; a held
        revision released by scrolling; the beginning-jump held; a scan
        restarting; disclosure kept across revisions; unseen activity on an
        in-place change); the cache by revision; the overlay with an entry
        changing among its cells; `chatwidget/tests/entries.rs` (the
        Scrollback tests moved from `tests.rs`, now `tests/mod.rs`, plus
        Fullscreen twins: an entry from its start completing in place,
        Ctrl-O in place, message → thinking → tool → message order without
        deferral, a turn's end leaving entries in place); the app on a
        vt100 screen: a tool call in every Fullscreen frame from its first
        and one cell only, and each step's thought before its call or
        answer in both presentations; a selection extended into a live entry
        that changed out of view reads the revision it showed.
      - Snapshots: no `.snap` file changed. Inline snapshots: the two in
        the rewritten view tests (`empty_entries_add_…`, the in-place
        cell's `version 2` rows) kept identical content;
        `a_reader_in_a_live_entry_…` ends on `new entry` where the
        live-tail test ended on `new tail` (the fixture's text: there is no
        tail). `esc_interrupts_while_the_answer_streams` now asserts the
        status row is still there.
      - Deviations: animation ticks re-lay out only the entry that animates,
        not only its header row (a `TextLayout` cannot splice a row; the
        per-entry cache already bounds the work to one entry). The tick
        rates (32 ms shimmer, 600 ms spinner) stay for T2. The pending-input
        preview's reserved row (§4.7) is a visible change and is left for
        T7. A `LiveCell` in Scrollback's viewport shows its latest rows when
        taller than the space, the thinking cell's behavior, for tool calls
        too. Visible beyond stability: in Fullscreen a message an entry
        interrupts (rare: a later reasoning item or a compaction cell
        within one response) goes on under a new `•` bullet after it; and
        since the status row now lives all turn, every later step's
        `─ Worked for … ─` separator carries the time, where it fell back
        to a plain rule once the first answer had hidden the status.
      - Docs (both languages): `terminal-interface#layout` (Fullscreen's
        entries in place, the status line for the whole turn, the Esc
        callout), `#thinking` (in place in Fullscreen, written on
        completion in Scrollback), `#transcript` (running entries where they
        started), `internals#tui-details` (thinking and compactions).
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features`; `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`, `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` (all 35 pages prerendered). CI on Linux, Windows
        and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43184542093
        (run on b385659, which differs from the landed commit only in this
        line; Site: .../runs/43184542135).
      - No golden changed (nothing model-visible). Nothing for Hanafubuki
        (the TUI crate only). No module-size exception added or raised;
        `notices-policy.toml` lists `chatwidget/tests/mod.rs`,
        `chatwidget/tests/entries.rs` and `history_cell/live.rs` as
        original.
- [x] T1b: Scrollback's tail: the recent entries repainted in place above
      the composer, the oldest completed ones written to scrollback at the
      tail's limit (answers line by line), the whole tail written on the
      next message; its height kept during a turn; mouse capture per D16
      (`tui.scrollbackMouse`); Ctrl-T with controls for written entries.
      Every later T milestone covers both presentations.
      Done (2026-11-12):
      - `app/scrollback.rs`: each conversation's `Tail` (swapped with it
        like its cells and view) keeps the entries not written yet; the
        conversation's transcript view, unused in Scrollback until now,
        lays them out and paints them at the top of the inline viewport
        (`transcript_view/tail.rs`: `measure_tail`, `tail_lines`,
        `render_tail`, `click_tail`, `handle_tail_key`), with the same
        cells, ids, controls and per-entry cache as Fullscreen. All writes
        happen in the frame (`draw_scrollback`), so no frame shows an entry
        in neither place or both: past the limit (60% of the screen, at
        least 8 rows, always one row left above the viewport) the oldest
        completed entries leave as shown, a collapsed one with more to show
        ending with a dim `ctrl + t to expand` (the transcript key's hint)
        in place of its control, an expanded one without it; an entry
        without a control (an answer's lines) leaves in part, as far as
        needed, the rest kept as a continuation `Remainder` cell; a live
        entry never leaves (`LiveEntryCompleted`, renamed from
        `WriteLiveEntry`, tells the tail it may). A tail taller than its
        area shows its last rows under the header row of the entry they
        start in. A message of yours entering the tail writes what was
        before it whole; so do a session opened in its place (`/new`,
        resume: what was live is completed), switching to the other
        conversation (what has completed; its banner after it) and a resize
        (what has completed; the height floor restarts); the exit
        writes everything, live or not, and draws once before `leave()`.
        `/clear` empties it. The workspace's `held` lines are gone.
      - The viewport's height does not go below its height so far in a
        turn (reset when no turn runs), so `Tui::draw` clears it only when
        it grows.
      - No deferral: the chat's `deferred`/`defers`/`flush_deferred` are
        gone; an update that adds an entry while a message streams first
        commits the message so far in both presentations, as Fullscreen did
        in T1. The chat no longer draws live entries itself (the
        `LiveCell` `Renderable` and the live column are gone).
      - Mouse (D16): `tui.scrollbackMouse = "tail" | "off"` (new
        `AgentScrollbackMouse` in the config crate, re-exported by the
        facade; the patch, `/config interface`'s row and the shadow check
        know it). With `tail`, Scrollback captures presses, releases and
        the wheel (`?1000h ?1006h`, no motion; on Windows crossterm's
        console mouse mode, which turns Quick Edit off while held) while
        the tail shows a control; the first wheel event lets it go until
        the next key or entry (an answer's next lines are not an entry). `Tui::set_inline_mouse_capture` keeps the
        state per `Tui` (not the global), and entering the alternate
        screen resets it. A click on a control toggles it; F4, the list
        keys and Enter work in the tail, and Esc leaves the focus before
        it primes editing an earlier message.
      - Ctrl-T: the Scrollback transcript opens compact with controls
        (click, F4); a second Ctrl-T shows the detailed presentation, a
        third closes it (`pager.close_transcript`'s description says so).
      - Tests: `app/scrollback/tests.rs` (what leaves at the limit and in
        order, a live entry holding back what follows, an answer line by
        line, a control's entry whole with its hint, an expanded one
        without its control, the next message, a session opened in its
        place, the exit, the limit's arithmetic); `transcript_view/tail/
        tests.rs` (painted from its first row, separated from what was
        written, the last rows under a header, a click and F4 with the
        tail staying at its latest rows and an entry added after a toggle
        showing); `app/tests/scrollback.rs` on vt100 (a call in place in
        the tail, expanded by a click and by F4/Enter; the viewport's
        height kept during a turn and shrinking after it; the mouse let go
        at the wheel and taken again at a key or entry, never with `off`;
        the oldest entries written above the tail in order on a 15-row
        screen with a row left above the viewport, the rest at the next
        message; a 20-line answer written in order after the question; the
        exit; a resize). `chatwidget/tests/entries.rs`: the in-place tests
        now run in both presentations, the two deferral tests gone; the
        transcript overlay's tests (Ctrl-T twice, a click on a control).
      - Snapshots: `the_overlay_frames_the_detailed_transcript.snap`
        replaced by `the_overlay_frames_the_compact_transcript.snap` (the
        overlay opens compact, 80 columns so the new `ctrl + t details`
        hint fits). No golden changed (nothing model-visible).
      - Deviations from §4.11: any completed entry without a control may
        leave in part, not only an answer (nothing can be expanded in it,
        the plan's reason). A resize writes the tail as far as it has
        completed (a live entry stays, as everywhere), not whole. The
        tail's thinking has no control until T2 (no activity id yet), so
        completed thinking in the tail is not expandable; Ctrl-O still
        changes live thinking there.
      - Docs (both languages): `terminal-interface#modes`, `#reading` (the
        tail, its writes, its controls and the mouse), `#layout` (no
        separate running-calls row), `#thinking`, `#transcript`,
        `#terminal`, the keymap table; `config-reference#tui`
        (`tui.scrollbackMouse`); `#keys` (F4 in the tail);
        `internals#tui-details` (no deferral, the tail, overlays).
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features`; `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`, `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` (all 35 pages prerendered). CI on Linux, Windows
        and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43194742996
        (run on aebba9a, which differs from the landed commit only in this
        line; Site: .../runs/43194742999; the Windows job ran every new
        tail test). Not tried by hand on Windows Terminal or conhost:
        CI proves the build and the tests on `windows-latest`, which use
        vt100, not a console.
      - Hanafubuki at its next pin: `AgentTuiPreferences` and
        `AgentTuiPreferencesPatch` gain `scrollback_mouse`, and the facade
        exports `AgentScrollbackMouse`; a struct literal without `..`
        must add the field. Nothing else (the TUI crate).
      - Module size: exceptions added for
        `config_center/forms/mod.rs` (file, ceiling 601) and its
        `section_form` (ceiling 207): the `/config interface` row took them
        just over; S4 replaces the module. `notices-policy.toml` lists the
        five new files as original.
- [x] T2: thinking entries (`Thinking 3.4s` → `Thought · 20.7s`), one per
      reasoning item, in order, each expandable; Ctrl-O for all;
      `ReasoningUpdated` forwarded; `Working 25s (Thinking 20.7s)`. No
      history format change.
      Done (2026-11-12):
      - `history_cell/thinking.rs`: a `ThinkingCell` per reasoning item with
        an activity id, `thinking:{reasoning id}` live and
        `thinking:h{history item id}` replayed (`ReplayedItem::Reasoning`
        carries the item's `AgentHistoryItemId`, unique in a session and its
        forks, so two pages never share one). The frozen `expanded` flag is
        gone: collapsed it is its header, expanded (`expanded_lines`) the
        dim Markdown follows, so the transcript view's disclosure decides,
        in both presentations, the Ctrl-T overlay too. Headers `• Thinking
        3.4s` (shimmer, tenths, a 100 ms animation tick) and `• Thought ·
        20.7s` (`1m 05s` past a minute, tenths cut, not rounded),
        `(truncated)` kept. An entry has an id and a `+ Show details`
        control only once text has arrived, so a Provider that sends none
        leaves the header alone and Ctrl-O never gives it a `− Hide
        details`. This fixes the reported bug: a click on a thinking's
        control expands it in Fullscreen and in Scrollback's tail (F4/Enter
        too), each entry alone, and a live one expanded stays expanded at
        completion (same entry, same id).
      - Ctrl-O: the chat sends `AppCommand::ToggleThinking` (so the keymap,
        `captures_keys`, an open approval and the Ctrl-C quit arming behave
        as before); the app has the conversation's transcript view expand
        every `thinking:` id among the cells it shows (Fullscreen's
        conversation, Scrollback's tail), or collapse them all when all are
        expanded (`TranscriptView::toggle_thinking`). Not sticky: a later
        thinking starts collapsed. A reader keeps the entry at the top of
        the view (`disclosure_anchor`, now shared with `toggle_activity`);
        following stays following. The footer's `ctrl + o to show/hide
        thinking` (`ThinkingToggle`) and the chat's `THINKING_FRAME` (the
        status row and the app's live tick already redraw) are gone;
        `chat.toggle_thinking`'s description is "Expand or collapse every
        thinking".
      - Status row: the bridge forwards `ReasoningUpdated` as
        `TurnUpdate::Reasoning(AgentReasoningSnapshot)`; the status widget
        keeps a `ThinkingClock` (`turn_completed_millis`, and `active_since`
        read once from epoch milliseconds onto `Instant`) and reads
        `• Working 25s (Thinking 20.7s) • esc to interrupt` once the turn
        has thought (shown on `turn_completed_millis > 0 ||
        active_since`, never on `phase`, which each step resets), `• Working
        25s • esc to interrupt` before. The working timer still pauses for
        approvals; the thinking clock runs only while an item is open. The
        separator before the answer: `─ Worked for 25s · thought for 20.7s
        ─` when the turn thought (when it appears is unchanged).
      - Tests: `history_cell/thinking/tests.rs` (tenths, `Thought ·`,
        truncated and `1m 05s`, ids live/replayed and kept at completion, no
        id or control without text, the 100 ms tick; live digits asserted by
        prefix only), `status_indicator_widget/tests.rs` (tenths format,
        the clock with a closed and an open stretch and a step's reset, the
        row with the timer paused for an approval), `history_cell/tests.rs`
        (the separator's `thought for`), `transcript_view/disclosure/
        tests.rs` (each thought alone by a click, Ctrl-O over every thought
        with text and back, a hand-collapsed one re-expanded, a text-less
        one without a control, a live one staying expanded at completion,
        a reader's top entry kept), `chatwidget/tests/entries.rs` (ids, the
        command sent, no footer hint, the status row and separator, the new
        headers), the new `app/tests/thinking.rs` on vt100 in both
        presentations (a click on `+ Show details`/`− Hide details`, Ctrl-O
        twice, Ctrl-O while an approval waits), and the resume test asserting
        a `thinking:h…` id.
      - Snapshots: the status row's (`status_indicator_widget` ×3,
        `bottom_pane` ×3) now `• Working 0s • esc to interrupt`; the
        thinking cell's (`Thought · 12.4s`, `Thought · 1m 15s (truncated)`,
        the live expanded one now its body only, the text-less one renamed
        `an_item_without_text_has_no_control_and_no_id`); the live collapsed
        one and the two footer toggle snapshots removed with their code. No
        golden changed (nothing model-visible; no kernel or facade change:
        `ReasoningUpdated` already reached the TUI's sink, and the exec
        `--json` stream is untouched).
      - Deviations: the header click (§4.8) is left for T3, which owns it
        for every entry: toggling on a press in Fullscreen would break a
        drag that starts on a header, and press-and-release-on-the-same-cell
        belongs with T3's other entries; the control click, F4/Enter and
        Ctrl-O work in both presentations. The shimmer is redrawn at the
        100 ms tick in the conversation (the status row keeps its 32 ms).
        Ctrl-O does not reach the Ctrl-T overlay (its own view, pager keys).
      - Docs (both languages): `terminal-interface#layout` (the status row,
        the thinking entry, the separator), `#keys` and the keymap table
        (Ctrl-O), `#reading` (a thinking's control in the tail),
        `#thinking` (rewritten); `internals#tui-details` (ids, disclosure,
        Ctrl-O, ticks, the status clock); `providers-models#reasoning`
        (Ctrl-O's sentence).
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features`; `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`, `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` (all 35 pages prerendered). CI on Linux, Windows
        and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43200798717
        (run on ac9be0b, which differs from the landed commit only in this
        line; Site: .../runs/43200798790).
      - Hanafubuki at its next pin: nothing (the TUI crate only).
      - Module size: no exception added or raised. `notices-policy.toml`
        lists `app/tests/thinking.rs` as original.
- [x] T3: an id and a control on every expandable entry, a click on its
      control or header, in Fullscreen and in Scrollback's tail.
      Done (2026-11-12):
      - Entries: a thinking and a tool call already had ids and controls
        (`+ N lines` for a long output, `+ Show details` for a folded
        `compress`). New: a task plan (`plan:{n}`) collapses to `• Updated
        Plan · X/Y complete` and its steps without the note; past three
        steps, three of them, in progress then pending then completed
        (Codex 57a808a392's preview, wrapped rather than clipped; `plan.rs`
        keeps its `rust-v0.93.0` marker, which the notices check accepts),
        with `+ Show details` when it has a note or more than three steps;
        a short plan without a note is shown as before, in order. Its
        expanded form is today's checklist under the same header. A message
        of yours (`user:{n}`) past 12 wrapped rows shows its first 12 and
        `+ N lines`. Neither has an identity of its own, so
        `history_cell::next_activity_id` numbers them per run (disclosure is
        never persisted); every message has an id, a control only when long,
        as a short tool call has an id without one.
      - The header click (`transcript_view/disclosure.rs`): a header is the
        entry's first row of text (`TextLayout::header_row`, past a
        separator and a prompt's blank padding; F4's emphasis, F4's
        "header visible" test and the tail's header row use it too). A
        plain first left press there, on an entry with a control, is kept
        with the entry's ids; the release toggles it when it is on the same
        cell and that cell still shows the same entry's header. In
        Fullscreen and the Ctrl-T transcript the press still starts a
        selection: only a release that leaves no selected text toggles, so
        a drag from a header selects. A press on a control still toggles at
        once. A toggling release ends the multi-click count, so a quick
        second click on a header toggles it back rather than select a word.
        Scrollback's tail forwards `Up(Left)` as well as `Down(Left)`
        (`press_tail`/`release_tail`, was `click_tail`). The whole header
        row is the target, past its text too.
      - Scrollback now writes a long message and a long plan as they are
        shown, collapsed ones ending with `ctrl + t to expand`, as T1b
        writes every entry with a control.
      - The Ctrl-T transcript takes Ctrl-O (`chat.toggle_thinking`), which
        T2 left out.
      - Keyboard: F4, Up/Down, Enter, Left/Right reach the new entries as
        before.
      - Tests: `transcript_view/disclosure/tests.rs` (a header click toggles
        one entry, past its text too, and a quick second click toggles it
        back; a drag from a
        header selects `Thought` without toggling; a release on another
        entry, a header without a control and the detailed presentation
        toggle nothing; a long message's first 12 rows, `+ 8 lines`, F4 and
        Enter, its header click past the padding), `transcript_view/tail/
        tests.rs` (a header click in the tail, a release elsewhere or
        without a press), `history_cell/tests.rs` (a long plan's preview
        and control, a short plan's none and its order kept, a message over
        12 rows), and
        `app/tests/thinking.rs` on vt100 (a click on `• Thought ·` in both
        presentations; Ctrl-O and a header click in Scrollback's Ctrl-T
        transcript).
      - Snapshots: the two plan snapshots
        (`plan_update_with_note_and_wrapping_snapshot`,
        `plan_update_without_note_snapshot`) gain ` · 1/3 complete` / ` ·
        0/2 complete` in the header, which is the same collapsed and
        expanded. No golden changed (nothing model-visible).
      - Deviations from §4.8: the exploration group does not exist until
        T5, which gives it its members' ids. A plan's and a long message's
        collapsed forms are new presentation, not T4/T5 tool formatting.
        Plans wrap their preview steps instead of clipping them (no
        clipping helper in the crate), so a wide step is no reason for a
        control. Double-click word selection on a header is traded for the
        quick second toggle (a drag still selects there).
      - Docs (both languages): `terminal-interface#reading` (the click row,
        F4's entries, which entries have a control, the header, the tail's
        click), `#transcript` (the header click, Ctrl-O and its row),
        `#layout` and `#thinking` (the plan's header, the line's click,
        Ctrl-O in the transcript); `plan-mode` (the plan's header and
        preview); `internals#tui-details` (a new Disclosure bullet).
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features`; `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`, `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` (all 35 pages prerendered). CI on Linux, Windows
        and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43207722205
        (run on 361982d, which differs from the landed commit only in this
        line; Site: .../runs/43207722487; Linux, Windows and macOS jobs all
        `success`).
      - Hanafubuki at its next pin: nothing (the TUI crate only).
      - Module size: no exception added or raised.
- [x] T4: `AgentToolPresenter` and `ToolPresenterRegistry`; presenters for
      every built-in tool and a fallback; `subject` on `tool_started`,
      `callId` on `execution_progress`; `tool_display` fetched off the UI
      thread at completion and for replay (not an event); the UI-thread
      history read removed. Docs:
      `json-events`, `plugin-rules`, `writing-plugins`.
      Done (2026-11-12):
      - Contracts (`tools/display.rs`, `tools/presenter.rs`):
        `AgentToolPresenter { tools(), subject(tool, arguments),
        display(tool, arguments, output) -> Option<AgentToolDisplay> }`
        (both default to `None`); `ToolPresenterRegistry` (one presenter
        per tool name, a second a `RegistrationError`; `subject` bounded to
        its first line and 200 chars, `MAX_TOOL_SUBJECT_CHARS`; `display`
        falls back), both guarded at the new `ExtensionPoint::ToolPresenter`
        (a panic: no subject, the fallback); `AgentToolDisplay` (`Command`,
        `Patch` with `AgentPatchPreview`/`AgentPatchFile`/`AgentPatchLine`,
        `FileRead`, `Listing`, `Search`, `WebSearch`, `WebPage`, `Mutation`
        with `AgentMutationKind`, `Text`, `Fields`; serde tagged `type`,
        camelCase); `AgentToolDisplay::fallback` (a structured output's
        top-level keys, a nested value summarized, else its text items),
        `structured_output` (the structured value before the argument
        notes), `output_text`; `ArgumentSubjects` (a subject-only presenter:
        one string argument per tool).
      - Presenters: execution (`exec_command`: `cmd`, `Command` from
        `UnifiedExecResult` with `workdir`, exit code, wall time;
        `write_stdin`: `session N ← <typed, escaped>` / `session N`,
        `Command` whose `command` is what was typed; `resize_execution`:
        subject only); workspace-fs (`read_file` `FileRead` with
        `start_line: Some(1)` for a read from offset 0 only, `list_directory`
        `Listing`, `search_text` `"q" in path` and `Search`, `apply_patch`
        `Patch` from `preview_patch` for a freeform string or `{patch}`, its
        subject the first path `and N more`, `move_path`/`delete_path`
        `Mutation`); web (`web_search` `WebSearch`, `fetch_web_page`
        `WebPage`); the kernel's `compress` (`CoreToolPresenter`, registered
        permanently with the registries: the ranges and topics as subject,
        the receipt then each range's label and summary as `Text`);
        `ArgumentSubjects` in view-image, memory, skills, goal, subagents and
        mcp (resources). The other kernel tools, MCP server tools and guest
        tools take the fallback. Each plugin injects the registry; the
        plugin and facade test harnesses provide it.
      - Kernel: `AgentKernelRegistries::tool_presenters` (provided by the
        kernel plugin). `tool_started`'s existing `presentation.subject` is
        the presenter's subject from the normalized arguments (computed in
        `CallDisposition::presentation`, never in the registration, whose
        presentation feeds the tool generation hash), else the planned
        tool's own (a sourced tool's, `tool discovery`).
        `AgentEventKind::ExecutionProgress { call_id: Option<String>, update
        }`: `SessionToolProgressSink` carries the invocation's call id, the
        queue carries it through coalescing, the application's own
        `report_session_execution_progress` stamps `None`.
        `AgentService::tool_display(session, call_id)` pages the session's
        transcript latest first until it finds the call (and its output),
        then `present_tool_call(namespace, name, arguments, output) ->
        AgentToolCallDisplay { subject, display }`, the arguments normalized
        against the schema of the tool registered under that name now, raw
        otherwise. The facade delegates both and re-exports the display
        types.
      - TUI: `TurnUpdate::ToolCompleted { call_id, is_error }` no longer
        reads history on the UI thread (`tool_result`, `read_into` and
        `TOOL_ITEMS_PAGE` gone); the bridge reads `tool_display` on a
        blocking thread of the Agent's runtime (`replay::read_tool_display`)
        and replies `ToolDisplayed`, always: a drop guard sends `None` when
        the read failed, panicked or never ran (a runtime that shut down),
        and a reply for a session the interface has left is dropped. The
        chat marks the entry completed and keeps it live in
        `awaiting_display` until the reply (also across the turn's end), so
        Scrollback's tail never writes it without its output; a session
        opened in its place clears it. The cell takes the display's subject
        (if any) and reads the display as text (`ToolResultView::from`:
        listing rows, `path:line: text`, `title — url`, `key: value`) until
        T5's views; `compress` folds by name. Replay presents each call with
        its output through `present_tool_call` from the page it read, with
        its subject. `diff_render` takes `AgentPatchPreview`.
      - Deviations: no second top-level `subject` on `tool_started`: the
        existing `presentation.subject` (documented as "a command, a path")
        is filled, so `exec --json` gains one field, `callId`. `display`
        returns `Option` (`None` is the fallback). `Web` is two variants,
        `WebSearch` and `WebPage`; `Mutation` has a typed `kind`, not a
        verb string. `AgentPatchPreview` is a copy of workspace-fs's
        `PatchPreview` in the contracts, converted in plugin-workspace-fs
        (contracts may not depend on workspace-fs; no layer or twin change).
        `present_tool_call` takes `namespace` and `name` as history records
        them. Memory, Skills, goal, subagents, view_image and MCP resources
        present subjects only (their outputs take the fallback's `Fields`),
        MCP server and guest tools the fallback. T4 also shows the text of
        every structured result (read, listing, search, web, fields), which
        showed only a header before.
      - Tests: `contracts/tests/presenters.rs` (registration lifetime, a
        duplicate refused, a panic's fault and fallback, the bounded
        subject, the fallback's keys before notes); each presenter on
        recorded arguments and outputs (`plugin-execution`,
        `plugin-workspace-fs`, `plugin-web` `presenter/tests.rs`,
        `kernel/src/core_tools/tests.rs` for `compress`); the plugin tests
        assert the presenter registered and removed; kernel: a coalesced
        output keeps its call, a sink's progress names its call and the
        application's none (`persistence_contract/progress.rs`); facade
        (`tests/tools/filesystem.rs`): `tool_started`'s subject,
        `tool_display` of a read, an unknown call, the normalization rule,
        an unknown tool's `Fields`; TUI: the entry live between
        `ToolCompleted` and `ToolDisplayed` in both presentations, a display
        arriving after the turn's end, a `None` reply, the display read as
        text, the compress fold, the reply that always comes (a
        `DisplayReply` dropped unread, read, and while unwinding,
        `agent_bridge/replay/tests.rs`), and `app/tests/tools.rs` on vt100 (a
        `read_file` shows `Called read_file: notes.txt` and its text, live
        and resumed, in both presentations).
      - Goldens and snapshots: no request golden changed (nothing
        model-visible; `tool_round_trip_events.json` pins kinds only); no
        history corpus changed; no `.snap` changed.
      - Docs (both languages): `plugin-rules` (a registry row, a new
        `#tool-presenters` heading, the panic and bounds tables, the
        capability scope, the plugins table, the checklist, the guests'
        tools row), `json-events#tools` (`presentation.subject`, `callId`
        with an example, the display not in the stream), `writing-plugins`
        (a guest's tools take the fallback), `embedding` (the facade
        calls), `architecture` (the bridge no longer reads `read_turn_page`),
        `terminal-interface#layout` (the subject, what a call shows),
        `host-plugins` (the registry row), `internals#plugin-host-details`
        (the registry guards its entries). The app tests that assert a
        command's output now wait for it (it may arrive after the turn's
        answer).
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features`; `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`. `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` built but could not prerender locally (port 3000
        held by another project's server); CI's Site run covers it.
        CI on Linux, Windows and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43316467480
        (run on 2684718, which differs from the landed commit only in this
        line; Site: .../runs/43316467447; Linux, Windows and macOS jobs all
        `success`).
      - Hanafubuki at its next pin: `AgentEventKind::ExecutionProgress`
        gains `call_id` (a pattern without `..` or a constructor must add
        it; its JSON gains `callId`); `ExtensionPoint` gains
        `ToolPresenter`; `AgentKernelRegistries` gains `tool_presenters`
        (and the kernel plugin provides `ToolPresenterRegistry`; a host's
        own test harness that mounts a presenting plugin provides it);
        `test_support::MountedPlugin` gains `presenters`. `tool_started`'s
        and the activity's `presentation.subject` now carry a command line or
        path for built-in tools where they were `null`. New:
        `AgentHandle::tool_display`, `present_tool_call`,
        `AgentToolCallDisplay`, `AgentToolDisplay` and its parts; a host
        plugin may register presenters. `compress_call_ranges`,
        `AgentCompressRange`, `preview_patch` and `PatchPreview` stay on the
        facade (the TUI no longer uses them).
      - Module size: no exception added or raised. `notices-policy.toml`
        lists `app/tests/tools.rs` and `agent_bridge/replay/tests.rs` as
        original.
      - For T5/T6: the TUI reads displays as text in
        `ToolResultView::from` (`history_cell/tool.rs`); T5 replaces it with
        a view per display. `FileRead.start_line` is `Some(1)` only from
        offset 0 (D9 would set it always). `write_stdin`'s `Command.command`
        is what was typed (empty for a poll). `execution_progress` carries
        `callId`, but the bridge still drops it (T6): the exec_command's
        call id stays on a background execution's later output, so T6 must
        ignore output for an entry already completed.
- [x] T5: per-tool views (command, patch with line numbers, `read_file`
      with line numbers, listing, search, web, MCP, fallback), exploration
      grouping; `startLine` on `read_file` if D9 is accepted (goldens
      regenerate).
      Done (2026-11-12):
      - D9: `FileReadResult.start_line` (`startLine`, from 1), counted by
        the reader: `lines_before` reads the file's first `offset` bytes
        through the open handle in 64 KiB chunks (cancellable) in place of
        the seek, so both identity checks stay. `Option`, `serde(default)`
        and skipped when absent, so a read recorded before decodes; the
        presenter takes it, else 1 for offset 0 (the old rule).
      - TUI (`history_cell/tool/`, `tool.rs` moved to `tool/mod.rs`):
        `ToolResultView` removed; a `ToolCallCell` holds its calls (one, or
        a run of exploring calls), each with its outcome and its
        `AgentToolDisplay`. A frame (bullet green/red/spinner, title,
        rows under `└`) and a view per display, one module each:
        `command` (`Ran CMD`, the last 3 output lines clipped, `(no
        output)`, `exit 101 · 4.2s` / `still running · 4.2s`; expanded `$
        CMD` highlighted as bash, the whole output, `✓ 4.2s` / `✗ exit N
        · 4.2s` and ` · in CWD`; `write_stdin`: `Interacted with` /
        `Waited for` and the subject), `patch` (`Edited/Added/Deleted path
        (+A -R)` or `Edited N files` with a row per file, `Failed to
        edit` when it did not apply; expanded the diff), `file_read`
        (`Read path (lines a–b[, more follows])`; expanded the text behind
        a right-aligned line-number gutter, highlighted by extension,
        wrapped under the gutter), `listing` (`Listed path (N entries)`;
        directories first, files with sizes), `search` (`Searched "q" in
        path (N matches)`; `path:line: text`, the query bold where it
        matched literally), `web` (`Searched the web for q`, `Fetched
        URL`; results' titles and URLs, the page's title and first 40
        lines), `mutation` (`Moved a → b`, `Deleted x`, no control),
        `text` (running `Running CMD` / `Running TOOL: SUBJECT`; no
        display `Called TOOL` (`Ran CMD` for a command); Text and Fields
        `Called/Failed TOOL: SUBJECT` and the first 3 lines; `compress`
        folded to 1 as before). The control: `+ N lines` for output lines
        (command, read, listing, search, text), `+ Show details`
        otherwise, none when the collapsed rows show it all. Details are
        laid out only expanded (`Extent`), so a collapsed read is never
        highlighted. `diff_render`: `patch_title`, `patch_summary_rows`,
        `patch_diff_lines`, and `push_wrapped_diff_line` takes an optional
        gutter (an added file's rows are numbered from 1).
      - Exploration grouping (`exploration.rs`, Codex's exploration cell):
        `read_file`, `list_directory` and `search_text` explore, decided by
        name at `ToolStarted`. The chat keeps the open run; an exploring
        call joins it, any other entry the chat inserts (message, thinking,
        tool, note) closes it, as do the turn's end and a session opened.
        `Exploring` while a call runs, then `Explored` (`· N failed`),
        a row per call (`Read a, b` for reads in a row, `List src (24
        entries)`, `Search "q" in src (7 matches)`); expanded, each call's
        own view nested; ids are the calls' `tool:` ids, the control always
        `+ Show details`. `running_tools`/`awaiting_display` are by call id
        with their entry; an entry is completed for Scrollback's tail only
        once closed and no call of it runs or waits for its display.
        Replay groups a page's calls the same way.
      - Deviations from §4.6:
        - Grouping: a thinking or a message between two exploring calls
          ends the run (Codex folds reasoning into it), so with a reasoning
          model runs mostly form from the calls of one step. A single
          exploring call shows its own view (`Read …`), a run of two or
          more `Explored`; a run cut by a page boundary on replay is two
          entries; shell `cat`/`rg` commands do not group (Codex parses
          commands; the presenter contract gives no kind at start), so
          membership, like the running header, is by tool name
          (`exec_command`, the three exploring tools, `compress`).
        - Line numbers in a patch only for a file it adds: the patch format
          has no positions and the presenter has only the arguments and the
          committed output (no channel the model does not see;
          `structured_content` is lowered to the model), so an update's
          rows are unnumbered. Numbering them would need the applier's
          positions in the model-visible result (a decision not taken) or
          the presenter reading files that may since have changed.
        - A running command shows the spinner, not live seconds (its time
          comes with its display, `exit 101 · 4.2s`); T6/T7 may add them.
        - No tinted add/delete backgrounds (the colours the port already
          had). Command output has no ANSI colours to keep: it reaches the
          interface as the plain text E3's rendering returns. Search
          highlights only literal matches (the TUI evaluates no regex).
        - `exec_command`'s header before its display reads `Ran CMD`, not
          `Called exec_command: CMD`, so it does not change twice; a
          command line is cut at 200 columns in the header (a subject's
          bound), the cut a detail.
      - Tests: each view collapsed and expanded (`tool/tests/views.rs`
        snapshots: a failed command, one still running, a patch, a patch
        of three files that failed, a read from line 98, a listing, a
        search, the web tools, text and fields), the run
        (`tool/tests/exploration.rs` snapshot, ids, `Exploring` then
        `Explored`), the frame (`tool/tests/mod.rs`: running, no display,
        the command preview and its control, `write_stdin`, compress, a cut
        row, a long command line cut, tabs measured as laid out, a failed
        call's red bullet); `diff_render` (an added file numbered and
        wrapped behind its gutter); chat (`chatwidget/tests/entries.rs`: a run shares an entry
        until another comes and completes only then, two reads completed
        after the turn's end, a replayed run is one entry); the reader
        (`workspace-fs/tests/contracts.rs`: `startLine` at a line's start,
        inside one, at the end, its JSON, an old read decoding); the
        presenter (`startLine`, the offset-0 fallback); app tests on vt100
        (`app/tests/tools.rs`: `Read notes.txt (lines 1–2)` live and
        resumed, its text numbered expanded; the command tests wait for a
        row that is the output alone, since the approval's line names the
        command too).
      - Goldens: no request golden changed, on purpose and checked
        (`cargo test -p hanafubuki-agent --lib golden` without the update
        variable): `startLine` is in `read_file`'s result, which no golden
        request carries (the round trip calls `skills_list`), and
        `read_file` declares no output schema; its description was left
        alone. No history corpus changed.
      - Snapshots: the two old tool snapshots removed (the first-and-last
        excerpt, the diff under `Called apply_patch`); ten new under
        `tool/tests/snapshots/`; `diff_render`'s two lose the `└ Edited`
        header row, which moved to the entry's title, and their 4-column
        indent (the frame adds it); the several-files one numbers the added
        file's row.
      - Docs (both languages): `terminal-interface` (a new `#tools` heading
        with the table of views and the exploration run; `#layout`,
        `#reading`'s controls, `#transcript`), `tools#files` (`startLine`,
        `apply_patch`'s result), `architecture` (the diff's numbers),
        `internals#tui-details` (a Tool calls bullet).
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features`; `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`. `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` built but could not prerender locally (port 3000
        held by another project's server); CI's Site run covers it.
        CI on Linux, Windows and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43325557046
        (run on 20a95b3, which differs from the landed commit only in this
        line; Site: .../runs/43325557129, 213 pages prerendered, `#tools`
        among them; Linux, Windows and macOS jobs all `success`).
      - Hanafubuki at its next pin: `FileReadResult` gains `start_line:
        Option<u64>` (a struct literal must add it; its JSON gains
        `startLine`, which the model now reads in `read_file`'s result).
        Nothing else (the TUI crate only); `AgentToolDisplay` unchanged.
      - Module size: no exception added or raised.
      - For T6/T7: the command view's preview is `output`'s last 3 lines
        (`command.rs`), the place a live buffer feeds; a running call has
        no rows yet (T7's reserved shape). An exploring run stays live
        until another entry comes, so Scrollback's tail keeps it; a live
        `ExecutionOutput` must reach the call by id inside its entry
        (`running_tools` maps a call id to its entry). Rows are measured
        with tabs expanded (`expand_tabs`, the layout's stop of 4), so the
        collapsed preview's 3 rows hold; a live buffer's rows must be too.
- [x] T6: streamed command output: a bounded live buffer, the collapsed last
      3 lines, the expanded stream.
      Done (2026-11-12):
      - Bridge: an `ExecutionProgress` with `call_id: Some(_)` and an
        `ExecutionOutput` becomes `TurnUpdate::ToolOutput { call_id, text,
        truncated }`; every other progress is dropped (the application's own
        reports, a background terminal stopped or recovery, have no call).
      - Chat: the output goes to the call's entry only while the call is in
        `running_tools`, so a background process's later output (its
        `exec_command`'s `callId`, also while a `write_stdin` polls it) and
        anything after `ToolCompleted` are ignored without raising the
        revision. No timer or thread: the
        `LiveCell` update raises the revision and the live entries' frame
        tick (`LIVE_TICK`) draws it, so output is coalesced to frames.
      - Buffer (`history_cell/tool/live_output.rs`, after Codex's
        `LiveCommandOutput`, the kernel's `append_output` as reference): the
        line still being written is kept raw and shown as its
        `complete_prefix_len` prefix rendered by `render_plain_text`, so a
        `\r` redraw, a CR LF or an escape split between updates, and the
        monitor's `\r ESC[2K` before a PTY row sent again replace that line
        rather than append; ended lines are rendered once, tabs expanded
        (`expand_tabs`, so the 3-row preview bound holds). Bounds: the first
        200 and the newest 200 lines with `… +N lines` between, a line cut at
        1 KiB with `…`, the raw unfinished line collapsed to what it shows
        past 16 KiB; `truncated` (output skipped) ends the line before it and
        adds a `…` row. It receives decoded text (E2/E3) and only renders
        it: nothing is decoded again on the way, so PowerShell 5.1 output
        reaches it as the execution plugin left it.
      - Views (`command.rs`, `mod.rs`): `command::streamed` lays the live
        lines out with the same row builders as the finished view
        (`last_rows`, `script_rows`, `output_rows`, `hidden`): collapsed the
        last 3 lines (trailing blank lines trimmed, as `text_lines` does),
        `+ N lines` past them; expanded `$ SUBJECT` and the lines kept. The
        running header is `Running CMD · 3.4s` (§4.6's live seconds, tenths;
        the entry's animation tick is 100 ms for a running `exec_command`,
        600 ms for another tool). After `ToolCompleted` the entry keeps the
        live rows under `Ran CMD` until the display arrives, then the
        display's view takes over (the same last rows for the same output)
        and the buffer is dropped; with no display (`None` reply) the live
        rows stay. Both presentations through the existing live entry:
        Scrollback's tail never writes it while it runs. A call the turn's
        end overtakes (interrupted, failed) is marked failed
        (`ToolCallCell::end_running`): `Ran CMD`, a red bullet, its live
        rows, no tick; it was left `Running` with its spinner going, which
        the clock would have made a growing time.
      - Facade (additive): re-exports `AgentToolProgressUpdate`,
        `render_plain_text` and `complete_prefix_len`.
      - Deviations: §4.6's "everything up to 1 MiB, then the first and last
        200 lines" became always the first and newest 200 lines (each at
        most 1 KiB): every revision and clock tick lays the whole entry out
        again on the UI thread, so an expanded 1 MiB stream would be wrapped
        about ten times a second; the finished display (the model's bound,
        64 KiB by default, 256 KiB at most) replaces it anyway. The live
        rows stay through `awaiting_display` (T5 showed the header alone
        there, which would flicker). The clock counts from `ToolStarted`, so
        it includes an approval's wait. Known limits of the monitor's PTY
        preview, not changed: it marks only the last row rewritten, so a
        repaint of rows above it arrives again as new lines, and an
        unfinished line that wrapped is replaced by its last screen row
        until it ends (both also reach the kernel's activity preview).
      - Tests: the buffer (`live_output/tests.rs`: lines committed and the
        unfinished one, `\r` redraws and a split CR LF, a PTY row written
        again, a split escape, tabs, the gap, the head/tail bound and its
        count, a cut line, an endless progress bar and an endless line
        bounded); the entry (`tool/tests/streamed.rs`: the last 3 lines and
        `+ N lines`, the expanded stream, a redraw in place, the rows kept
        through completion and replaced by the display's, kept without one,
        another call's output ignored, the clock in tenths); the chat
        (`chatwidget/tests/entries.rs`: output streams in place in both
        presentations, nothing for an unknown call or after completion, a
        command the turn's end overtakes stops its clock and keeps its
        output; two tests that pinned `Running` after an interrupt now read
        `Ran`);
        on vt100 (`app/tests/tools.rs`): a real command that prints, then
        waits for a file the test creates (POSIX shell or PowerShell),
        shows its first line under `Running` before it finished and both
        lines under `Ran` after, in both presentations. Three Scrollback
        tail tests used a running `exec_command` without output as a
        fixture; they use `web_search` now, whose header has no clock.
      - Goldens and snapshots: none changed (nothing model-visible; the
        running header has no snapshot, its time being live).
      - Docs (both languages): `terminal-interface#tools` (the row and a
        paragraph on live output), `tools#commands`, `internals#tui-details`
        (a Streamed output bullet), `embedding` (the re-exports).
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features`; `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`. `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` built but could not prerender locally (port 3000
        held by another project's server); CI's Site run covers it.
        CI on Linux, Windows and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43335634496
        (run on 79aaf1f, which differs from the landed tree only in this
        line; Site: .../runs/43335634512, 214 pages prerendered; Linux,
        Windows and macOS jobs all `success`). The feature without the
        turn-end fix ran green before too
        (https://github.com/licyk/hanafubuki-agent/actions/runs/43331933917).
        Windows ran the streaming app test under the shell `resolve_shell`
        picks on `windows-latest` (`pwsh` first); no T6 test runs Windows
        PowerShell 5.1 itself.
      - Hanafubuki at its next pin: nothing to change; the facade gains
        three re-exports (`AgentToolProgressUpdate`, `render_plain_text`,
        `complete_prefix_len`). The TUI crate only otherwise; no twin
        touched.
      - Module size: no exception added or raised.
      - For T7: a running command's header now ends with its clock (`·
        3.4s`, then `· 1m 02s`), which can change where a long header wraps
        and so its row count while it runs. A running command has no rows
        until its first output, then 1 to 3 (`ToolCall::view`'s header path
        and `command::streamed`), where T7's reserved `(running…)` row goes;
        at the display's takeover the same last rows stay and the finished
        view adds `exit N`, `still running` or `(no output)`. The tick is
        100 ms for a running `exec_command`.
- [x] T7: stability: running entries reserve their collapsed shape, the
      toggle keeps the entry's header row, Scrollback's live region keeps its
      height; frame-by-frame tests.
      Done (2026-11-13):
      - Reserved row (`history_cell/tool/mod.rs`, `ToolCall::view`): from
        its start until what it shows arrives, a call whose view has no
        rows gets a dim placeholder under its header, `(running…)` while an
        `exec_command` runs, `…` otherwise (another tool, and a finished
        call waiting for its display), in the collapsed and the expanded
        rows. The first output (`command::streamed`) or the display takes
        its place. `Outcome` gains `shown` (its display arrived, or there
        will be none): `show` sets it, the chat now calls `show` with an
        empty display for a `None` reply, `end_running` sets it for what it
        ends, and a session opened in place settles the calls still waiting
        for a display (`on_session_opened`), so no `…` stays for good.
      - The clock (`frame`): a running command's ` · 3.4s` is no longer
        part of the wrapped title; it follows the header's last row only
        when its widest (`CLOCK_COLUMNS` = 14, ` · 99h 59m 59s`) fits there
        and is left out otherwise, so whether and where it shows never
        depends on the time (`ToolCall::clock`; `text::running` lost it).
      - Toggle anchor (`transcript_view/disclosure.rs`,
        `disclosure_anchor`): when the top row of the view belongs to the
        toggled entry and is at or above its header (a separator, a
        prompt's padding, the header), the top row's text is kept, since
        both forms share the rows up to the header; it used to restart the
        entry at its header, so its header jumped a row. An entry below the
        top already kept its header (the top anchor), so Fullscreen, the
        Ctrl-T transcript and Ctrl-O now keep a visible header on its row.
      - Pending-input preview (`bottom_pane/pending_input_preview.rs`,
        `ReservedPendingInput`): while a turn runs it keeps the rows (its
        blank row included) it reached at the current width, blank once the
        turn has read the messages, reset at the turn's end
        (`BottomPane::pending_rows`).
      - Scrollback: no new mechanism; T1b's floor keeps the viewport's
        height, and with the reserved shape a call no longer grows it
        between its start, its clock, its completion and its display when
        its rows stay the same (asserted: the viewport area is unchanged).
      - Deviations from §4.7: the pending preview keeps the rows it
        reached in the turn rather than "one reserved row" (one row cannot
        hold a header, a message and the blank after them; what remains is
        growth past the most it reached). A clock without room is left out
        rather than moved to a row of its own, which would have been a row
        the finished header drops. Remaining jumps, bounded and recorded:
        `Running` becoming `Ran` takes a row off a header whose `Running`
        form overflows its last row by at most four columns, which its
        `Ran` form fits (measured: a 61-column `Running` header at widths
        58–61); a display with no collapsed rows (a read, a listing, a
        search, the web tools, a move, a delete, `write_stdin` without
        output) drops the placeholder's row; a display with more adds the
        rest of its rows and its control: two and the control for most
        (§6's bound), three for a failed or backgrounded command whose
        output did not stream (its exit row too), a row per file for a
        patch of several files; a single exploring call joined by a second
        becomes `Exploring` (the run's own shape, not reserved). In
        Scrollback an expansion past the floor grows the viewport upward,
        so the entry's header moves up by the rows gained (the inline
        viewport cannot grow downward); collapsing keeps it (T1b).
      - Not covered by a test: §6's "Scrollback's viewport not cleared
        when a second tool starts" (no second-tool test; the viewport's
        area is asserted unchanged across one call's frames), and the
        Scrollback expansion that moves the header up.
      - Tests: `history_cell/tool/tests/shape.rs` (a command's
        `(running…)` → `…` → `(no output)`, its first line in the reserved
        row; another tool's `…` replaced by its result, header alone without
        a display, `end_running`; the clock never changing the header's
        rows or their width at 20–90 columns across 0.9s, 9.95s, 10.05s,
        59.95s, 62s and 125s; no clock without room);
        `transcript_view/disclosure/tests.rs` (a toggle keeps the header on
        its row, with the separator as the top row: failed before the
        fix); `bottom_pane/tests.rs` (a read steer's rows kept, the next
        message in them, given back at the turn's end); the new
        `app/tests/stability.rs` on vt100 in both presentations: a running
        command whose header fills 93 of 100 columns through 9.95s, 10.05s,
        61s, its output, completion, display and `LiveEntryCompleted`: in
        every frame the entry and the status row are there, its header row
        and row count are the same, no row above it moves, and Scrollback's
        viewport area is unchanged (fails at 10.05s with the old clock);
        a command (3 lines, 2 more, completion, display) and an MCP tool
        (completion, a 5-line display) grow by at most the control and two
        rows per step, the rows above moving by exactly that while
        following, and not at all while reading (Fullscreen, scrolled up).
        Updated: the tests that read a running or waiting call's rows
        (`chatwidget/tests/entries.rs` ×3, `app/scrollback/tests.rs` ×2: a
        running call written at a session's opening or the exit now ends
        with `└ …`), and three that completed a call without its display
        now show none (`tool/tests/mod.rs`, `exploration.rs`).
      - Snapshots and goldens: none changed (the exploration snapshot's
        test now settles its failed call with `show`, so its content is
        the same). Nothing model-visible.
      - Docs (both languages): `terminal-interface#layout` (the waiting
        messages' rows), `#reading` (a toggle keeps a visible header; the
        tail's expansion past its blank rows), `#tools` (the reserved row,
        the clock left out without room); `internals#tui-details` (a new
        "A stable layout" bullet).
      - Validation: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`; `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`; `cargo
        xtask check`; `cargo test --workspace --all-features`; `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`. `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` built but could not prerender locally (port 3000
        held by another project's server); CI's Site run covers it.
        CI on Linux, Windows and macOS: https://github.com/licyk/hanafubuki-agent/actions/runs/43343045698
        (run on f8f8ec2, which differs from the landed tree only in this
        entry and in the reworded remaining-jumps sentences of
        `terminal-interface#tools` and `internals#tui-details`, both
        languages; Site: .../runs/43343045815, success). The first Windows
        attempt failed only in the known timing flakes
        (`plugins::subagents::tests::the_depth_and_running_limits_…`,
        `plugins::review::tests::a_review_reads_only_…`); its failed job was
        rerun; Linux and macOS passed at once, the new `shape` and
        `stability` tests among them on every platform.
      - Hanafubuki at its next pin: nothing (the TUI crate only; no facade
        change, no twin touched).
      - Module size: no exception added or raised. `notices-policy.toml`
        lists `history_cell/tool/tests/shape.rs` and
        `app/tests/stability.rs` as original.

**`exec_command` ([7])**

- [x] E1: failing tests, no fix: Windows PowerShell 5.1 (`powershell.exe`
      named explicitly) on `windows-latest`, in pipe and PTY mode: UTF-8
      and ANSI-code-page output mixed in one stream, `cmd /c echo` captured
      and not, `'中文' | findstr .`, `Write-Progress` under ConPTY, stderr
      without CLIXML, no BOM, `Read-Host` through `write_stdin`; the code page
      936 decoder cases; CJK with cursor moves, a read inside an escape, the
      U+FFFD hold-back, `DaFilter` at end of file, silent loss past 64 KiB.
      Record which of the plan's (a)–(g) reproduce.
      Done (2026-11-12):
      - Windows (`execution/tests/contracts/windows_powershell.rs`,
        `#[cfg(windows)]`): `exec_command`'s real path (`start_exec_command`,
        `wait_unified_execution`, `write_stdin`) over the real
        `ExecutorManager`, with the request's `PATH` limited to System32 and
        `WindowsPowerShell\v1.0` so the shell resolves to 5.1 (asserted, and
        `$PSVersionTable` prints 5). Reproduced on `windows-latest`, twice
        (https://github.com/licyk/hanafubuki-agent/actions/runs/43160853306,
        https://github.com/licyk/hanafubuki-agent/actions/runs/43161569893),
        and landed `#[ignore]`d:
        - (a) ANSI-code-page bytes written to the raw handle become U+FFFD,
          alone and after PowerShell's UTF-8 in one stream, in pipe and PTY
          mode; the Cmd shell's built-ins (`echo café & dir /b`) in pipe mode
          too (code page 437 here, 936 in the reported `dir`).
        - (c) `'中文' | findstr .` prints `??` (`$OutputEncoding` is 20127,
          ASCII), pipe and PTY.
        - (d) `[Console]::ReadLine()` answered with `中文 ü` through
          `write_stdin` reads `?? ü` (`[Console]::InputEncoding` 437), and
          `chcp` shows 437: the prefix leaves the console's input code page.
        - (e) `Write-Progress` under ConPTY leaves `Copying 0% [ ]` and every
          later frame in the output; in pipe mode it leaves nothing.
      - Did not reproduce on the runner, and stay un-ignored to pin it:
        - The prefix does set the console's *output* code page: `cmd /c echo
          中文` keeps 中文 in pipe and PTY mode, which the plan's §4.9 step 2
          assumed; so does (b), `$x = cmd /c echo 中文; $x`.
        - (f) no BOM, although `[Console]::OutputEncoding` has a 3-byte
          preamble; (g) no CLIXML, neither for a failing cmdlet's stderr nor
          for progress in pipe mode.
        - `Write-Output '中文 ü'` is whole in both modes; the Cmd shell's
          built-ins under ConPTY are whole; 300 numbered lines arrive whole
          from cmd and PowerShell, pipe and PTY, with the default bounds.
      - Platform-independent, all reproduced and `#[ignore]`d for E3: CJK
        with CSI G and with CR + CSI C (`foundation/src/terminal_text/
        tests.rs`); a read starting inside `ESC[31m` at ring trimming
        (`process/src/executor/output/tests.rs`), the monitor's skip and
        `bound_execution_output` (`plugin-execution/src/monitor/tests.rs`;
        the skip through the fake port, since the plugin may not depend on
        the process crate) and the kernel's merge
        (`kernel/src/service/progress/tests.rs`); a title (OSC) holding
        `\xff` cut by a poll, whose rest printed `tle` on the next one, and
        100 KiB after the offset returning 64 KiB with `truncated: false`
        (`execution/tests/contracts/output.rs`, Unix, real processes);
        `DaFilter` dropping a final `ESC[ 99%` at end of file
        (`process/src/executor/tests.rs`, Unix PTY).
      - Decoder cases for E2 (`process/src/executor/output/tests.rs`),
        through a test helper `decode_stream(reads, code_page)` that runs
        today's `Utf8Carry` and lossy UTF-8 and that E2 makes its decoder:
        GBK `中文`, UTF-8 and GBK in one stream, trail bytes in the ASCII
        range across reads (`錯誤`, `乗`), CP866 `Привет`, and the reported
        `dir` listing's bytes (`驱动器 D 中的卷没有标签。`, `屏幕截图 …`,
        `270 个文件 … 字节`), which today decode exactly to the garbling
        reported. UTF-8 staying UTF-8 under any fallback passes and pins it.
      - The reported mid-listing loss (`dir` of 270 files showing ~45 lines,
        `..` followed by a fragment of a later line) reproduces in
        `unified.rs`'s offsets, not in the TUI: the buffer's cap is the
        model's `max_output_bytes` (`shell.rs`: `output_bytes_cap =
        max_output_bytes.unwrap_or(256 KiB)`). The head arrives before the
        first poll's yield, the rest of a slow listing after it, and by the
        next poll the buffer has dropped the bytes after the offset; that
        read starts at the oldest retained byte, mid-line, with `truncated`
        set but no marker in `output`. Reproduced on Linux and on
        `windows-latest` (cmd, pipe); also a finished command over
        `max_output_bytes` returns only its tail, unmarked.
      - Not reproducible as asked: `Read-Host` under `-NonInteractive`
        (which `exec_command` passes) is refused ("Windows PowerShell is in
        NonInteractive mode"), so (d) is tested with `[Console]::ReadLine()`
        and the `Read-Host` test was dropped.
      - Seen on the runner: the first cmdlet of a 5.1 command (module
        autoload) took ~40 s in the clean environment and ~23 s with D17's
        variables; 5.1 reads the machine `PSModulePath` (here with Az)
        whatever the environment says.
      - No production change and no test seam. No golden changed. Nothing
        for Hanafubuki. No module-size exception
        (`execution/tests/contracts/windows_powershell.rs` is the largest
        new file, under 900).
      - Validation (Linux): `cargo fmt --all --check`, `RUSTFLAGS="-D
        warnings" cargo check --workspace --all-targets --all-features`,
        `cargo clippy --workspace --all-targets --all-features -- -D
        warnings`, `cargo xtask check`, `cargo test --workspace
        --all-features`; every ignored test run with `--include-ignored` and
        seen failing for its named cause; `cargo clippy --target
        x86_64-pc-windows-gnu -p hanafubuki-agent-execution --all-features
        --tests`. The final tree ran the whole CI workflow on Linux, Windows
        and macOS, green
        (https://github.com/licyk/hanafubuki-agent/actions/runs/43162060854).
- [x] E2: the complete PowerShell prefix (UTF-8 without BOM for
      `[Console]::OutputEncoding`, `InputEncoding` and `$OutputEncoding`;
      `$ProgressPreference = 'SilentlyContinue'`) and `-OutputFormat Text`
      (`agent-shell-v5`); decoding by run of non-ASCII bytes, the ANSI code
      page as fallback (D15), before the ring buffer; `chcp 65001` for
      `cmd.exe`; `PYTHONUTF8` / `PYTHONIOENCODING`; D17's environment.
      E1's findings for E2: the prefix already sets the console's output
      code page (step 2's assumption holds), so its new parts are
      `InputEncoding` (chcp 437 today), `$OutputEncoding` (ASCII today) and
      `$ProgressPreference`. Raw ANSI-code-page bytes under ConPTY are
      decoded by conhost before the PTY, so the run decoder does not reach
      them (that PTY test stays ignored). D15's premise that ANSI and OEM
      pages differ only for Western accents does not hold for Cyrillic (1251
      against 866) or Greek (1253 against 737): a console-page program there
      relies on `chcp 65001`. The first cmdlet's module autoload is slow in
      the clean environment (~40 s on the runner, ~23 s with D17's list).
      Done (2026-11-12):
      - The PowerShell prefix (`execution/src/shell.rs`,
        `POWERSHELL_UTF8_OUTPUT_PREFIX`): `UTF8Encoding($false)` for
        `$OutputEncoding` (first, so a console setter that throws cannot
        skip it), `[Console]::OutputEncoding` and `InputEncoding`, then
        `$ProgressPreference = 'SilentlyContinue'`; `agent-shell-v5`, so a
        remembered v4 approval is asked again once. `-OutputFormat Text` was
        not added: E1 found no CLIXML, and the review's read-only matcher
        (`agent/src/plugins/review/read_only.rs`) pins the argv's shape.
      - Cmd scripts start with `chcp 65001 >NUL & `; `PYTHONUTF8=1` and
        `PYTHONIOENCODING=utf-8` joined `NON_INTERACTIVE_ENVIRONMENT`; D17's
        thirteen system variables joined the Windows allowlist.
      - Decoding by run (`process/src/executor/output/decode.rs`,
        `RunDecoder`, replacing `Utf8Carry`; `output.rs` became
        `output/mod.rs`): ASCII passes, a maximal non-ASCII run that is
        UTF-8 stays, any other is decoded with the fallback (`encoding_rs`,
        a code-page table for 866, 874, 932, 936, 949, 950, 1250–1258),
        taking the ASCII byte after it when that byte completes its last
        character (a GBK trail) and never when it would be swallowed. A run
        at a read's end is held; the decoders live in the `OutputBuffer`,
        under its lock, so a run no read continued for 100 ms is stored
        when the offset is next observed (`read_executor_output`,
        `ExecutorSession::info`), since the reader thread is blocked in
        `read`. The fallback is `GetACP` for pipes on Windows (D15); PTY
        output and every platform but Windows get none, which keeps
        today's behaviour exactly (an unfinished scalar held, malformed
        bytes passed), as ConPTY writes UTF-8. `encoding_rs` is declared in
        the process crate, its only user, as the root manifest keeps
        third-party crates more than one member uses.
      - Un-ignored, green on `windows-latest`: the two `chcp` input code page
        tests, `pipe_ansi_code_page_output_…`,
        `pipe_utf8_and_ansi_code_page_output_mix_…`, both
        `…_input_piped_to_a_native_program_keeps_cjk`,
        `pty_progress_leaves_no_trace` (the preference alone fixes it),
        `pty_console_input_receives_non_ascii_text`; and the five decoder
        cases (GBK, mixed, ASCII trail bytes, CP866, the reported `dir`
        listing). New: the Cmd shell runs in 65001 (pipe, PTY), the system
        and Python variables reach a command, a UTF-8 run keeps the ASCII
        byte after it, an unfinished GB18030 sequence keeps it too, a held
        run is stored after the hold, one decoder per stream.
      - Still ignored: both `pty_…ansi…` cases (conhost decodes raw bytes
        with the console's code page, UTF-8, before ConPTY: no decoder of
        ours sees them); E3's cases, unchanged.
      - Found on the runner, re-ignored and open:
        `pipe_cmd_built_ins_write_text_the_decoder_reads`. `cmd.exe` keeps
        the code page it started with (OEM, 437 there) for its own
        built-ins' pipe output; `chcp 65001` changes it only for the
        programs it starts afterwards (probe:
        https://github.com/licyk/hanafubuki-agent/actions/runs/43167771554,
        `echo café` after `chcp` in one line, in `( )`, `call`, `if` and
        `for` all wrote `caf\x82`; only a nested `cmd /c` wrote UTF-8; failing
        run https://github.com/licyk/hanafubuki-agent/actions/runs/43165837630).
        The reported case is fixed regardless: on Chinese Windows OEM and
        ANSI are both 936, and the decoder reads the listing (its unit test).
        Elsewhere (437/850 against 1252, 866 against 1251) a built-in's
        non-ASCII text on a pipe is misread. Two fixes, each needing a
        decision: run the script in a second `cmd.exe` started after `chcp`
        (passing it through a variable and delayed expansion, so the outer
        parser never splits it; changes how every Cmd command is parsed and
        quoted, and adds a variable to its environment), or read Cmd pipes
        with the OEM code page (`GetOEMCP`) instead of D15's ANSI one, which
        needs the shell passed to the executor.
      - Goldens: none changed (the prefix and the environment are not
        model-visible). Hanafubuki: `POWERSHELL_UTF8_OUTPUT_PREFIX`, which it
        can read, changed value; an approval it stored for a v4 command is
        asked again. No module-size exception.
      - Validation (Linux): `cargo fmt --all --check`, `RUSTFLAGS="-D
        warnings" cargo check --workspace --all-targets --all-features`,
        `cargo clippy --workspace --all-targets --all-features -- -D
        warnings`, `cargo xtask check`, `cargo test --workspace
        --all-features`; `cargo clippy --target x86_64-pc-windows-gnu -p
        hanafubuki-agent-process -p hanafubuki-agent-execution
        --all-features --tests -- -D warnings`; the site's `bun install
        --frozen-lockfile`, `bun run format:check:site`, `bun run
        site:check`, `bun run site:test`, `VITE_BASE_PATH=/hanafubuki-agent/
        bun run site:build` (all 35 pages prerendered). The final tree ran
        the whole CI workflow on Linux, Windows and macOS, green
        (https://github.com/licyk/hanafubuki-agent/actions/runs/43168232034).
- [x] E3: PTY output through a `vt100` screen model; width-aware pipe
      rendering; sequence-aligned read starts, the hold-back always,
      `DaFilter::finish`, head and tail with a marker (model-visible:
      goldens regenerate on purpose).
      E1's findings for E3: step 9's marker must also cover a poll whose
      offset the buffer has already dropped (the reported `dir`): that read
      starts mid-line at the oldest byte with no marker, because the buffer's
      cap is the model's `max_output_bytes`; and a finished command over
      `max_output_bytes` returns its tail alone.
      Done (2026-11-12):
      - The reported loss (`dir` of 270 files, ~45 lines, two glued): an
        `exec_command` session's buffer keeps a fixed 4 MiB
        (`EXEC_RETAINED_OUTPUT_BYTES`, `resolver.rs`), no longer the model's
        `max_output_bytes`. A poll reads everything it has not read
        (`execution/src/output_window.rs`): within the bound it returns it;
        past it (running or finished) the head and the tail, each cut at a
        line break (`end_point`, `aligned_start`; a character or sequence
        boundary when none is within 1 KiB), with `[… N bytes omitted …]`
        on a line between them, `truncated`, and the offset moved to the
        end; a poll whose start the buffer dropped gets the marker (N counts
        from its offset) and the newest output. Decision: one rule for
        running and finished polls, so a chatty process never leaves the
        model megabytes behind. The shell plan's `output_bytes_cap` (hashed
        in the approval) is now that constant; `agent-shell-v5` is
        unreleased, so the version stays.
      - PTY output through a `TerminalScreen` (`execution/src/
        terminal_screen.rs`, `vt100` 0.16 now a production dependency of
        the execution crate), one per PTY execution for the polls (in its
        record) and one in the monitor for the preview, fed at the PTY's
        sizes (`AgentExecutionOutput` now carries `terminal_size` and
        `resizes`). A take returns the rows scrolled off since the last,
        then the screen's rows from the first that changed; a row left
        unfinished is returned again whole (`rewrites_last_line`, which the
        monitor turns into `\r ESC[2K` for the preview). `vt100` drops
        scrollback past its bound silently, so output goes in 128-byte
        slices, new scrollback rows are collected after each, and at 256 rows
        the screen's state is replayed into a fresh parser, only between
        sequences; an unfinished trailing sequence is held for the next push.
        A resize unwraps rows (vt100 does not reflow), as a terminal does.
        Known limits, as on a terminal: rows scrolled inside a scroll region
        a program set never reach the scrollback; a wide character that finds
        one column left moves to the next row without marking the row
        wrapped (vt100 follows tmux), so that line gains a break (rejoining
        it by heuristic could glue genuine lines). The scrollback is not
        emptied while the alternate screen is up.
      - `exec --json`'s `execution_output` `text` for a PTY is the screen's
        rows (documented in `json-events`, both languages). Not changed:
        a background terminal's recent lines (`recent_output_lines`) still
        render PTY output with `render_plain_text`; the plan names only the
        model and the preview.
      - `render_plain_text` is width-aware (`unicode-width`, now a
        foundation dependency): a wide character takes two cells,
        overwriting half of one blanks the other, zero-width characters stay
        with the one before. `complete_prefix_len` works on bytes, so the
        pipe hold-back in `unified.rs` no longer depends on the tail being
        free of U+FFFD. `aligned_start` / `resume_point` (foundation) align
        the ring's read after trimming (`read_from`, only when `from` predates
        the window), the monitor's skip, `bound_execution_output` and the
        kernel's merge; skipped bytes count in `truncated`.
        `DaFilter::finish` passes held bytes on at end of file, before the
        decoder's, for every exit reason of the PTY reader.
      - Un-ignored: both CJK `terminal_text` cases, the ring, monitor (2) and
        kernel sequence starts, `DaFilter` at end of file, the four
        `contracts/output.rs` cases, and the three Windows marker cases
        (`pipe_`/`pty_a_listing_over_the_output_bound_…`,
        `pipe_a_slow_listing_…`). New: strict listings (every line a whole
        numbered line or the marker, numbers jumping only across it: slow
        and fast, pipe and PTY, over the bound and within the default), a
        poll more than 4 MiB behind whose marker counts exactly the bytes
        missed, a PTY prompt and its answer across two polls, the monitor's
        PTY preview of a repaint, window and screen unit tests, `DaFilter`,
        alignment and width cases. Still ignored, not E3's: the two
        `pty_…ansi…` cases and `pipe_cmd_built_ins_write_text_the_decoder_
        reads` (E2's findings). The fake PTY test feeds CR LF now, as a PTY
        (`onlcr`) does: a bare LF moves down without returning, on a screen.
      - Goldens: none changed. They hold `exec_command`'s schema and
        description, not its output, and neither changed; the TUI snapshots
        pass unchanged.
      - Found on CI: macOS `seq` prints `1e+06`; the 4 MiB test generates
        with `awk`. The Windows listings now use the strict check (no line
        glued, repeated or left out without the marker).
      - Hanafubuki: twins to port by hand: `terminal_text` (width,
        `complete_prefix_len(&[u8])`, `aligned_start`, `resume_point`,
        `end_point`, `unicode-width`), `da_filter` (`finish`),
        `terminal_size` (`TerminalResize: Deserialize`), `executor` (the
        PTY reader's flush, `read_from`'s aligned start).
        `AgentExecutionOutput` gained two fields; an exec approval it stored
        is asked again (the hashed `output_bytes_cap` changed). No
        module-size exception.
      - Validation (Linux): `cargo fmt --all --check`, `RUSTFLAGS="-D
        warnings" cargo check --workspace --all-targets --all-features`,
        `cargo clippy --workspace --all-targets --all-features -- -D
        warnings`, `cargo xtask check`, `cargo test --workspace
        --all-features`; `cargo clippy --target x86_64-pc-windows-gnu -p
        hanafubuki-agent-foundation -p hanafubuki-agent-process -p
        hanafubuki-agent-execution -p hanafubuki-agent-plugin-execution -p
        hanafubuki-agent-kernel --all-features --tests -- -D warnings`; the
        site's `bun install --frozen-lockfile`, `bun run format:check:site`,
        `bun run site:check`, `bun run site:test`,
        `VITE_BASE_PATH=/hanafubuki-agent/ bun run site:build` (all 35 pages
        prerendered). The final tree ran the whole CI workflow on Linux,
        Windows and macOS, green
        (https://github.com/licyk/hanafubuki-agent/actions/runs/43176826154);
        the first probe failed on macOS only, on `seq`'s format
        (https://github.com/licyk/hanafubuki-agent/actions/runs/43175255432).

## Plan: model metadata from the Provider, with fallbacks (2026-11-11)

The plan is [docs/plan-model-metadata.md](docs/plan-model-metadata.md), after
Hoshimi `83f7813`. Every §9 decision (D1–D7) was accepted as recommended
(2026-11-11). All milestones (M1–M8) are done; the open items below stay.

- M1 (done): the listing is tolerant per entry and reads the §3 fields, and
  the §4.1 types exist (unused until M2).
  - `model_catalog.rs` became `model_catalog/`: `mod.rs` (the client and its
    paging), `entry.rs` (one entry), `shapes.rs` (the field readers per
    listing shape). `ModelCatalogClient::list` returns `ModelCatalogListing
    { entries, skipped }`: an entry without a usable id is skipped and
    counted (logged at warn, the count only); any other field it cannot read
    is unknown, an effort that does not parse is left out, and efforts past
    32 are cut. Only a page without a `data` array or over a bound fails; an
    Anthropic page with `has_more` and no `last_id` now ends the listing.
  - `ModelCatalogEntry` gained `max_output_tokens`, `image_input` and
    `reasoning`. A capability is yes when any of its fields says yes, else no
    when any says no, else unknown. The kernel still records windows only.
  - `ModelMetadata`, `ModelReasoningSupport`, `Sourced<T>` and
    `ModelMetadataSource` are in `model_metadata.rs`; a fact that does not
    decode is unknown, and `Sourced`'s `value` and `source` have no default.
  - Deviation: the fixtures are the providers' documented example responses
    (no credentials to record real ones), cited in the suite, in
    `crates/model-api/tests/model_listing/fixtures/`, since a `tests/fixtures/`
    directory fails `check-source-layout`.
  - Deviation: fields from Hoshimi that no documented example confirms are
    decoded too, which §7.3 said to leave out: `contextWindow`,
    `max_output_tokens`, `max_completion_tokens`, `maxTokens`,
    `input_modalities`, `input`, `vision`, `capabilities.vision`, a boolean
    `reasoning`, `capabilities.reasoning`. `max_model_len`, `context_window`
    and Codex's reasoning levels were read before M1 and stay unconfirmed too.
  - Deviation: Anthropic's `/v1/models` documents `max_input_tokens`,
    `max_tokens` and `capabilities` (settling §3's "Unverified"), so
    `max_tokens` (max output), `capabilities.image_input.supported` and
    `capabilities.thinking.supported` are read beyond §3's table. Anthropic's
    `capabilities.effort.<level>` and OpenRouter's `reasoning.supported_efforts`
    and `default_effort` are documented but left for M6.
- M2 (done): `kernel::ModelMetadataRegistry` (`kernel/src/model_metadata/`)
  replaces `ModelCatalog` and the image capability cache, and the facade keeps
  it in `model-metadata.json` through the kernel's `ModelMetadataStore` port.
  - Keyed by `ModelRouteKey`, now in model-api's `route.rs` with
    `ModelEndpointKey` (protocol and endpoint: a listing's scope), both serde.
    Deviation: the endpoint is the SHA-256 of the canonical base URL, not the
    URL, since the key is persisted and a base URL's query may hold a
    credential (model-api gains `sha2`).
  - Per route: reported, learned and catalog facts, each an M1
    `ModelMetadata`; resolved per fact as §4.2 (`resolve`), with the
    configured window and image input passed in (`ConfiguredModelFacts`). A
    learned fact only in the safe direction: a window while smaller than the
    others' (or theirs is unknown), a "no" over a reported "yes"; a learned
    "yes" is ignored. Catalog facts are resolved but nothing writes them.
  - `record_listing` merges per model: a listed model takes its entry whole
    (window, max output, image input, reasoning with efforts and default),
    unknowns included; a model left out keeps its reported facts for 7 days
    after it was last listed, a model the latest listing names keeps them
    with no expiry. The shrink guard counts, as Hoshimi's does: a listing
    with fewer than half the models of the endpoint's last recorded one is
    held once (the hold is persisted), the next is recorded whatever it
    says; as many other models (a local server restarted with another one)
    are recorded at once. Residual: a real drop to under half (3 models,
    then 1 new one) holds the new model's facts for one listing.
    Learned facts expire after 7 days. Bounds: 4096 routes, 32 endpoints'
    listings, the oldest observed dropped, never the route just learned.
  - The store: `load` once when the kernel is built, `save(snapshot)` after
    each change, handed over under the registry's lock so in order. The
    facade's `FileModelMetadataStore` passes it to a writer in the Agent's
    worker group, which writes the newest snapshot waiting (config's
    `write_atomic`) and the last one at shutdown; `{"version": 1, ...}`, 16
    MiB at most; a file that does not read is quarantined (`quarantine_file`)
    and the run starts empty. A run that records nothing writes nothing. The
    kernel's other constructors keep a `NoopModelMetadataStore`.
    `AgentService::without_default_model_reporting_faults` takes the store,
    `model_context_window` takes the protocol.
  - Behaviour changes: the image refusal is learned for 7 days, not 6 h, and
    persisted, so it holds across restarts (planned for M5, landed here with
    the fold); sessions still read only the learned image fact (`Unknown`
    otherwise) until M5. A listing's facts hold across restarts, and a model
    a new listing leaves out keeps its window for 7 days (it lost it at once
    before). A draft's listing (`test_provider_draft`) is now written to the
    file too, as it was recorded in memory before.
- M3 (done): the facade discovers models in the background, and a session
  takes its window again as each turn starts.
  - Config: `[modelMetadata] discovery = "auto" | "off"` (default `auto`)
    in `config.toml` (`AgentSettings.model_metadata`), not `providers.toml`,
    so `config set`, `-c`, profiles and `AgentConfig::set_user_value` reach
    it and a host opts out with no new write API; decoded field by field, an
    unusable value is a repair. It is not in `AgentSettingsPatch`: a nested
    patch type is strict, which needs a strict-decoding policy entry, left
    with the `/config` editor for M7 (the TUI's form-coverage test lists the
    key as set by `config set` only).
  - Discovery (`agent/src/model_discovery.rs`): one worker in the Agent's
    worker group, with its own runtime, lists one Provider at a time through
    the kernel's `list_provider_models` (now taking a cancellation token;
    shutdown cancels a listing under way and it records nothing). Triggers:
    the service's construction (the default Provider), `create_session`, a
    model selection (`/model`, a role's model) and every activation for a
    turn (`activate_provider_session`). Deviation: the last is beyond the
    plan's "selected", so a resumed session on a non-default Provider gets
    its Provider listed; a send on a channel, rate-limited, never waits.
  - Rate limit, per endpoint (protocol and base URL): due when the
    endpoint's recorded listing (`listedAtMs`, persisted in the model
    metadata, and also set by the listings the person asks for) is a day old
    or ahead of the clock, and this run's last attempt is a day old after a
    success or 15 minutes after a failure. So a new run (each `exec`) does
    not list again within the day; failures are this run's only. A failure
    is logged at warn with the Provider's id, never its answer.
  - Late resolution (`kernel/src/service/context_window.rs`) runs in
    `stage_turn_admission`, after its last fallible check, under the
    session's lock and in the admission's commit (deviation: not
    `prepare_sampling_step`, which holds the record read-only and commits
    nothing). `AgentSessionConfig.context_window_source` (tolerant, left out
    when `None`, kept `None` with no window by `normalize_session_config`);
    creation and model selection set it. A configured window is never
    replaced, a window without a source counts as configured, a window no
    longer known is kept, a session with no route (not bound to its
    Provider) is not resolved; a changed window rewrites the usage window
    in the same commit. The kernel re-exports `ModelMetadataSource`.
  - Tests: the test constructors (`new_with_host` and the crate's) run no
    discovery, and `test_support::configure_test_provider` writes
    `discovery = "off"`, which covers the standalone runtime's tests, the
    CLI's and the TUI's (the standalone fixture that rewrites `config.toml`
    writes it too). `FakeServer::with_listings` answers `GET …/models` from
    `FakeListing`s on tasks of their own, kept apart from `requests()`;
    `CapturedRequest` gained `method`. No request golden or history corpus
    changed.
  - Host-visible: `AgentSessionConfig.context_window_source`,
    `AgentService::list_provider_models(…, cancellation)`,
    `AgentService::model_listing_recorded_at`, `AgentSettings.model_metadata`
    (`AgentModelMetadataSettings`, `AgentModelDiscovery`, re-exported by the
    facade), and the Agent's own `GET /models` at start and on a binding.
    `construct` fell to 186 code lines (the history store's opening moved
    to `open_history_store`); `kernel/tests/loop_contract/main.rs` is at
    899 of 900.
- M4 (done): a refusal as too long teaches the route a window, which the
  refused session takes at once; the threshold compaction keeps room for a
  reply (D4).
  - model-api `context_overflow.rs` (out of `image_input.rs`): the
    classifier, widened with Hoshimi's wordings, English and Chinese, as
    lowercase substrings (no regex), over the code and the message. A 429
    never counts, now even with a code (it did before); the three codes
    still count at any other status; the 400/413/422 gate stays, an in-band
    error is read; a rate-limit or quota marker (`rate limit`, `quota`,
    `tokens per min`, `限流`, …) never counts, even in the stream.
    Deviation: left out as too broad, bare `context length` / `context
    window` (the narrow `exceeds the context length` is in) and `request
    too large` (OpenAI's TPM rate limit and byte-size limits word it so);
    both stay `Remote`.
    `context_limit_in(message)` reads Hoshimi's five patterns, each at its
    first match, a value outside `PLAUSIBLE_CONTEXT_WINDOW` (1024..=20M,
    exported) moving on to the next; deviation: the first also reads
    `maximum prompt|input length is N` (xAI's wording).
  - Learning (`kernel/src/service/context_window.rs`): in
    `run_sampling_request`, on `ProviderContextWindowExceeded` and unless
    the turn is cancelled, before the loop compacts and retries: the named
    window, else the refused request's estimate within the same bounds
    (deviation: the plan bounded only a named one; a refused request under
    1024 tokens is taken for a misread refusal). The limit is parsed in the
    kernel from the error's message, so `ModelError` is unchanged.
    `ModelMetadataRegistry::learn_context_window` keeps the smallest live
    learned window (an expired one no longer counts; no change, no save).
    Learned whatever auto-compaction says, from a turn's requests only (a
    compaction's own overflow is trimmed as before and teaches nothing).
    Residual: a misread refusal, or a second one after the compaction (a
    smaller estimate), teaches the route a small window for 7 days across
    sessions and restarts; only `contextWindowTokens` overrides it, and
    nothing clears it yet (M7's `/status` will show its source).
  - Applied at once: its own commit under the session's lock, through M3's
    `refresh_context_window` (now returning the window taken), so a
    configured window stays and the usage window moves with it; the
    session's `AgentEventKind::ModelContextWindowLearned { tokens, previous }`
    only when what it took is the learned window. Behaviour change: the
    reactive compaction now runs against the learned window, so on a window
    unknown before it can fail as a known window's does
    (`ContextBudgetExceeded`, its next request still over the threshold)
    where it always retried. The TUI writes one info line before the
    compaction cell, `exec`'s text mode one line on stderr; `exec --json`
    carries the new kind (documented, per the stability rule).
  - D4: `AgentContextTokenStatus::calculate` takes `max_output_tokens`
    (host-visible): the limit is `min(window × percent / 100, window −
    min(max_output or 32768, max(4096, window / 10), window / 2))`. The
    largest output is resolved from the registry where the threshold is
    checked (`AgentService::max_output_tokens`), threaded into the
    background band and the post-install check, whose
    `ContextBudgetExceeded.limit` is now the status's limit. No request
    golden changed (window 100000 at 80% stays 80000). Deliberate test
    changes: the background suite's window of 10000 now compacts in the
    foreground at 5904, so its `IN_BAND` went from 6000 to 5500; the
    token-status contract test (`recovery_fork.rs`) moved its boundary to
    a window of 100000 and gained the headroom cases.
  - Tests: model-api `context_overflow/tests.rs` (every wording at each
    gating status, the codes, 429 and quota texts, what is left out, each
    limit pattern, commas, Chinese, the bounds); kernel
    `service/context_window/tests/learned.rs` (named and estimated windows,
    the event and its order, proactive compaction by the learned window,
    larger than reported unused, configured kept while the route learns,
    auto-compaction off, recovery after the rewrite, the next run, the
    route's largest output in the threshold) and the registry's smallest
    window (`precedence.rs`); the TUI's notice (`app/tests/compaction.rs`,
    a transcript check over an in-band `response.failed`).
  - Module size: `kernel/src/activity.rs`'s exhaustive event dispatch took
    the new kind's arm, so its exception's ceiling went from 646 to 647
    (`architecture/module-size-policy.toml`), rather than a rewrite only to
    fit the number.
- M5 (done): a session's image input is its route's resolved image fact
  under the person's policy, so a model known not to read images is sent
  none from the first request.
  - `ModelMetadataRegistry::observed_image_input(route, configured, now)`
    maps the resolved fact (`resolve`, so a learned "no" beats a reported
    "yes" and a catalog "no" is ignored): yes `Supported` (produced for the
    first time; it acts as unknown does), no `Unsupported`, none `Unknown`.
    `ModelInputCapabilityPolicy::effective` still applies the policy last.
  - `ConfiguredModelFacts.image_input`: `imageInput = "disabled"` is the
    configured "no" (`Some(false)`), `"auto"` configures nothing; read in
    one place, `configured_image_input` (`service/image_capability.rs`),
    where D3's per-model override is to be read. Behaviour-identical to
    leaving it unset, since `effective` applies the policy too; it makes
    the resolved fact's source honest for M7.
  - Applied where the capability was set: creation, model selection (was
    reset to `Unknown`), recovery (was learned-only) through
    `AgentService::session_input_capabilities`; and the refresh before a
    turn's admission, a steering, each sampling step, each tool call and
    each snapshot (unchanged points), now one step under the session's
    lock (`lock_after_persistence`) instead of a route read, a registry
    read and a re-check. A session whose transport names no route keeps
    what it has, as before (`record_unsupported_image_input` still sets
    `Unsupported` directly). Child sessions and forks copy their source's.
    Deviation: the snapshot of a session not loaded
    (`persisted_runtime_snapshot`) has no transport, hence no route, and
    still shows the policy alone.
  - Behaviour: a model reported (or learned) without image input gets no
    image: the step's request replaces history images by the placeholder
    before it is sent (no refused round trip, no `ImageInputFallback`),
    `view_image` is not planned, and a host's image at a turn's start or in
    a steering is refused with the existing `InvalidInput` message. A fact
    that changes mid-turn applies from the next step (the plan and the
    request read the refreshed capability; a `view_image` planned before is
    refused when it runs). Known yes or unknown: as before. No request
    golden or history corpus changed; no host-visible API changed (the
    snapshot's `input_capabilities.image_input` is now `false` for such a
    model).
  - Tests: kernel `service/image_capability/tests.rs` (a listing's "no"
    after an image was sent: placeholder at once, no fallback, a turn with
    an image refused; steering with an image refused; a reported "yes"
    sent images until a refusal, which then beats it for every session;
    `disabled` over a reported "yes"; recovery seeds the record; creation
    and selection over real transports); the registry's `precedence.rs`
    (a reported yes is `Supported`, configured "no" wins, a catalog "no"
    stays `Unknown`); facade `tests/model_vision.rs` (a listing of
    `input_modalities: ["text"]` through `list_provider_models`: the next
    request has no `view_image` and no image, a turn with an image is
    refused; an in-band image refusal in one run, then the next run over
    the same data directory: a new session takes no images and offers no
    `view_image`).
  - Docs: `providers-models` (images; the stale "largest output … not
    acted on"), `architecture` (model metadata: image input),
    `config-reference` (`inputCapabilities.imageInput`). `internals`
    documents no image capability path, so it is unchanged.
  - Residual: a compaction's own request (`compaction/source.rs`) is not
    sanitized, so images in history from before the "no" was known reach
    a model known not to read them (as with a learned refusal before M5).
  - Residual: a listing's wrong "no" now keeps every image from the model
    (the person's included, refused at the turn's start) with nothing to
    override it until D3's per-model `imageInput` (M7): the policy has
    only `auto` and `disabled`, a learned "yes" is ignored, and a model
    its listing still names keeps the reported fact with no expiry.
- M6 (done): the effort a request carries is narrowed to what the route
  is known to take, a refused reasoning parameter is retried once without
  it and learned, and every protocol sends an effort (D1).
  - Listing (model-api `model_catalog/shapes.rs`): the documented effort
    lists M1 left, OpenRouter's `reasoning.supported_efforts` and
    `default_effort`, Anthropic's `capabilities.effort.<level>.supported`
    (from the M1 fixtures); Codex's levels first, then OpenRouter's, then
    Anthropic's, the first that lists any. Deviation: the efforts are now
    sorted on the ladder (`reasoning_effort_rank`), those off it after in
    the entry's order (M1 kept Codex's order).
  - Narrowing (`request_reasoning_effort`, model-api `reasoning_effort.rs`,
    used by the kernel's `service/reasoning.rs`): §4.7's table; ladder
    `minimal < low < medium < high < xhigh < max`; listed → as is, else
    Hoshimi's alias if listed (`minimal`/`light` → `low`, `balanced` →
    `medium`, `deep` → `high`, `xhigh` → `max`), else the nearest ranked
    listed level (a tie goes lower), else none (an unranked effort not
    listed, or no listed level ranked). Resolved per request in
    `begin_sampling` and in both compaction entries (`prepare.rs`,
    `background/launch.rs`, through a new `compaction_source` parameter);
    the estimate requests still build with the configured effort. The
    request also carries the route's largest output
    (`ModelReasoningConfig.max_output_tokens`, host-visible). Deviation:
    the one warning per session is a `log::warn!`
    (`SessionRuntime.reasoning_narrowing_logged`, per run), not an event:
    a new `AgentEventKind` would grow `activity.rs` past its ceiling.
  - D5 (`reasoning_refusal.rs`): `ModelError::UnsupportedReasoning` /
    `AgentError::UnsupportedReasoning`, after the overflow and image
    classifiers; 400/422 only (in-band never), a message naming
    `reasoning`, `thinking`, `effort` or `output_config`, and a code
    `unsupported_parameter`/`unsupported_value` or a refusal wording
    (`unsupported`, `not supported`, `does not support`, …, `unknown`,
    `unrecognized`, `not allowed`, `not permitted`, `unexpected
    keyword`). No Chinese wording (no real example found). The fallback in
    `run_sampling_request`: once, only if the request carried reasoning,
    `ModelRetryScheduled { stage: ReasoningFallback }` (new
    `AgentModelRetryStage`, `reasoning_fallback` in `exec --json`),
    `ModelMetadataRegistry::learn_reasoning_unsupported` (7 days,
    persisted, beats a reported yes). The image fallback now sanitizes the
    active request, so the two compose. Residual: a no-route transport
    learns nothing and is refused once per request; a refused level
    (`unsupported_value`) teaches "no reasoning", not the levels.
  - D1: Chat Completions sends `reasoning_effort` (OpenAI, OpenRouter's
    parameters page and DeepSeek take it; no `max_completion_tokens`).
    Anthropic: `takes_manual_thinking(model)` by id: Claude 3 and 4 up to
    4.5 (dated ids, Bedrock/Vertex ids included) and every non-Claude id
    take `thinking: {type: "enabled", budget_tokens}` (budgets 1024, 4096,
    8192 (`medium` and off-ladder), 16384, 24576, 32000 for `minimal` …
    `max`; `max_tokens` = budget + 16384 under the route's largest
    output, else 16384 with the budget cut to leave 4096; under 1024 →
    no thinking; never with a forced tool); every other Claude id takes
    `thinking: {type: "adaptive", display: "summarized"}` with
    `output_config: {effort}` (`minimal` → `low`), `max_tokens` 32768
    under the largest output, else 16384. No temperature/top_p/top_k is
    sent. `supports_reasoning_effort` is removed; `normalize_session_config`
    keeps an effort on every protocol (drops it only for a neutral
    session). The helpers are `chat_completions/reasoning.rs`,
    `anthropic_messages/thinking.rs` and `anthropic_messages/assistant.rs`.
  - Thinking round trip: the Anthropic decoder keeps each `thinking`
    block's text and signature (`signature_delta`) and each
    `redacted_thinking` block's `data` as `ModelOutputItem::Reasoning.replay`
    (`ModelReasoningReplay`, ≤ 1 MiB a block, Debug redacted,
    `#[serde(skip)]`), carried on `ConversationItem::Reasoning.replay`
    (`#[serde(skip)]`: in memory only, no history-format change, the
    corpora unchanged). `regular_model_request` places the current turn's
    blocks (after its last `TurnStart`) as the new
    `ModelInputItem::Reasoning`; only Anthropic with manual thinking sends
    them, first in the assistant message they preceded; Responses and Chat
    skip them. Guard: a manual request continuing tool calls whose message
    has no block (recovered turn, mid-turn compaction, a step that thought
    nothing) is sent without thinking. Deviation from the brief: adaptive
    thinking sends no block back. Anthropic drops the "must start with a
    thinking block" rule there and lists removing every block as valid,
    while its 5.1+ models refuse a block whose prefix (system, tools, any
    earlier message) changed, which the Agent's requests do between steps
    (task plan last, world state moved, `tool_search` tools); the binding
    models all refuse manual thinking, so replay is required exactly where
    binding cannot happen. Residuals: adaptive models that do not bind
    (4.6–5) lose interleaved reasoning continuity; once a manual step's
    response has no signed block, the guard keeps thinking off for the
    rest of that turn; the blocks stay in memory while the session is
    loaded (≤ 1 MiB each), uncounted, and the token estimate does not count
    them; the fake server's manual-thinking refusal is a model of
    Anthropic's rule, not a recorded body. Unverified against a live
    Provider.
  - Behaviour change: a Chat Completions or Anthropic session with an
    effort configured now sends it (it was dropped); a refusal the
    classifier misses (in-band, or a body without `error.message`) fails
    the turn where the effort used to be left out.
  - Interface: `/model` has no effort step and `/config` no effort field,
    so only the `/providers` policy form changed: a choice of the default
    model's known levels (with "Provider default", and the configured
    effort kept when unlisted), free text otherwise; the levels are read
    with the registry (`ProviderReply::Loaded { registry,
    reasoning_efforts }`) through the new facade
    `AgentApplicationService::model_reasoning_efforts` and kernel
    `AgentService::model_reasoning`; `FormView::chosen`.
  - M5 residual fixed: a compaction's request is sanitized for images
    when the session takes none, and both compaction entries refresh the
    session's image input first.
  - Tests: model-api (`reasoning_effort/tests.rs`, `reasoning_refusal/tests.rs`
    with the bodies found (openai/openai-python#2532, Anthropic's
    documented 400s; the other wordings illustrative) and negatives, `anthropic_messages/thinking/tests.rs`,
    `tests/provider_protocols/thinking.rs`: Chat field, adaptive and
    manual bodies, budgets, guards, a fake-server round trip of a signed
    and a redacted block, adaptive sending none; the listing suite's
    effort fields); kernel (`service/reasoning/tests.rs`: narrowing,
    unsupported, refusal → one retry, event, learned, next turn none, a
    second refusal fails; compaction narrowed and sanitized; registry
    persistence of the learned refusal); facade
    (`tests/model_reasoning.rs`: a manual-thinking round trip through the
    real Anthropic transport against a fake that refuses a continuation
    without the block, and a refusal in one run not sent in the next);
    TUI (`app/tests/settings.rs`: the policy form's choice). Deliberate
    test changes: the protocol policy test that expected Chat/Anthropic to
    refuse reasoning, the normalization and persistence-contract tests
    that expected the effort dropped, the Anthropic decoder test that
    expected signatures dropped. No request golden or history corpus
    changed.
  - Docs: `providers-models` (effort per protocol, narrowing, the
    budget table, the fallback, the round trip, the policy form),
    `architecture` (model metadata: reasoning; thinking; the model API),
    `internals` (listing rows, decoder row, the refusal classifier,
    normalization, thinking details), `json-events`
    (`reasoning_fallback`), `config-reference` (`reasoningEffort`),
    `embedding` (`model_reasoning_efforts`), `data-directory` (the learned
    fact), both languages. `terminal-interface` defers the form to
    `providers-models#interface`, so it is unchanged.
  - Module size: `encode_anthropic_request` fell to 184 lines, so its
    exception was removed from `architecture/module-size-policy.toml`
    (the check requires it); `encode_chat_request` is at 219 of 227.
- M7 (done): per-model overrides (D3), showing what is known (§4.9),
  forgetting learned facts, and the `/config` editor of
  `modelMetadata.discovery`.
  - Config: `providers[].models."<id>" = { contextWindowTokens,
    maxOutputTokens, imageInput = "auto" | "supported" | "unsupported",
    reasoningEfforts }` (`AgentProviderSettings.models`, a
    `BTreeMap<String, AgentModelOverrideSettings>`, config
    `providers/models.rs`). `decode_providers` takes `models` out of the
    raw Provider and reads it per model and per field with `Fields`, so a
    bad override never drops its Provider: a field that cannot be used
    (a window of 0, an effort list holding `none`) is unset with a repair,
    an entry that is not a table (or whose id is empty or over 256 bytes)
    is dropped with one; an unreadable `reasoningEfforts` is unset, never
    `[]`. `write_provider` keeps the existing `models` (the draft is the
    connection, strict, and has none), so the overrides are written by
    hand only. Not in the `/providers` policy form: the form writes the
    shared policy and its default model is editable in the same form, so
    per-model fields there would not say which model they belong to.
  - Kernel: `AgentConfiguredModelFacts { context_window_tokens,
    max_output_tokens, image_input: Option<bool>, reasoning_efforts }`
    (public, `model/configured_facts.rs`), kept on the session as
    `AgentSessionConfig.configured_facts` (tolerant per field, left out
    when empty, so no history corpus changed). The facade fills it with
    `configured_model_facts(provider, policy window, model)`: the override,
    its window else the policy's `contextWindowTokens` (so for that model
    the override beats the policy key), at creation, model selection
    (`AgentSessionModelConfigurationUpdate.configured_facts`, which
    replaces its `context_window_tokens`) and every activation
    (`activate_model_session` gained the parameter; persisted without a
    revision, as the compaction reconcile is). So an edit of
    `providers.toml` reaches a live or resumed session at its next turn.
    Behaviour change: the policy's `contextWindowTokens` now reaches an
    existing session at its next turn too (it waited for `/model`).
    `ConfiguredModelFacts` gained `max_output_tokens` and `reasoning`;
    `configured_facts(config)` (`service/model_facts.rs`) is used by the
    reasoning narrowing, the largest output (D4 headroom and Anthropic's
    budget), the window and the image input. `configured_image_input`
    moved there: the policy's `disabled` is a "no" over everything, else
    the model's `imageInput`. `reasoningEfforts = []` is `Unsupported`,
    a list `Supported { efforts, default: None }`.
  - Window: the configured window is `configured_facts`'. A window
    without a source (a host's, or one written before) is adopted into
    `configured_facts` at creation and at recovery
    (`adopt_configured_window`), so hosts and old sessions keep it.
    Deviation from M3's "a window no longer known is kept": a session
    whose window was configured or learned, and that resolves to none
    (override removed, learned window forgotten), now has an unknown
    window; a reported one, and one written without a source (before M3),
    are still kept (a source-less one gives way to a reported window once
    its configuration no longer names it). `changed_window` compares the
    origin, so a source-less configured window stays source-less.
  - `imageInput = "supported"`: configured wins at resolution; a refusal
    still falls back for its request and is still learned (so a session
    without the override, on the same route, takes the "no"), and the next
    request (step or turn) sends images again. So under it a model that
    really refuses images is refused once per request; honest and tested.
    The same for `reasoningEfforts` and a reasoning refusal.
  - Forgetting: `ModelMetadataRegistry::forget_learned(endpoint,
    model)` drops the learned facts of one route or every route of an
    endpoint (reported facts stay), saves once if anything changed, and
    returns how many routes had any. Kernel `AgentService::
    forget_learned_model_facts(protocol, base_url, model)`, facade
    `AgentApplicationService::forget_learned_model_facts(provider_id,
    model)`, TUI `/model forget` (the session's model; `/model` now takes
    inline arguments, `Usage: /model [forget]`), CLI `providers forget ID
    [--model MODEL]`. A live session takes it at its next admission.
  - Showing it: kernel `AgentService::model_metadata(protocol, base_url,
    model, &configured)` replaces `model_reasoning`; facade
    `model_metadata(provider_id, model)` (the override, not the policy's
    window) replaces `model_reasoning_efforts`, and
    `session_model_metadata(session_id)` gives the session's facts with its
    own window and source. Pickers (`/providers`' and `/model`) show
    `200K · vision · reasoning` for what an entry reports
    (`tui/src/model_facts.rs`). `/status`: `Context window 80% left (40K
    used / 200K, reported)`, `200K (reported)` before any usage, or
    `unknown (compacts on the Provider's error)`; the model line adds
    `, sends high` or `, not sent: the model takes none` when the effort
    is narrowed. The policy form's window placeholder is `200K, reported`
    (else `unknown: compacts on the Provider's error`), and its effort
    choices take a configured `reasoningEfforts`. `doctor --online` prints
    a `model` line per listed model: `window 200000 (reported), images no
    (reported), …` or `nothing known`. The catalog source is named
    `catalog` (M8 can rename it). The learned-window notice (M4) is
    unchanged.
  - `/config general` edits `modelMetadata.discovery`:
    `AgentSettingsPatch.model_metadata: Option<AgentModelMetadataSettings>`
    (now `Deserialize` with `deny_unknown_fields`, and listed in
    `architecture/strict-decoding-policy.toml` as an `ipc-request`);
    the coverage test's `COMMAND_KEYS` exemption is gone.
  - Host-visible: `AgentSessionConfig.configured_facts`,
    `AgentSessionModelConfigurationUpdate.configured_facts` (replaces
    `context_window_tokens`), `activate_model_session(…,
    configured_facts, session_id)`, `AgentService::model_metadata`,
    `session_model_metadata`, `forget_learned_model_facts` (kernel and
    facade), `model_reasoning` and `model_reasoning_efforts` removed,
    `AgentProviderSettings.models`, `AgentSettingsPatch.model_metadata`;
    the facade re-exports `AgentConfiguredModelFacts`, `ModelMetadata`,
    `ModelMetadataSource`, `ModelReasoningSupport`, `Sourced`,
    `request_reasoning_effort`, `AgentModelOverrideSettings`,
    `AgentModelImageInput`.
  - Tests: config (`per_model_overrides_read_field_by_field_and_survive_a_write`);
    kernel registry `tests/forget.rs` (one model, an endpoint, persisted,
    configured over learned and reported), `context_window/tests/
    configured.rs` (a forgotten learned window no longer limits the next
    turn, unknown and back to reported; a configured window taken and
    given up at the next turn), image (`a_configured_image_input_beats_
    what_is_listed_and_learned`), reasoning (`configured_levels_replace_
    the_listed_ones`); facade `tests/model_vision.rs` (an override reaching
    a live session at its next turn and surviving a Provider edit, then
    removed; a refusal forgotten in one run is not kept by the next); TUI
    (window in the picker, the form's placeholder and `/status`, `/model
    forget`; `/config general` saving discovery); CLI (`providers
    forget`, `doctor --online`'s model lines). No request golden or
    history corpus changed.
  - Policies: `architecture/strict-decoding-policy.toml` gained
    `AgentModelMetadataSettings`; `architecture/notices-policy.toml`
    lists `tui/src/model_facts.rs` and its tests as original. No
    module-size exception or ceiling changed.
  - Docs: `providers-models` (new "Per-model settings" and "Forgetting
    what was learned"; window order, images, reasoning, the policy form,
    the picker), `config-reference` (`[providers.models."ID"]`, the
    discovery editor, the policy rows), `terminal-interface` (`/model
    forget`, `/status`, `/config general`), `cli-reference` (`providers
    forget`, `doctor --online`), `troubleshooting`, `embedding` (the new
    calls, the patch, activation), `architecture` (configured source,
    late resolution, image input, reasoning, known limits), `internals`
    and `data-directory` (forgetting), both languages.
  - Behaviour change: `/model gpt-5.1` (text after `/model`) used to go
    to the model as a message; it is now `Usage: /model [forget]`.
  - Unverified: the "sends …" wording of `/status` (only the library
    narrowing it calls is tested); a second step of the same turn after a
    refusal under `"supported"` or configured `reasoningEfforts` (one
    fallback per request is the existing loop's rule, tested per turn); an
    override picked up across a restart (live pickup through the same
    activation is tested); `providers forget` persisting across CLI runs
    (the facade test covers it across runs). `kernel/tests/loop_contract/
    main.rs` is at 900 of 900.
- M8 (done): models.dev (D2), optional and off by default.
  - Config: `[modelMetadata] modelsDev = true | false` (default `false`),
    `AgentModelMetadataSettings.models_dev`, read field by field (an
    unusable value is a repair); `/config general` gains the toggle **Use
    models.dev** beside discovery (its subtitle says it is a request to
    models.dev from this machine); the coverage test lists the key. The
    patch type stays strict, and `models_dev` is a required field of it, as
    `discovery` is (host-visible: a patch's `modelMetadata` must carry it).
  - model-api `models_dev.rs`: `ModelsDevCatalog::from_api_json` reads
    `api.json` one provider at a time (a map visitor, so the document is
    never one `Value`); a provider or model that does not read is left out,
    a field not of its type is unknown, the count logged at debug; only a
    body that is not an object or names no usable model fails. It keeps,
    per provider, its `api` host and, per normalized id, `limit.context`
    (within `PLAUSIBLE_CONTEXT_WINDOW`), `limit.output` (> 0), and a "yes"
    to `reasoning` and to `"image"` in `modalities.input`; a "no" is not
    kept (add-only). `fetch_models_dev(url, cancellation)`: its own reqwest
    client (no Provider credential or header), User-Agent
    `hanafubuki-agent/<version>`, connect 10 s, a minute in all, 32 MiB,
    cancellation. `ModelEndpoint::host` is new.
  - Matching: an id is lowercased, cut after its last `/`, stripped of a
    trailing `-latest`, then of a trailing `-YYYYMMDD` (eight digits).
    The Provider's own entry first (`ModelProviderNames`: a models.dev key
    equal to the Provider's id or a word of it, or a label of the base
    URL's host, or a provider whose `api` is at that host); else every
    provider's entries for the id merged, the smaller limit and any "yes"
    (deviation: the plan said "first provider wins" was not to be taken
    and left the fallback open; the smallest is the safe direction, as
    `built_in.rs` says, and deterministic). Two ids of one provider that
    normalize alike merge the same way. A `reasoning` "yes" is
    `Supported { efforts: [], default: None }`, which leaves the effort as
    configured, so models.dev never changes the reasoning on the wire.
  - Kernel: the copy is `ModelMetadataSnapshot.modelsDev`
    (`{fetchedAtMs, catalog}`, an `Arc` shared by every snapshot, tolerant:
    a copy that does not read is none), used for 7 days
    (`MODELS_DEV_TTL_MS`, a fetch time ahead of the clock counted from
    now), looked up per resolve by model id; the per-route `catalog`
    bucket M2 kept (and nothing wrote) is removed, `resolve` takes the
    looked-up facts in its place, so the §4.2 precedence is unchanged
    (configured > learned > reported > models.dev > built-in). The
    endpoint's names are this run's (`RegistryState.provider_names`, not
    saved), taken wherever the kernel gets a Provider's id with its base
    URL: `create_model_session`, `activate_model_session`,
    `set_session_model_configuration`, `model_metadata` and
    `model_context_window` (both gained `provider_id`, host-visible). A
    session's window recorded from models.dev is dropped, like a
    configured or learned one, once nothing knows it (models.dev turned
    off, its copy expired). `forget_learned` keeps the copy. New kernel
    API: `record_models_dev_catalog`, `forget_models_dev_catalog`,
    `models_dev_fetched_at`; re-exports `fetch_models_dev`,
    `ModelsDevCatalog`, `MODELS_DEV_URL`. `ModelMetadataSource::Catalog`
    keeps its name and its wire value (`catalog`); `/status`, the
    pickers' placeholders and `doctor --online` show it as `models.dev`.
  - Facade: the discovery worker, at each request (start, a session
    created, given a model or activated) and whatever `discovery` says,
    then drops the copy when `modelsDev` is off, else downloads when the
    copy is a day old (or missing) and this run's last try is a day old
    after a success or 15 minutes after a failure (the discovery periods).
    A failure is logged at debug (what failed, never what models.dev
    answered) and the copy goes on being used. `ModelDiscoveryPolicy`
    carries the URL (`Clone`, no longer `Copy`); the test policy points it
    at a port where nothing listens, so no test reaches the network.
    Deviation: the brief's "24 h cache" is the refresh age; the copy is
    used for a week, so models.dev unreachable is silent and the copy keeps
    answering (with a 24 h life a failed refresh would drop it at once).
  - Tests: model-api `models_dev/tests.rs` (normalization, provider
    names) and `tests/models_dev/` (a fixture in models.dev's shape with
    entries that do not read: tolerance, refusals, own entry first, the
    cautious merge, the kept form's round trip and damage, the fetch's
    User-Agent and no credential, errors, cancellation); kernel
    `model_metadata/tests/models_dev.rs` (reported beats models.dev,
    models.dev beats built-in, learned and configured above it, a "no"
    not kept, own entry first else the smallest, forgetting learned facts
    keeps the copy, saved and expired after a week, a copy that does not
    read, dropping it saves once); facade `tests/model_discovery.rs`
    (off by default: no request to the test server; on with discovery
    off: fetched once with the User-Agent, a session's window from it at
    its next turn, the copy spares the next run a download (a second
    answer waits, unasked), a Provider renamed to a models.dev key takes
    its own entry over the other, in a session and in `model_metadata`;
    unreachable:
    silent, the copy kept; turned off: the copy and the session's window
    dropped); config (`modelsDev` read, repaired, set); TUI (the toggle
    saved, `models.dev` as a source). The precedence test that set the
    removed per-route bucket by hand became the kernel's models.dev
    suite. No request golden or history corpus changed.
  - Docs: `providers-models` (new "models.dev" section; window order,
    images, forgetting), `config-reference` (`modelMetadata.modelsDev`,
    the third-party request), `data-directory`, `architecture` (the
    sources, discovery, late resolution, images), `internals` (decoding,
    the file), `terminal-interface`, `cli-reference`, `embedding`, both
    languages. No page said models.dev was not yet available.
  - Policies: no strict-decoding, module-size or notices change (the new
    files are outside the derived directories; `AgentModelMetadataSettings`
    was already listed).
  - Unverified: the live `https://models.dev/api.json` (the fixture is
    written in its shape, not recorded: no network here) and its current
    size against the 32 MiB bound; Hoshimi's caution stands that a flat
    id's limits are one provider's numbers, which is why the fallback
    takes the smallest. A models.dev largest output is not sent: the
    request's `max_output_tokens` (Anthropic's `max_tokens` and thinking
    budget) skips that source, per §3's "a third-party flag must never
    change a request on its own"; it still enters the compaction headroom.
  - Residuals: a copy
    on disk with `modelsDev` turned off still answers until the worker's
    first request at start; the failure backoff is per run, so short runs
    (`exec`) with models.dev unreachable try once each.
- Follow-up (2026-11-12, done): Chat Completions' `reasoning_content` is
  sent back (DeepSeek's thinking mode refuses a request with tools whose
  earlier assistant messages, of any turn, lack it; guides read
  2026-11-12).
  - Type: `ModelReasoningReplay::ReasoningContent { text }` (≤ 1 MiB,
    Debug redacted), with `outlives_its_turn()` (true for it only) and
    `text_len()`. Decoder: a stretch whose every delta came as
    `reasoning_content` keeps it (`reasoning_delta` says the field); a
    stretch with a `reasoning` delta (OpenRouter's) keeps none. No model-id
    sniffing.
  - Encoder: the three assistant `json!` sites of `encode_chat_request`
    moved to the new `chat_completions/assistant.rs`, which joins the
    `reasoning_content` of the reasoning before a message's text or first
    tool call (no separator: the whole text a response not streamed
    carries) onto that message, in both shapes (`content: null` for calls
    alone); not gated on `request.reasoning`; reasoning no message follows
    is dropped; a reasoning item between the text and the calls no longer
    splits the message. `encode_chat_request` is 185 lines, so its
    module-size exception was removed (the check asked for it). Anthropic:
    `assistant_message`, `continues_without_thinking` and
    `is_assistant_item` match the variants, so the text is left out and is
    no thinking block for manual thinking's guard.
  - Kernel scope: `ConversationItem::reasoning_input(in_current_turn)`
    places a block that outlives its turn from every turn of the projected
    history, Anthropic's after the last `TurnStart` only; the compaction
    request (which carries the tools) places the former too.
    `hierarchical`'s `apply_view` keeps reasoning with the message after
    it (after the summaries before it, dropped with a folded one); before,
    a summary anchored before an answer separated them.
  - Persistence: `replay` is written only for this variant
    (`"replay": {"type": "reasoning_content", "text": "…"}`, skipped
    otherwise), and read tolerantly (any other shape, kind, or one over
    the bound → none). Anthropic's stay memory-only. A transcript page
    (`AgentTranscriptPage`, `sessions show --json`) leaves it out, as
    `modelText`; forks and children copy it (they continue the
    conversation). Not counted in `retained_bytes` / `maximum_history_bytes`
    (as Anthropic's were not), so a DeepSeek item can persist its text
    twice (shown ≤ 256 KiB, sent back ≤ 1 MiB). Corpora and goldens
    unchanged.
  - Fixed on the way: reload validation required a completed turn's final
    answer to have the `final_answer` phase, which Chat and Anthropic
    messages never have, so such a session failed its first turn after a
    restart (`dangling final answer message id`); it now takes `None` too,
    as `finish_completed` does.
  - Token accounting: the request estimate serializes the request, so the
    text already counted; `estimate_conversation_items_tokens` now counts
    the bytes of reasoning that outlives its turn (Anthropic's and the
    shown text still not).
  - Classifier: a refusal whose message names `reasoning_content` is never
    `UnsupportedReasoning` (it is about the history, not the reasoning
    parameter), so DeepSeek's "Missing `reasoning_content`…" or a server's
    "Unrecognized … reasoning_content" stays `Remote`.
  - Tests: model-api decoder/encoder units, `provider_protocols/thinking.rs`
    (Chat send-back, Anthropic leaves it out), classifier; kernel
    persistence codec, transcript, `apply_view`; facade
    `tests/model_reasoning.rs` against a fake that refuses a `deepseek-`
    request with tools and an assistant message without the field (tool
    loop, second turn, a turn without a call, a reopened runtime,
    compaction and the turn after it, `reasoning` not sent back). The main test fails with the
    attach disabled, and its restart half with persistence disabled.
  - Residuals: a session from before this (or a message whose thinking
    passed 1 MiB) has assistant messages with no text to send, which go
    without the field; DeepSeek with tools may refuse every request of it,
    compaction's too, while they are in the window: start a new session.
    The refusal wordings in the tests are illustrative, not recorded from
    the live API.
- Model Providers and models.dev through the proxy (done, 2026-11-12):
  `[network] proxy` (the host's `NetworkSettingsPort`) reaches every model
  API client. model-api's `ModelHttpProxy` (system, disabled or a URL;
  reqwest gains `socks` and `system-proxy`, which the binary already
  unified in through web-search; its `Debug` hides a URL, which may hold
  credentials) is on `ModelProvider::with_proxy`, applied by every
  transport and the listing, and taken by `fetch_models_dev`. The kernel
  keeps one (`set_model_proxy`/`model_proxy`) for each model it builds:
  sessions (created, reconfigured, activated for a turn), listings and
  private sessions (titles, recaps, memory extraction). The facade sets it
  at start and at each settings change (`apply_settings_to_plugins`, with
  MCP's), keeping the last one when the host cannot say. A request already
  streaming keeps its client; a session takes a change at its next turn.
  `"system"` is reqwest's default (proxy variables, then the OS settings),
  so it is the old behaviour. An unusable proxy URL fails the client's
  construction without naming the URL.
- MCP stdio servers through the proxy (done, 2026-11-12): they already
  took the proxy variables of the executor's overlay, but the standalone
  Agent set the overlay only at start, so a change reached no process (the
  commands neither). Its `ConfiguredNetwork` now puts the proxy in the
  overlay at each read (start and each settings change, before what the
  proxy reaches is restarted); the overlay names every proxy variable in
  upper and lower case (`ALL_PROXY` added; upper case only on Windows), so
  a stdio server, which inherits the Agent's environment, keeps no
  lower-case one the setting overrides. A proxy policy change reconnects
  every enabled MCP server, the stdio ones started anew, not only the
  Streamable HTTP ones. A host keeps its own overlay current before
  `apply_saved_settings`.
- Everything else that reaches the network, checked (2026-11-12): MCP
  OAuth (login, discovery, the token refresh: rmcp's `refresh_token` uses
  the MCP OAuth client the proxy builds) web fetch redirects (each hop
  builds its client with the proxy) and the Jina Reader fallback (its client
  takes the proxy) already took it; tests now prove both
  with targets only the proxy reaches. Plugin package and marketplace
  downloads are `git` through the instance's executor (inherited
  environment plus the overlay), so they take the proxy variables, the
  lower-case `http_proxy` curl needs for `http://` remotes among them since
  the overlay names both cases; a test runs `git ls-remote` through it. Skills
  install only from a folder; one from Git is the `skill-manager` Skill's
  `git clone` through `exec_command`, a command, so it takes the same
  variables. A
  user's git `http.proxy` still wins over the variables. No other crate
  makes HTTP requests (reqwest is only in model-api, web-search and mcp).
- Loopback around a proxy URL (2026-11-12): an explicit proxy no longer
  takes this machine's loopback (`localhost`, `127.0.0.0/8`, `::1`) in
  the model API's clients (a local Ollama or LM Studio Provider, the
  listing) and MCP's HTTP client (Streamable HTTP servers, OAuth), as the
  processes' `NO_PROXY` already said; web search keeps its proxy for every
  target, since it refuses loopback targets anyway. Left as is, and
  documented: the web fetch's public-target check resolves names here even
  through a proxy (the SSRF guard; `network-logging#web-fetch-dns`); `"system"` is the OS settings for the
  HTTP clients on macOS and Windows but only the environment's variables
  for the processes; a git `http.proxy` wins for clones; a host keeps its
  own process overlay current.
- Web fetch names through the proxy, opt-in (2026-11-12, the user chose
  the opt-in over an automatic fallback or always): `webFetchResolveThroughProxy`
  (default `false`; `/config` Web tools, the settings patch, the host's
  form schema with `settings.fields.webFetchResolveThroughProxy.*`, which
  Hanafubuki's frontend must label). With it and a proxy URL, a name that
  does not resolve here is left to the proxy (no address is pinned); one
  that resolves here is still checked, and literal private addresses and
  the blocked names still fail. Residual, documented: what the proxy
  reaches for an unchecked name (an intranet behind a company proxy);
  `socks5://` still resolves here (use `socks5h://`).

## Plan: Codex's messages sent while the Agent works, in the terminal interface (2026-11-11)

Ported from Codex `57a808a392` (`codex-rs/tui/src/chatwidget/input_queue.rs`,
`input_flow.rs`, `input_restore.rs`, `input_receipts.rs`, `interaction.rs`,
`bottom_pane/pending_input_preview.rs`, the composer's `queue` key and the
`edit_queued_message` binding). No kernel or facade change, nothing
model-visible, no golden.

- [x] One input queue per conversation, in the chat (`chatwidget/input_queue.rs`,
      `input_flow.rs`), over the bridge's steering (`agent_bridge/steering.rs`).
      Done (2026-11-11):
      - Enter while a turn runs steers it: one steer in flight at a time, each
        kept until the turn's `SteeringConsumed` covers the sequence it was
        accepted under, and written into the conversation only then. Enter
        before the turn has started queues (Codex's `user_turn_pending_start`).
      - Tab (`composer.queue`) while a turn runs queues; with nothing running
        it sends, as Enter does. A command typed with Tab runs at once (Codex
        queues it to parse at dequeue: not ported).
      - A turn's end settles the queue once every steer has its answer.
        Completed or failed: the unread steers and the held messages go as one
        turn, else one queued message per turn. Esc with steers waiting
        interrupts and sends them as one turn; Ctrl-C, or Esc with none, puts
        everything back in the composer before the draft. Nothing is dropped:
        a steer that raced the turn's end becomes the next turn.
      - Esc interrupts while the answer streams (Codex), so the docs' callout
        is gone.
      - Shift+Left / Alt+Up (`chat.edit_queued_message`) puts the last queued
        message back in the composer, before the draft (Codex replaces the
        draft).
      - The waiting messages show above the composer (Codex's three groups);
        a draft written during a turn shows `tab to queue message`, and the
        `?` shortcuts list Tab (to queue, or to send with nothing running).
      - A review holds Enter's messages for its end (the bridge's own hold is
        gone); Esc then warns instead of cancelling the review, Ctrl-C cancels
        it and puts them back.
      - Editing an earlier message, `/new`, `/resume` and the other commands
        that run with no turn are refused while a message waits; a session
        opened drops the queue.
      - Changed from before: a cancelled review puts the messages held for
        it back in the composer (they used to be sent); Enter before a turn
        has started, the session's creation included, queues a turn of its
        own (it used to steer the first turn); a failed session creation
        with a message waiting ends as a failed turn; the commands that run
        with no turn are refused while a message waits; Tab within a paste
        (no bracketed paste) is the paste's tab; in a subagent shown, a turn
        the interface starts there is read from the subagent's journal only
        (its events came twice, through the call's sink too, so its answer
        was written twice and its end seen twice).
      - Validation (Linux): `cargo fmt --all --check`, `RUSTFLAGS="-D
        warnings" cargo check --workspace --all-targets --all-features`,
        `cargo clippy --workspace --all-targets --all-features -- -D
        warnings`, `cargo xtask check`, `cargo test --workspace
        --all-features`, `bun run format:check:site`, `site:check`,
        `site:test` and `VITE_BASE_PATH=/hanafubuki-agent/ bun run
        site:build` (every page prerendered) passed; the timing tests
        (`app::tests::queue`, the two review tests in `app::tests::features`,
        `app::tests::subagents`, the paste one) passed 10 runs in a row.
- [ ] Open:
      - A rejected turn start (the Agent stopped) drains the queue into
        failures one message at a time, as Codex does; each stays in the
        conversation with its error. Not changed.
      - A subagent left with messages queued drops them with its view, as its
        draft is dropped; Codex keeps each thread's input. Not ported.
      - Shift+Left and Alt+Up were tested through crossterm's events only, not
        on real Windows or macOS terminals, which may report them differently
        or keep them for themselves.

## Plan: a more customizable modular architecture, after KohakuTerrarium (2026-11-11)

The plan is [docs/plan-modular-architecture.md](docs/plan-modular-architecture.md).

**Sources.**

- KohakuTerrarium `c1d17b45`, read from source. Its architecture summary file is
  empty.
- Codex `57a808a392` (agent roles).
- kardia-core 0.2.3.

**What it adds.** It keeps the plugin mechanism (kardia, the registries, the guards,
the deny-only combinations) unchanged and adds declarative layers above it:

- typed plugin options;
- explicit context order;
- roles: inheritable, per-session agent definitions;
- a kernel-owned, narrowing-only capability scope;
- role-aware subagents;
- roles from packages;
- linked development packages.

**Status.** Every §9 decision (D1–D9) was accepted as recommended (2026-11-11), and
the plan records them. Every milestone, M1 to M7, has landed (2026-11-11); what is
still open is listed under each.

**For each milestone,** record when it is done:

- what landed;
- the validation commands that ran;
- whether the goldens changed;
- what Hanafubuki must change at its next pin.

None touches the `hanafubuki-agent-host` twin. Order: M1 and M2 are independent; M3,
then M4, then M5; M6 after M3; M7 after M1.

- [x] Decisions: D1–D9 of the plan's §9 answered. Done (2026-11-11): all accepted as
      recommended. Narrowing-only definitions (D1); descriptors for host rows (D2);
      `web` stays on the shared settings (D3); built-in `default` and `explorer`
      (D4); no host roles yet (D5); linked development packages (D6, so M7 is in
      scope); `agent_type` (D7); role switching between turns (D8);
      `[pluginOptions.<row>]` (D9).
- [x] M1: typed plugin options and row descriptors.
      - `[pluginOptions.<row>]`, decoded per key and tolerant, with strict writes.
      - The `PluginOptionSchema` vocabulary; a guest's `configSchema` lowered to it
        and enforced.
      - A descriptor per row, shown in `AgentPluginStatus`.
      - `subagents` limits become options, applied through `on_update`.
      - A generic options form in `/plugins` and `/config plugins`, with the
        coverage test extended.
      - Not model-visible.
      Done (2026-11-11):
      - The vocabulary (`PluginOptionSchema`, kinds, values, the tolerant
        `resolve` and strict `PluginOption::check`) and the `configSchema`
        lowering live in `hanafubuki-agent-packages` (`src/options/`), since
        the guest host may not depend on the configuration; the config crate
        re-exports them. `crates/config/src/plugin_options.rs` holds
        `[pluginOptions]`, layered per row and key with each value's layer,
        `AgentConfig::set_plugin_options` (checked before writing) and
        `PluginOptionKey`; `config set` refuses `pluginOptions.*` and a
        profile's `pluginOptions` unchecked, the CLI's `config get/set
        pluginOptions.<row>.<key>` check against the descriptors.
      - `crates/agent/src/host/descriptors.rs`: one descriptor per row (the
        core rows included); `AgentHostPorts::plugins` is
        `Vec<(AgentPluginRow, AgentPluginDescriptor)>`; a row without one
        refuses construction. `AgentPluginSettings::options_value`;
        `AgentPluginStatus` gains `summary` and `options`; option issues join
        the configuration warnings.
      - `subagents`: `maximumRunningAgents` (1–16, 3), `defaultWaitSeconds`,
        `minimumWaitSeconds`, `maximumWaitSeconds` (10–300; 30, 10, 300),
        `PluginDef::typed` from `from_settings`; `on_update` takes a new
        running limit in place and restarts for new wait bounds (which
        `wait_agent`'s spec states).
      - Guests: lowering issues are package issues and `plugins validate`
        issues; settings are checked before the guest receives them (fallback
        with a warning, file untouched); `set_package_settings` also checks
        the lowered bounds. `AgentPackageGuest.options`.
      - TUI: the generic options form (`config_center/options.rs`), `o` on
        the Built-in and Installed tabs, `/config plugins` lists each row's
        options form; coverage test
        `every_option_of_every_descriptor_has_the_generic_editor`.
      - Docs (both languages): config-reference (`#plugin-options`),
        configuration, config-layers, plugin-rules, terminal-interface,
        writing-plugins (`#config-schema`), embedding, host-plugins.
      - Validation: `cargo fmt --all --check`, `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`, `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`, `cargo
        xtask check`, `cargo test --workspace --all-features`; the site's
        `bun install --frozen-lockfile`, `format:check:site`, `site:check`,
        `site:test`, `site:build`. Goldens and history corpora unchanged; no
        diff under `crates/host`, `crates/foundation`, `crates/process`.
      - Hanafubuki at its next pin: its host rows supply descriptors (D2),
        and its `AgentPluginStatus`/`AgentPackageStatus` uses no longer
        derive `Eq` (a number option's bounds are `f64`).
      Open:
      - A profile's or a project's `pluginOptions` is written by hand only.
      - Guest option properties show in key order (the `configSchema`'s
        property order is not kept: `serde_json` sorts its maps).
      - A `secret` option's value would show in clear in
        `AgentPluginStatus.options` (and diagnostics); no option is secret
        yet.
      - `plugins validate` now reports a `configSchema` property that is not
        an option (a nested object), which fails it as any issue does.
      - `AgentPackageGuest.settings` is the checked settings: the JSON form
        and `plugins settings ID` show, and a save writes back, those.
- [x] M2: explicit context order.
      - `ContextRenderer.order`; requests ordered by `(slot, order, kind)`.
      - Ranks reproduce today's order, so the request goldens do not change.
      - A remounted plugin keeps its sections' place (new test).
      - Hanafubuki's host renderers take the 500–599 band.
      Done (2026-11-11):
      - `ContextRenderer.order: u16`, its position within its slot;
        `ContextRendererSnapshot::find` returns the renderer alone. The
        kernel (`sampling/prompt.rs`) orders each run of adjacent sections,
        world-state sections and prompt fragments alike, by `(slot, order,
        kind name)`; registration and mount order no longer show.
      - Ranks, reproducing the previous order: the kernel's
        `tool_catalog_omissions` 0; capability plugins in manifest order,
        `memory` 100, `skills` 110, `goal` 120, `subagents` 130, `review`
        140, `hooks` 150, `plugins` 160; host rows 500–599;
        `workspace-context` 700 (`workspace_instructions`,
        `workspace_roots`) and 701 (`environment`), `permission-mode` 710
        (`permission_mode`) and 711 (`sandbox`); every guest section and
        prompt 900 (`GUEST_ORDER`), so guests sort by kind among themselves.
      - Tests: `a_remounted_context_plugin_leaves_the_request_unchanged`
        (`golden.rs`: `workspace-context` off and on, then a new session's
        first request is the one before byte for byte and still
        `default_mode_request`; it failed before the change, with the
        roots and environment after the sandbox); the kernel's ordering
        test now covers ties broken by kind.
      - Docs (both languages): plugin-rules (`#context-renderers`, with the
        rank table; the registry table; the goldens), architecture
        (`#sections-slots-and-renderers`), host-plugins (the example sets
        `order: 500`; choosing an order in the host band), writing-plugins
        (a guest's sections come last in their slot, by kind).
      - Validation: `cargo fmt --all --check`, `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`, `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`, `cargo
        xtask check`, `cargo test --workspace --all-features`; the site's
        `bun install --frozen-lockfile`, `format:site`, `format:check:site`,
        `site:check`, `site:test`, `site:build`. Request goldens and history
        corpora unchanged; no diff under `crates/host`, `crates/foundation`,
        `crates/process` or `architecture/`.
      - Hanafubuki at its next pin: every host renderer sets `order` in
        500–599 (each its own, in its current registration order), and its
        goldens must not change.
      Open:
      - The bands are a convention: registration does not check that a
        renderer's `order` is in its owner's band.
      - An installed code plugin with two or more sections or a prompt in
        one slot now shows them in kind order, after the context plugins',
        not in declaration and start order: model-visible without a
        remount, as the plan's 900 band prescribes. No golden has a guest
        section, so none pins it.
- [x] M3: roles: definitions, sources, resolution.
      - `agents/*.toml` and `[agents]` in every layer, with Codex key aliases.
      - Precedence: built-in, then package, user, profile, trusted project.
      - `extends`, with the narrowing merge table and provenance.
      - `AgentRoleCatalog`; the built-in `default` and `explorer` (D4).
      - `roles list/show`, `doctor`, `/warnings`.
      - A new bilingual `roles` page.
      - No session uses roles yet.
      Done (2026-11-11):
      - `crates/config/src/roles/`: `AgentConfig::role_sources()` reads, per
        source and lowest first, the user's `agents/` (recursive, by path
        key) and `[agents]`, the profile's `[profiles.NAME.agents]`, and a
        trusted project's `.hanafubuki-agent/agents/` and `[agents]` (trust
        checked directly, so a project with only `agents/` counts); a host's
        configuration has the user's alone. `RoleDefinition` per role:
        camelCase keys plus Codex's `developer_instructions`, `model`,
        `model_reasoning_effort` (ours wins over an alias, with an issue),
        `configFile`/`config_file` relative to the layer's folder (the
        entry's keys over the file's; the file is then skipped by the
        directory, by `key()`). A wrong-typed key is left out with an
        issue; unknown and other Codex keys are ignored with an issue; a
        role name has no `:` (reserved for package roles). `agents` is
        exempt from the layers' unknown-key warnings (but reported under
        `-c`), and never a repair of `config.toml`.
      - Kernel `model/scope.rs` (re-exported by the kernel and the facade):
        `ToolPattern` (`Name` glob, `Plugin`, `Provider` glob; `parse`,
        `matches`, `Display`), `ToolIdentity`, `AgentToolScope { allow:
        Vec<BTreeSet<ToolPattern>>, deny }`, `AgentCapabilityScope { role,
        tools, plugins_off, permission_mode_cap }` with `narrow` and
        `stricter_permission_mode`. Not in `AgentSessionConfig`, no serde:
        M4 adds both.
      - `crates/agent/src/roles/`: `resolve` (built-ins, then each source;
        a higher source replaces a name whole, an unusable role included;
        within a source the first keeps it, the other is a duplicate
        issue), the chain (at most 8 roles; missing or unusable parent,
        cycle, self-extends: unusable with why), the merge table root
        first via `AgentCapabilityScope::narrow`, per-field provenance
        (field → deciding roles, nearest first), `AgentRoleCatalog`
        (`load`, `role`, `roles`, `unusable_role`, `unusable`, `issues`),
        built-in `default` and `explorer` (Codex's description adapted).
        The service keeps it (`AgentRoles`: compute under a rebuild gate,
        then assign), built at construction and rebuilt by every
        `reload_packages`, which each settings write and package change
        runs; `AgentApplicationService::roles()`.
      - CLI `roles list` / `roles show NAME` (no Agent start; `-p` adds the
        profile's roles), `doctor`'s `roles` lines (never `FAIL`), TUI
        `/warnings` source **roles**.
      - Tests: config `roles/tests.rs` (every key, tolerant keys, narrowing
        keys unusable, Codex aliases, names and stems, entries before
        files and `configFile`, profile and trusted project, a trusted
        project with only `agents/`, untrusted project and host, `agents`
        not an unknown key); kernel
        `model/scope/tests.rs`; facade `roles/tests.rs` (built-ins, the
        merge table field by field, a three-level deny's provenance, a
        seeded 500-chain monotonicity property, cycles, missing/unusable
        parents, depth, descriptions, duplicates, precedence, `load` and
        rebuild); CLI `management.rs`
        `roles_list_and_show_resolve_without_the_agent_and_doctor_reports_them`;
        TUI `warnings_list_the_issues_of_the_roles`.
      - Docs (both languages): new `roles` page (Configuration group, after
        config-reference in both sidebars), config-reference (`#agents`),
        config-layers (`#roles`), configuration (`agents/`), cli-reference
        (`#roles`, `doctor`, which commands start the Agent),
        troubleshooting (`roles` line), terminal-interface (`/warnings`),
        exit-codes, embedding (`roles()`), architecture (the CLI's commands
        that never start the Agent).
      - Validation: `cargo fmt --all --check`, `RUSTFLAGS="-D warnings"
        cargo check --workspace --all-targets --all-features`, `cargo clippy
        --workspace --all-targets --all-features -- -D warnings`, `cargo
        xtask check`, `cargo test --workspace --all-features`; the site's
        `bun install --frozen-lockfile`, `format:site`, `format:check:site`,
        `site:check`, `site:test`, `site:build` (the roles page prerendered,
        both languages' bundles built). Request goldens and history corpora
        unchanged; no diff under `crates/host`, `crates/foundation`,
        `crates/process` or `architecture/`.
      - Deviations from the plan: `tools.allow` resolves to a list of allow
        lists, every one of which must match (the plan's flat
        `Option<Vec<ToolPattern>>` cannot intersect globs or `plugin:`
        patterns exactly); provenance names every deciding role, not one
        source; `instructions_mode` and `permission_mode_cap` are not
        separate fields of `AgentResolvedRole` (the instructions are merged,
        the cap is in `scope`); a narrowing key (`extends`,
        `permissionModeCap`, `tools`, `plugins` and their lists) that cannot
        be read makes the role unusable instead of being dropped, since
        dropping it would widen the role; the description is required after
        the merge (the merge table inherits it); no package or host source
        kind exists yet (M6, D5).
      - Hanafubuki: none.
      Open:
      - There is no `config reload` command: the catalog is rebuilt at
        construction and by `reload_packages` (each settings write, package
        and MCP change); a hand-edited role file applies then or at the
        next start.
      - A user role named `default` replacing the built-in one: settled in
        M4, the name is reserved (a configured `default` is ignored with
        an issue).
      - `[agents]` is written by hand only: `config get/set` do not reach
        it.
      - The "pattern matches nothing" warning needs the live registries;
        M4 did not build it (see M4's Open).
- [x] M4: the session capability scope.
      - `AgentCapabilityScope` in `AgentSessionConfig`, tolerant and normalized.
      - Filters:
        - tools and sources at planning (core tools and `tool_search` never
          filtered; a filtered tool is "no tool named");
        - context, admission, prompt and continuation entries by owner;
        - the permission-mode cap.
      - Never filtered: approval rules, tool lifecycle hooks, observers.
      - Children narrowed; forks copy.
      - `create_session(role)`, `set_session_role`, `--role`, `/role`, the `role`
        event field.
      - Default sessions' requests and history bytes unchanged; one new explorer
        golden.
      Done (2026-11-11):
      - Kernel `model/scope.rs`: serde for the scope (a pattern as the text
        it parses from; empty fields left out; `Deserialize` field by field,
        an unreadable field at its default, an unreadable pattern dropped
        from its list, an allow list that is not a list allowing nothing),
        `is_empty`, `capped_permission_mode`, `turns_off`, `summary`
        (`AgentCapabilityScopeSummary`: allow lists and patterns, deny
        patterns, plugins off, cap), `normalize` (allow lists sorted and
        once each, `CORE_PLUGINS` dropped from `plugins_off`, a tool scope
        over `MAX_SCOPE_TOOL_PATTERNS` = 128 patterns or with one over 256
        bytes allows no tool); `narrow` keeps the parent's role when the
        child names none. `AgentSessionConfig` gains `scope` and
        `role_instructions`, both tolerant and `skip_serializing_if`
        empty, so a default session's record is byte for byte the old one;
        `normalize_session_config` normalizes the scope (creation,
        update, recovery, children); every request (and a compaction's)
        carries `effective_instructions()`: the base instructions, then the
        role's.
      - Filters: `AgentToolCatalogContext.scope`; the planner skips a
        source whose owner is off (`AgentToolContributor::owner`) and,
        after the mode policy, keeps a handler tool only if its owner is
        not off and the tool scope allows it (`provider` only for a sourced
        tool); core tools and `tool_search` have no handler and are never
        asked; no omission is recorded. `collect_world_state` and
        `collect_turn_extensions` take the scope and skip entries whose
        owner is off; prompt contributors
        (`PromptContributorSnapshot::without_plugins`, in the step and in
        `render_prompt`) and continuation hooks (the step's snapshot) the
        same; `stage_turn_admission` freezes the capped mode, and the
        facade's `turn_permission_mode` caps it too, so the
        `permission_mode` section and a child's inherited mode agree.
        Approval rules, tool lifecycle hooks, observers and renderers are
        untouched.
      - Children: `AgentChildSessionRequest.scope`; the child's scope is
        `parent.narrow(&request.scope)`; forks copy the configuration.
        Kernel `session_scope` and `set_session_role(session, scope,
        role_instructions)` (idle only, validated before it is assigned,
        a new revision, durable).
      - Snapshot `role` and `scope` (the summary), and the session summary's
        `role`, all left out for `default`.
      - Facade: `create_session(mode, role: Option<String>)` resolves from
        the current catalog (`default` → the empty scope, no instructions;
        unknown or unusable → invalid input naming the usable roles), takes
        the role's model and reasoning effort over the Provider policy's,
        and its scope and instructions; `set_session_role(session, role)`
        commits scope and instructions, then selects the role's model and
        reasoning effort if it sets them, and returns the snapshot. The
        name `default` is reserved: a configured `default` is ignored with
        an issue (M3's Open item).
      - CLI global `--role NAME` (interactive and `exec`); TUI `/role
        NAME`, `/role` alone a picker of the usable roles, `Role set to …`,
        the role (with what it narrows) in `/status`, a `role` status-line
        item; `TuiOptions.role`.
      - Tests: kernel `model/scope/tests.rs` (round trip, tolerant fields,
        canonical form, bounds, role kept by a child, cap, summary),
        `validation/tests.rs` (a default config's bytes, an old config, a
        role's record), `context/tests.rs` and `admission/tests.rs`
        (contributors of a plugin off not asked), `loop_contract/scope.rs`
        (a filtered tool is "no tool named" exactly like an unknown one,
        direct and deferred, with no omission; core tools and
        `tool_search` survive a scope naming them; a deferred selection of
        a filtered tool ignored after a switch; a fork keeps the scope; a
        tool lifecycle hook and an approval rule of a plugin off still
        deny and still ask under full access; prompt and continuation off;
        the cap at admission; `set_session_role` refused while a turn
        runs, `default` clears; a child never wider),
        `loop_contract/sources.rs` (a source of a plugin off never asked, a
        `provider:` deny); facade `tests/sessions/roles.rs` (creation with
        a role file, unknown role, reserved `default`, instructions after
        a model selection, the cap in the turn and in its `permission_mode`
        section, `set_session_role` and `default`,
        recovery after the role file is deleted, `skills` off refuses an
        explicit Skill, selects nothing for a `$mention` and drops the
        Skill tools, `memory` off drops the
        summary and tools while the contribution still reaches the store),
        `tests/descriptors.rs` (the core rows are `CORE_PLUGINS`),
        `tests/golden.rs` (every recorded corpus opens with no role and no
        scope), `roles/tests.rs` (a chain over 128 patterns unusable,
        `default` reserved and still extendable); CLI
        `exec.rs` (`--role explorer`, the summary's `role`, an unknown role
        exits 1 before any request); TUI command tables and the status
        line snapshot (`Role: explorer`).
      - Goldens: one new `explorer_request.json` (the default request with
        only explorer's six tools and the kernel's three; same
        instructions and input); every other request golden and the
        history corpora byte-identical.
      - Docs (both languages): plugin-rules (`#capability-scope`, the
        never-filtered list, a note under the registry table, the
        explorer golden), architecture (`#sessions-and-turns`,
        `#a-plan-per-step`), roles (`#sessions`, `default` reserved,
        when edits apply), sessions (`#roles`, children and forks, the
        summary's `role`), cli-reference (`--role`), exec (`--role`, no
        session object in the stream), json-events (`role` in
        `sessions list --json`), terminal-interface (`#role`, `/status`,
        the `role` item), embedding (`#sessions`: `create_session(mode,
        role)`, `set_session_role`, the snapshot; the cap in permissions).
      - Validation (Linux only): `cargo fmt --all --check`,
        `RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
        --all-features`, `cargo clippy --workspace --all-targets
        --all-features -- -D warnings`, `cargo xtask check`, `cargo test
        --workspace --all-features`; the site's `bun install
        --frozen-lockfile`, `format:site`, `format:check:site`,
        `site:check`, `site:test`, `site:build` (every page prerendered).
        One new request golden (`explorer_request.json`, generated alone
        with `HANAFUBUKI_AGENT_UPDATE_GOLDEN=1` on its test); every other
        golden and the history corpora byte-identical; no diff under
        `crates/host`, `crates/foundation`, `crates/process` or
        `architecture/`.
      - Deviations from the plan: the role's instructions are a field of
        their own (`role_instructions`), not part of `instructions` and not
        of the scope, so a model selection (which sets `instructions`
        again) and an unconfigured session (whose `instructions` must be
        empty) keep them, and the canonical-context digest keeps hashing
        the base instructions alone; the tool-pattern bound fails closed
        (allow nothing) instead of truncating, since dropping a deny
        pattern or an allow list widens; `exec --json` has no session
        object, so `role` went on the session summary
        (`sessions list --json`) and the runtime snapshot instead, the
        event stream unchanged; no snapshot event exists in the kernel,
        so `set_session_role` returns the snapshot as `set_session_model`
        does and the TUI refreshes its status from it; `set_session_role`
        with a model or effort is up to three commits (scope first, which
        holds the idle check); the facade caps the mode as well as the
        kernel, so the world state shows the frozen mode; a role whose
        chain holds more than 128 tool patterns is unusable at resolution
        (with why) rather than giving a session that allows no tool; with
        `skills` off a `$mention` is not refused, it selects nothing and
        stays text (the kernel only refuses an explicit reference); the
        new TUI `/role` file carries the Codex notice header, being
        modelled on `/model`'s picker (`architecture/` is unchanged).
      - Hanafubuki at its next pin: `create_session(mode, None)`; struct
        literals of `AgentSessionConfig` (`scope`, `role_instructions`),
        `AgentChildSessionRequest` (`scope`), and any of
        `AgentSessionRuntimeSnapshot`/`AgentSessionSummary` (`role`,
        `scope`) take the new fields; a direct call of
        `collect_world_state` or `collect_turn_extensions` passes a scope.
        Its goldens and the shared corpus do not change; it may offer role
        choice with `roles()` and `set_session_role`.
      Follow-up (2026-11-11): the kernel's `turn_started` event carries the
        session's `role` (`AgentEventKind::TurnStarted { role }`, the scope's,
        `None`/`null` for `default`), so `exec --json` names the role on its
        first line; json-events and exec (both languages) say so. Hanafubuki:
        a match on `TurnStarted` becomes `TurnStarted { .. }`.
      Follow-up (2026-11-11): the configuration center offers role choice:
        a **Session role** domain (`ConfigSection::Role`, `/config role` or
        `roles`), summarized as the session's role and the usable count
        (`ConfigState.role`, `roles`), opens `/role`'s picker by replaying
        `/role`, so `/role`'s own gates apply (not during a turn, a session
        made first) and the choice goes through `set_session_role`. Test:
        `config_center/tests.rs` (every domain parses from its key; the role
        domain replays `/role`). Docs: terminal-interface (`#config`'s
        sections and table, `#role`), both languages. Hanafubuki: none.
        Validation (Linux only): `cargo fmt --all --check`,
        `RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
        --all-features`, `cargo clippy --workspace --all-targets
        --all-features -- -D warnings`, `cargo xtask check`, `cargo test
        --workspace --all-features`; the site's `bun install
        --frozen-lockfile`, `format:site`, `format:check:site`,
        `site:check`, `site:test`, `site:build` (every page prerendered).
        Goldens and history corpora unchanged; no diff under `crates/host`,
        `crates/foundation`, `crates/process` or `architecture/`.
      Open:
      - The "a pattern that matches nothing" warning: the sourced tools
        vary per step, so it needs the live registries at selection; not
        built.
      - `set_session_role` is not atomic when the role sets a model: a
        failed model selection leaves the new scope with the old model.
      - An older build reading a scoped session runs it unscoped (the
        plan's migration table accepts it); a deny pattern a later build
        writes that this one cannot parse is dropped, which widens.
- [x] M5: role-aware subagents.
      - `spawn_agent { agent_type }` (D7).
      - `AgentRoleCatalogReader`; children take the role's instructions, model and
        narrowed scope.
      - The guidance lists spawnable roles (at most 16, sorted).
      - Model-visible: regenerate the goldens on purpose, here and in Hanafubuki.
      Done (2026-11-11):
      - `crates/agent/src/roles/reader.rs`: `AgentRoleCatalogReader`, built by
        the application over its `AgentRoles` (now loaded before the plugin
        manifest) and passed to the `subagents` row beside the driver;
        `spawnable()` (name and description, by name) and `spawn_role(name)`
        (what the child takes, or the spawnable names), both reading the
        catalog at each call. `AgentSessionRole` moved to `roles/mod.rs` with
        `AgentResolvedRole::session_role` (`default` → nothing), which the
        facade's `session_role` shares.
      - `spawn_agent`: optional `agent_type` (absent or empty → `default`);
        resolved before any check or creation; unknown, unusable or not
        spawnable → `invalid tool argument agent_type: … use one of <the
        spawnable roles>`, no child made. The child is created in one
        commit with the role's scope (the kernel narrows the parent's) and
        instructions; then, if the role sets a model or an effort,
        `AgentSessionDriver::select_role_model`: the role's model of the
        parent's Provider through the model selection, and the role's
        effort, else the parent's (kept across the model change, which
        would otherwise reset it to the Provider policy's). Origin
        `Subagent`, title the task name, the parent's permission mode,
        `maximumRunningAgents` (M1) bounding the running ones, as before.
      - Kernel: `AgentChildSessionRequest.role_instructions`; a child takes
        them in place of its parent's when `request.scope.role` names a
        role, and keeps its parent's role instructions otherwise.
        `create_child_session` now also runs `validate_session_config` on
        the child's configuration, since those instructions do not come
        from the (valid) parent.
      - `AgentSessionDriver` is `session_driver/mod.rs` plus `model.rs`:
        `select_model` (the facade's `set_session_model` now delegates to
        it; the driver carries the base instructions) and
        `select_role_model`.
      - `list_agents` reports `agent_type` per agent (the session's role,
        `default` when none); `ListedAgent` and the runs keep it.
      - Guidance (`subagent_guidance`) and `agent_type`'s schema
        description list the spawnable roles, a line each (the name in
        backticks, then the description): at most 16, in name order,
        each description's whitespace collapsed to one line and cut at 200
        characters on a character boundary (`…`). The "your model, your tools" wording of the guidance and of
        `spawn_agent`'s description now says the role may change them; the
        dropped-`agent_type` note of `tools.rs`'s header is gone.
      - Tests: `plugins/subagents/tests/roles.rs` (the suite moved to
        `tests/mod.rs`): an unknown and a non-spawnable `agent_type` are
        the same field-named error listing `default, explorer, lead`, with
        no child and no request; a `lead` parent spawning `explorer` gets
        the intersection of the two tool lists and no lead instructions,
        while a default child keeps the lead's tools, instructions and role,
        and `list_agents` names `explorer` and `lead`; a role's model and
        effort, else the parent's (the effort kept across a model change),
        and the role's instructions after the base ones past the model
        change; a role written after start and `reload_packages` is
        spawnable and in the next step's guidance; 20 roles written in reverse order plus a non-spawnable one: the
        guidance and the schema list `default`, `explorer`, `r00`–`r13`,
        each cut at 200 `é` and `…`, no second line, no hidden role.
        Kernel `persistence_contract/children.rs`: a child whose role
        instructions are over the bound is refused (`LimitExceeded`) and
        not made.
      - Goldens regenerated on purpose
        (`HANAFUBUKI_AGENT_UPDATE_GOLDEN=1 cargo test -p hanafubuki-agent
        --lib golden`): 14 request fixtures changed, every hunk one of the
        new `subagent_guidance` text (role wording plus the `default` and
        `explorer` list), `spawn_agent`'s new description wording, or its
        new `agent_type` property; `plan_mode_request.json`,
        `tool_round_trip_events.json`, the `recorded_*` transcripts and the
        history corpora are byte-identical.
      - Docs (both languages): roles (`#subagents`, the intro, the
        `spawnable` key), sessions (`#children`, `#subagents`, the
        running limit's option), tools (`#subagents`: `agent_type`, the
        role in `list_agents`, the guidance), architecture (lineage, the
        session driver, "Subagents"), plugin-rules (the feature calls'
        ports, the `subagents` update rule, `#capability-scope`, the
        `subagents` row of "The plugins today", a table prettier reflowed
        in the Chinese page), internals (child sessions).
      - Validation (Linux only): `cargo fmt --all --check`,
        `RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
        --all-features`, `cargo clippy --workspace --all-targets
        --all-features -- -D warnings`, `cargo xtask check`, `cargo test
        --workspace --all-features`; the site's `bun install
        --frozen-lockfile`, `format:site`, `format:check:site`,
        `site:check`, `site:test`, `site:build` (every page prerendered,
        the new `#subagents` section in both languages' bundles). No diff
        under `crates/host`, `crates/foundation`, `crates/process` or
        `architecture/`.
      - Deviations from the plan: without `agent_type` (or with
        `default`) a child keeps its parent's role, scope and role
        instructions, not "base plus `default`'s" (none), so a lead's
        default helper is still a lead; the role instructions reach the
        child through a new kernel request field rather than a second
        commit (`set_session_role` would assign the scope un-narrowed);
        the model selection moved from the facade into the driver, which
        both share; besides adding the list, the guidance's and
        `spawn_agent`'s "your model, your tools" sentences were reworded;
        `list_agents`' role field is named `agent_type`, as the argument;
        the tools page was updated too.
      - Hanafubuki at its next pin: its integration goldens change (the
        guidance and `spawn_agent`'s spec); regenerate and review them.
        Struct literals of `AgentChildSessionRequest` take
        `role_instructions: None`.
      Follow-up (2026-11-11): `agent_type`'s description lists no role (it
        points at the guidance, which is read per step), so no spec goes
        stale; the guidance is shown only when the step's tool plan offers
        `spawn_agent`, from the kernel's new `PromptStep.tools` (the names
        the plan offers, Hidden ones left out), so the `explorer` golden
        lost it. Request goldens regenerated on purpose: 17 `agent_type`
        hunks and the `explorer` guidance; history corpora unchanged.
        Hanafubuki: `PromptStep` gains `tools` and is no longer `Copy`;
        regenerate its integration goldens.
      Open:
      - A role's model selection that fails (its Provider gone or
        disabled) fails the spawn after the child exists: the agent is
        listed, idle, with its parent's model, and takes follow-up tasks.
      - With more than 16 spawnable roles, the rest are not listed but can
        still be spawned by name.
      - The 200-character cut leaves `explorer`'s "Rules:" (how to use
        explorers) out of what the model reads; its description could be
        reordered or shortened.
- [x] M6: roles from packages.
      - The `agents` component of our format.
      - Claude Code `agents/*.md`, with the tool-name table.
      - `<plugin>:<role>` ids, present while the package is enabled.
      - Update `docs/plan-plugin-system.md` §3.3 and §10.
      Done (2026-11-11):
      - Packages crate: `ManifestPaths.agents`, read by the legacy reader
        (ours, Codex, Claude, Cursor) like `commands`; Agent Plugins has
        none. `claude_agents.rs`: `read_claude_agents(paths)` (each a
        directory of `.md` files, recursive to depth 8, or one file; by
        `ResolvedPath` key, each once) returns `ClaudeAgent { file, name,
        description, instructions, tools, unusable, issues }` and the
        files that are no agent; frontmatter key by key (a wrong type left
        out with an issue; other keys ignored, as the Skill catalog does,
        but Claude's narrowing `disallowedTools` and `permissionMode`
        reported as not read), the body the instructions; `tools` a
        comma-separated string or a YAML list, mapped by the §3.6 table
        (`mcp__<server>__<tool>` unchanged), an unknown name left out with an issue, so a list of
        unknown names is `Some([])` and allows nothing; a `tools` that is
        neither text nor a list makes the agent unusable; `model: inherit`
        honoured, any other value ignored with an issue. Frontmatter that
        is not YAML gets one repair (a top-level plain value holding `: `
        quoted) before it is an issue. `commands`' frontmatter split is
        now `split_frontmatter`, shared.
      - Config crate: `read_role_files(paths) -> RoleFiles { definitions,
        issues }`, the M3 reader over a plugin's directories or files
        (bare names; `discover::read_paths`, sharing `read_definitions`
        with `read_directory`).
      - Facade `packages/roles.rs`: when a package is active, the
        manifest's `agents` (else `agents/` if it exists) as role files
        (our format) or agent files (Claude); a Codex, Cursor or Agent
        Plugins package with a declared `agents` gets an issue. Each role
        is renamed `<plugin>:<role>` (the id's plugin name) and its read
        issues become the package's (`role `<name>`: …`).
        `LoadedPackage.roles`; `resolve_role_names` (beside
        `resolve_server_names`): among enabled packages with roles, the
        first by id keeps a plugin name, a later one brings none, with an
        issue. `LoadedPackages::roles()` → `AgentPackageRoles { id,
        definitions }`.
      - Facade roles: `AgentRoleSource::Package { id, origin }` (`plugin
        `<id>` file …`); `resolve(packages, sources)` adds the packages'
        definitions after the built-ins and before the configuration's
        (`add_source`, the one per-source rule: `default` reserved, the
        first of a name keeps it), through the same candidate, chain and
        narrowing merge. `AgentRoleCatalog::load(config)` stays
        configuration-only (CLI `roles`, `doctor`); `load_with(config,
        packages)` is the service's. `AgentRoles::load(config, packages)`
        at construction (after the packages opened) and `rebuild(config,
        || packages.loaded().roles())`, read under the rebuild gate, by
        every `reload_packages`.
      - `AgentPackageContent.roles` (names); CLI `plugins info` prints a
        `roles:` line; TUI `/plugins` page a **Roles** row. `plugins
        validate` also reads the roles and lists what they leave out, so
        (failing on any issue, as it does) it now fails a plugin whose
        agent file has, say, `model: sonnet`, as `--run` would through the
        package's issues anyway.
      - Tests: packages `claude_agents/tests.rs` (every key, the whole tool
        table, unknown tools left out, a list of unknown tools allows none,
        no `tools` and an unreadable `tools`, `model: inherit` and others,
        a wrong-typed key left out alone, a description holding `: `,
        files without frontmatter or YAML, paths read once),
        `tests/packages/manifest.rs` (`agents` paths); config
        `roles/tests.rs` (`read_role_files` over directories and files);
        facade `roles/tests.rs` (a package role between built-in and user,
        narrowed by `explorer`, a within-package duplicate, an empty allow
        list allowing nothing, a user role extending it, `rebuild` with
        packages), `tests/packages/roles.rs` (enable and disable add and
        remove a Claude and an own-format package's roles, the package's
        issues, `content.roles`, a session in `demo:reviewer` offered only
        its tools with its instructions, the guidance listing it, refused
        once off; two `demo` packages from two marketplaces; `plugins
        validate` lists a role's issue); TUI `commands/plugins/tests.rs`
        (the Roles row).
      - Docs (both languages): roles (`#plugins`, the sources list, the
        `:` note, `#reload`), writing-plugins (the `agents/` row and
        field, `#roles`, `#claude-agents` with the table, `#validate`), architecture
        (`#installed-plugins`: the roles bullet and the reload), plugins
        (intro, `#install`, `#lifecycle`), terminal-interface (`#plugins`:
        the page's rows; `/warnings`' installed plugins), cli-reference
        (`#plugins`, `plugins info`, `#roles`), config-reference (the
        `plugins` row), embedding (`roles()`, `AgentPackageContent`). `docs/plan-plugin-system.md` §3.3 and §10
        point at this plan for `agents/`.
      - Validation (Linux only): `cargo fmt --all --check`,
        `RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
        --all-features`, `cargo clippy --workspace --all-targets
        --all-features -- -D warnings`, `cargo xtask check`, `cargo test
        --workspace --all-features`; the site's `bun install
        --frozen-lockfile`, `format:site`, `format:check:site`,
        `site:check`, `site:test`, `site:build` (every page prerendered,
        the new sections in both languages' output). Request goldens and
        history corpora unchanged (the installed-plugin golden's package
        has no `agents/`); no diff under `crates/host`,
        `crates/foundation`, `crates/process` or `architecture/`.
      - The §3.6 table checked: `read_file`, `search_text`,
        `list_directory`, `apply_patch` (workspace-fs), `exec_command`
        (execution), `fetch_web_page`, `web_search` (web) all exist, and an
        MCP tool is `mcp__<server>__<tool>` (`flattened_alias`). No
        mismatch, but the server part is the MCP crate's `safe_identifier`
        of the server id (a character other than ASCII letters, digits, `-`
        and `_` becomes `_`), so a Claude name written with such a
        character matches nothing.
      - Deviations from the plan: a `tools` that is neither text nor a
        list makes the role unusable instead of being dropped (as M3 does
        for narrowing keys: dropping it would allow every tool); the
        duplicate rule is per plugin name, not per role: the later package
        brings no role at all; the prefix is the id's plugin name (the
        validated one), not the manifest's `name` that Skills use; what
        reading a package's roles leaves out is the package's issue
        (`plugins info`, `/plugins`, `plugins validate`), what resolving
        them finds is the catalog's (`/warnings`); `RoleLayer` gained no
        variant (the facade's `AgentRoleSource` did); a declared `agents`
        replaces `agents/`, as `skills` and `commands` do here (Claude Code
        adds declared paths to its defaults); unknown frontmatter keys are
        silent but for Claude's two narrowing ones; `plugins validate`
        reads roles too; the
        frontmatter repair is a small one of the packages crate's, since it
        cannot use the Skill parser's.
      - Model-visible only through M5's guidance and `agent_type` when a
        spawnable package role exists; the goldens have none.
      - Hanafubuki: none required. Compile-time only if it matches
        `AgentRoleSource` exhaustively (new `Package` variant) or builds
        `AgentPackageContent` or `ManifestPaths` literals (new `roles`,
        `agents`).
      Follow-up (2026-11-11): `disallowedTools` → deny list, `permissionMode` → cap (`plan` also denies `PLAN_MODE_DENIED_TOOLS` and, without `tools`, `mcp__*`).
      Follow-up (2026-11-11): a package role enabled later needs no
        `subagents` restart: `agent_type`'s description lists no role (see
        M5's follow-up). A session in a plugin role that leaves
        `spawn_agent` out (every Claude agent with `tools`: `Task` has no
        counterpart) no longer reads the sub-agent guidance;
        `tests/packages/roles.rs` checks the plugin roles' listing from a
        default session.
      Follow-up (2026-11-11): `roles list`, `roles show` and `doctor` show
        the installed plugins' roles. Packages crate:
        `PackageStore::read_installed(root)`, the store's listing without a
        store (no staging sweep, a record file that does not read not
        quarantined, nothing written, no lock); `installed()` shares it.
        Facade: `AgentRoleCatalog::load(config, plugins_root, safe_mode)`
        (was `load(config)`) reads them through
        `packages::read_installed_roles`: the packages the settings leave
        on (`package_enabled`, none in safe mode), by id, each read with
        `packages/roles.rs` alone (no `content::load`, which makes the data
        directory), and the one-package-per-plugin-name rule
        (`roles::resolve_names`, now shared with the running load); what
        reading them left out joins the catalog's issues as `plugin `<id>`:
        …` on this path only. CLI `roles` and `doctor` pass the data
        directory's `plugins/` and `--safe-mode`. Test: CLI
        `tests/plugins.rs` (a Claude plugin with an agent installed twice
        and a disabled one: `roles list`/`show` and `doctor` list
        `demo:reviewer` from `demo@local`, not the disabled plugin's, the
        model issue and the second package's issue on standard error and
        in `doctor`; none with `--safe-mode`; `plugins/` byte for byte
        unchanged, a planted `.staging/marker` and a removed `data/`
        included, and again with an `install.json` that does not read).
        Docs (both languages): roles (`#plugins`), cli-reference (`#roles`,
        `#agent-start`), troubleshooting (`#doctor`), architecture (the CLI
        crate), embedding (`AgentRoleCatalog::load`). Hanafubuki: a call of
        `AgentRoleCatalog::load(&config)` becomes `load(&config,
        &storage_paths.plugins_root, safe_mode)`.
        Validation (Linux only): `cargo fmt --all --check`,
        `RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
        --all-features`, `cargo clippy --workspace --all-targets
        --all-features -- -D warnings`, `cargo xtask check`, `cargo test
        --workspace --all-features`; the site's `bun install
        --frozen-lockfile`, `format:site`, `format:check:site`,
        `site:check`, `site:test`, `site:build` (every page prerendered).
        Goldens and history corpora unchanged; no diff under `crates/host`,
        `crates/foundation`, `crates/process` or `architecture/`.
      Open:
      - No test spawns a sub-agent with `agent_type: "demo:reviewer"`: the
        spawn looks the name up in the same catalog (`spawn_role`, no
        check of `:`), and the guidance listing it is tested.
      - The guidance lists at most 16 spawnable roles by name, so a plugin
        with many agent files can push `explorer` out of the list (still
        spawnable by name).
      - Nearly every Claude agent file sets a Claude `model`, so most
        Claude plugins with agents carry a standing `model` issue in
        `/plugins` and `/warnings`, and fail `plugins validate`.
      - `tools:` with no value reads as absent (YAML `null`), so it
        narrows nothing.
      - An `extends` inside a package names the full `<plugin>:<role>`;
        a bare name is looked up among every source's roles.
      - M7's linked packages will need the roles read again on their
        change signal (the reload already does it).
      - Windows and macOS not run; the new end-to-end tests are not
        `cfg(unix)`, and `set_enabled` is no longer gated.
- [x] M7 (D6): linked development packages.
      - `plugins link <dir>`.
      - A contained watcher that restarts the guest and re-reads its content.
      Done (2026-11-11):
      - Packages crate: `PackageSource::Linked { path }` (`{"type":
        "linked", "path": …}` in `install.json`, the resolved directory's
        `native()` form); `InstallRecord::linked`, `InstalledPackage::linked`.
        `store/link.rs`: `PackageStore::link_directory(dir, id) ->
        LinkOutcome { id, root, issues }` (`ResolvedPath`; the manifest's
        name must be the id's; no copy, no `contentHash`, nothing written
        into the directory, so `commands/` are not migrated, with an
        issue; a re-link keeps the grant, as an install does). `installed()`
        and `read()` list a linked record after the cache's packages, read
        as the directory is now (its manifest's version, else `local`; a
        directory that does not read is an issue, the package still
        listed). A link refuses an id in the cache, and an install refuses
        a linked id. `uninstall` removes a linked record (`removed: true`)
        and never touches the directory. A marketplace entry cannot carry
        the variant (refused, typed).
      - Guest host: `linked.rs`, `LinkedWatch`: `GuestHost::follow_linked(
        Vec<LinkedPackage { id, root }>)` (code or content; a changed set by
        id and `PathKey` replaces the watch), started at the supervisor's
        mount, stopped at its disposer. The watch is a thread
        (`plugin-linked-watcher`) of the instance's contained worker group
        (`GuestHostInputs.workers`, the service's
        `contained_with_panic_handler` group, so a panic is a kernel fault
        and the group's shutdown joins it): one recursive `notify` watch
        armed on that thread on each directory that is one; a directory
        missing, removed (its `Remove` event) or gone is tried again every
        100 ms and its reappearance is a change, so an export that replaces
        the directory is followed; `Access` and paths in `.git` ignored, an
        unattributed event counts for every directory; 500 ms debounce; the
        token checked every 100 ms. Each
        changed directory's revision counts one more, and
        `GuestHost::linked_changes()` (a `watch` counter) moves.
        `GuestPackage.linked`: `check_content` passes (no pin); the entry
        name digests `linked <revision>` (an installed package's name is
        unchanged), so `Shared::follow` (now comparing the record's
        `revision` too) gives a new epoch, and `reconcile` a new name, which
        kardia-loader answers by disposing the fiber and starting the new
        plugin (a WASM module compiled anew). The tools' identity and the
        hooks' generation take that entry name.
      - Facade: `packages/reload.rs`, `PackageReload { packages, config,
        blocking, roles, faults }`: `run()` is what `reload_packages` did
        (the packages, then the role catalog; the service's `reload_roles`
        is gone), `spawn()` what `spawn_package_reload` did; both service
        calls now build and run it. `AgentPluginInputs.packages` is a
        `PackageReload`; the `plugins` row's scoped worker (`plugins-linked`)
        waits on `linked_changes` and spawns `PackageReload::spawn` (a
        changed burst during a reload is one more reload). The facade hands
        the guest host the active linked packages (`LoadedPackage.linked`,
        `LoadedPackages::linked()`, set for every active linked package,
        even one whose manifest does not read, so its repair is seen) at
        open and at every reload.
        `AgentPackageSource::Linked { path, marketplace }` through
        `install_package` (an `Installed` report shared with installs);
        `update_package` refuses a linked package before the marketplace
        arm; uninstall is the unlink.
      - CLI `plugins link DIR [--marketplace NAME] [--json]`; `plugins
        remove` unlinks (the minimal surface: no `unlink`); `list`/`info`
        source `linked <dir>`; TUI `/plugins` source `Linked · <dir>`.
      - Tests: packages `tests/packages/link.rs` (recorded by reference, no
        cache, nothing written, read as it is now, a directory that is
        gone; remove keeps the directory, a re-link keeps the grant; link
        and install of one id refuse each other, wrong name, missing
        directory); guests harness `linked.rs` (a linked guest starts with
        no hash, signals no sooner than ~the debounce after a change, the
        packages applied again start it anew, unchanged packages keep it,
        a directory replaced by a moved-in copy is watched anew, an
        unlinked directory no longer signals); facade
        `tests/packages/linked.rs` (a role file added to the linked
        directory reaches `roles()` and `content.roles` with no call while
        the `plugins` row runs, a manifest saved broken drops the roles and
        its repair brings them back (checked to fail without the fix); `update_package` and a directory install
        refused; uninstall keeps the directory and clears the entry);
        `tests/plugin_guests.rs` (Linux, sandboxed: a linked code plugin's
        guest starts anew on its own once a file in its directory
        changes); CLI `tests/plugins.rs` (link, list as linked, a new Skill
        read, update and add refused, remove keeps the directory). The
        timing tests poll with bounds (a write repeated every 3 s until it
        is seen, within the test's deadline, since the OS watch arms on its
        own thread); the fixed waits are the negative check (2 s after an
        unlinked write) and 1 s of quiet between two changes in the
        guests harness.
      - Docs (both languages): writing-plugins (`#link` under `#dev`),
        cli-reference (`plugins link`, `remove` unlinks, `update` refuses),
        plugins (`#install`, `#lifecycle`), architecture
        (`#installed-plugins`: a link is a record; `#code-plugins`: the
        entry name, what a start needs, linked packages follow their
        directory, writing one), internals (`#plugin-guests`: the load,
        the content check, the linked packages bullet), embedding
        (`AgentPackageSource::Linked`, uninstall, update, `reload_packages`),
        plugin-rules (the guest host and `plugins` rows), data-directory
        (`install.json`). The plan's header and its `install.json`
        migration row.
      - Validation (Linux only): `cargo fmt --all --check`,
        `RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
        --all-features`, `cargo clippy --workspace --all-targets
        --all-features -- -D warnings`, `cargo xtask check`, `cargo test
        --workspace --all-features`, the new watcher tests run again
        several times; the site's `bun install --frozen-lockfile`,
        `format:site`, `format:check:site`, `site:check`, `site:test`,
        `site:build` (all 35 slugs prerendered, `#link` in both
        languages). Request goldens and history corpora unchanged; no
        diff under `crates/host`, `crates/foundation`, `crates/process` or
        `architecture/`; no module-size exception added or raised.
      - Deviations from the plan: the restart is the existing upgrade
        path, a new loader entry name (kardia-loader disposes the old
        fiber and starts the new plugin); this codebase never calls
        kardia-core's `registry().replace`, which kardia-hmr's own reload
        uses. The Skill watcher's debounce is 250 ms, not 500 ms; the
        linked watch uses the plan's 500 ms and the Skill watcher is
        unchanged. The watch lives with the guest host (the `plugins` row),
        so with that row off a linked package's content is read again only
        by other reloads. The migration row was wrong for this design: an
        older build does not list a linked package (it has no cache
        directory) and its next record write nulls the source (the plan's
        row now says so). `notify` 8 is now a dependency of the guest host
        too (the facade's and the CLI's version). Unlink is `plugins
        remove`, no separate command.
      - Model-visible: nothing (goldens unchanged).
      - Hanafubuki: none required. Compile-time only if it matches
        `PackageSource` or `AgentPackageSource` exhaustively (new `Linked`
        variants) or builds `GuestHostInputs` (new `workers`) or
        `GuestPackage` (new `linked`) literals; it does none today.
      Follow-up (2026-11-11): linked packages are recorded in
        `plugins/linked.json` (`{"packages": {"<id>": {path, version,
        format, grantedPermissions}}}`, read and quarantined as
        `install.json` is, an entry without a `path` skipped), so
        `install.json` holds the cache's packages alone and an older build's
        rewrite of it (dropping the entry or nulling its source) no longer
        loses a link. The store keeps the two files' records apart
        (`record::Records`): the cache decides which is the package when
        both name an id, a grant goes to the link unless the id is cached,
        uninstall clears both, and a write writes `linked.json` first (only
        when there is a link or the file exists). An `install.json` entry
        of source `linked` from the first build is read as a link (unless
        `linked.json` has the id) and moved by the store's next write; a
        read alone moves nothing. `PackageSource::Linked` and
        `InstallRecord::linked` stay as the in-memory record a linked
        `InstalledPackage` carries. Tests: packages `tests/packages/link.rs`
        (the record in `linked.json` and not in `install.json`; a link, its
        grant and the install refusal survive an `install.json` rewritten
        with `"source": null` and with the entry dropped; a grant then goes
        to the link; the first build's entry moved at the next write;
        `read_installed` lists what the store lists and leaves a staging
        marker and an unreadable `linked.json` in place). Docs (both
        languages): writing-plugins (`#link`), data-directory (`plugins/`),
        architecture (`#installed-plugins`, what a start needs), internals
        (the load); the plan's migration row and M7 lines,
        `docs/plan-plugin-system.md`'s store layout. This supersedes the
        deviation above that an older build's next write nulls the source:
        that write no longer reaches the link. Hanafubuki: none.
        Validation (Linux only): `cargo fmt --all --check`,
        `RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
        --all-features`, `cargo clippy --workspace --all-targets
        --all-features -- -D warnings`, `cargo xtask check`, `cargo test
        --workspace --all-features`; the site's `bun install
        --frozen-lockfile`, `format:site`, `format:check:site`,
        `site:check`, `site:test`, `site:build` (every page prerendered).
        Goldens and history corpora unchanged; no diff under `crates/host`,
        `crates/foundation`, `crates/process` or `architecture/`.
      Open:
      - The watch is recursive over the whole directory, a build output
        (`target/`, `node_modules/`) included: a big tree costs inotify
        watches (and can hit `max_user_watches`), and a build's writes
        restart the guest once the build is quiet. Nothing is excluded but
        `.git`.
      - A change made before the OS watch armed (just after the Agent or
        the `plugins` row started) is not seen until the next change.
      - The hook trust's content hash does not skip `.git` while the watch
        does: a commit in a linked directory makes its hooks `modified`
        without a reload (the next listing or run sees it).
      - A directory replaced so fast that neither its `Remove` event nor
        the 100 ms check sees it gone would keep a dead OS watch; the
        moved-in copy case is tested, other platforms are not.
      - Every change of a linked package with hooks makes them `modified`
        until trusted again (the hook trust hashes the package), by design
        ("trusted exactly as an installed one"), which is tedious while
        developing hooks.
      - `plugins dev` still reinstalls on change; it and `link` overlap,
        and `dev` on a linked id fails (an install refuses a linked id).
      - A linked package whose manifest renames the plugin keeps its id;
        nothing reports the mismatch.
      - Windows and macOS not run (`notify` uses ReadDirectoryChangesW and
        FSEvents there; the sandboxed end-to-end check is Linux only).
        CI (2026-11-11, `e8014c1`, macos-26-arm64): the facade's
        `a_linked_packages_change_is_read_again_while_the_plugins_row_runs`
        failed once ("…/agents/planner.md was not read"): none of the
        writes, every 3 s for the 60 s `TEST_WAIT` from ~0.2 s after the
        test started, reached the roles, though for the last 30 s no other
        test of the binary ran; the run before and the re-run passed. The
        cause is not confirmed: no macOS machine, and the test said nothing
        but its timeout. Ruled out: a lost wakeup (`linked_changes` is a
        `watch` counter, which keeps a change the row's worker has not
        seen); a reload held behind a gate (with no guest entries
        `reconcile`'s group update awaits nothing, `set_plugin_servers`
        returns at once with no servers, nothing else takes the packages'
        reload lock); a stale role rebuild (it reads the packages under its
        own gate); a disarm never re-armed (every disarm is re-armed at the
        next 100 ms poll, and that counts as a change); leaked watchers or
        OS watches (at most one inotify instance and two Skill watcher
        threads at a time, sampled every 2 s over a full run of the facade's
        tests on Linux). Most likely, by elimination from reading and Linux
        runs only, so unconfirmed: the OS watch, an FSEvents stream armed but
        silent, or `watch()` not returning (notify 8 starts the stream in
        it; the facade test now tells the two apart). Done: the watch records which linked directories it follows now (a set
        per run, `GuestHost::linked_watched()` under `test-support`); the
        facade test and the guests harness wait for it before their first
        write (which no longer races the arm, saving the 3 s retry), and
        on a timeout the facade test names the stage: the `plugins` row's
        state, the directories watched, the changes counted, the package's
        roles and issues as last read, and `roles()` (the guests harness:
        the directories watched and the changes counted). "Watched, no
        change counted" does not tell a silent stream from events that never
        go quiet for the debounce; the latter is excluded by reading only
        (nothing writes in the directory, and FSEvents reports no reads).
        Production behaviour is unchanged. Still open: the facade test is
        skipped on macOS (`cfg_attr(target_os = "macos", ignore)`, with a
        TODO there and in `linked.rs`'s `watch_directories`) until the cause
        is found. Not done, for a next failure that names the
        stream: notify 9 (no root-only `FSEventsPurgeEventsForDeviceUpToEventId`
        on every stream stop, no `CFRunLoopIsWaiting` spin in `stop()`;
        still a release candidate), or re-creating the OS watch on a timer
        (on notify 8 each re-creation blocks the worker in `stop()` while
        the purge call fails, and on inotify re-walks the whole tree).
      - Windows (2026-11-12): the guests harness's linked test failed twice
        in a row on Windows CI at its last step ("no signal within 60s;
        watched {demo@local}, changes counted 2"): a change in a directory
        replaced by another (removed, a copy moved in) was never seen. The
        watch noticed a replacement only by a `Remove` event naming the
        root or the path not being a directory at a poll; Windows' watch
        follows the directory it opened, reports the old files' removal
        but not its own, and the path is a directory again within the same
        poll, so the watch stayed armed on the removed directory. Fixed: an
        armed directory records its identity (device and inode on Unix,
        creation time elsewhere) and is disarmed, then re-armed, when the
        path's identity differs (`linked/tests.rs` reproduces it on any OS
        with a watcher that reports as Windows' does). Whether this is
        also the macOS case above is not known; its skip stays.

## Plan: a configuration center for every Agent setting (2026-11-11)

The plan is [docs/plan-configuration-center.md](docs/plan-configuration-center.md);
the implementation handoff is recorded in
[docs/plan-configuration-center-handoff.md](docs/plan-configuration-center-handoff.md).
Each phase was implemented and validated separately; C1–C6 are committed
(`e4dbb98`, C6's documentation, is the last). Superseded (2026-11-12) by
[docs/plan-settings-and-timeline.md](docs/plan-settings-and-timeline.md),
whose S1–S7 replace these hand-written forms with a settings catalog. The work
turns `/config` into a searchable configuration center over the typed Agent,
standalone, Provider, MCP, plugin and interface settings while preserving the
existing layered configuration and atomic writers.

- [x] C1: state, `/config` routing, full-screen navigation, search and
      layer summaries. Done: `ConfigState` reads the facade settings plus
      standalone settings, layer sources and warnings inside the TUI bridge;
      `ConfigCenter` is a full-screen searchable overview with per-domain
      summaries and layer/warning counts; `/config <section>` selects its
      domain and `/config quick` keeps the previous quick editor while the
      full forms are built.
- [x] C2: every `config.toml` domain form, typed atomic saves and
      override warnings. Done: General, Sandbox, Memory, Web, Compaction,
      Network/logging and Interface have whole-domain forms; saves go
      through `AgentSettingsPatch`, the running Agent's standalone patch or
      `AgentTuiPreferencesPatch`; proxy credentials stay secret; comments
      and unknown keys survive; a coverage test pins every scalar setting
      to a form or specialized editor; and a write a higher layer replaces
      says so.
- [x] C3: the complete shared Provider model policy, including turn
      budgets and revision conflicts. Done: `/providers` gained a `p` policy
      stage for the default Provider and model, reasoning effort, image
      input, context window, automatic and remote compaction, and every
      turn-budget field; stale revisions reload and keep the form.
- [x] C4: MCP protocol mode and per-tool concurrency, approval and
      exposure policies. Done: the server form edits `protocolMode`, a
      concurrency row per tool and approval/exposure override rows; values
      are validated before saving; round-trip and invalid-input tests cover
      every new field; the form says `mcp.toml` is unlayered and
      owner-only.
- [x] C5: integrated plugin, hook and interface editors plus the layers
      page. Done: `/config providers` opens `/providers policy`; the
      interface form links Theme, Status line, Terminal title, Keyboard,
      Presets and Interface mode; `/config layers` lists the files,
      trust state and warnings; scalar TUI preference writes now report a
      higher layer that shadows them, as the other domains already did.
- [x] C6: bilingual documentation and the full validation matrix. Done:
      the architecture, configuration, configuration-reference,
      terminal-interface, MCP and Provider pages describe the configuration
      center in both languages and keep matching heading ids; the full Rust
      and site matrices passed locally on Windows; all 34 documentation
      pages prerendered. Committed (`e4dbb98`).

# Follow-ups

## Plan: installable plugins on Kardia, compatible with Codex, with our own format (2026-11-10)

The plan is [docs/plan-plugin-system.md](docs/plan-plugin-system.md). Every
§9 decision was accepted as recommended. Sources: Codex `57a808a392`,
Kardia (`cordis-rs`) v0.2.0, pykardia. Each phase is validated, documented
in both languages, committed and pushed, and notes what Hanafubuki must
change at its next pin. Publishing Kardia to crates.io is confirmed with the
user first.

All phases (P1–P9 and K1) are done (2026-11-10), and K2 after them; the
open items each phase lists below still stand, but for those K2 closed.

- [x] P1: `crates/packages` package store: identity, the four manifest
      formats, marketplaces (local, git), versioned cache, data dirs, staged
      atomic install/uninstall, commands to Skills, content hashes,
      `install.json`; `plugins/` in the data dir; `[plugins]` and
      `[marketplaces]` (camelCase keys).
      Done: `hanafubuki-agent-packages` (domain layer: foundation, process),
      ported from Codex `57a808a392` with notices markers. `PluginId`;
      `find_manifest`/`load_manifest` over five formats, probed
      `.hanafubuki-plugin` (ours: Codex's fields plus typed, unused
      `runtime`/`contributes`/`permissions`/`configSchema`), Agent Plugins
      root `plugin.json`, `.codex-plugin`, `.claude-plugin`,
      `.cursor-plugin`; each field read on its own with issues (Agent
      Plugins `$schema`/`name` strict). `PackageStore::open(plugins_root,
      executor)`: `install_directory`, `install_from_marketplace` (local,
      `url`, `git-subdir` with ref/sha, sparse clone), staged copy, fallback
      manifest, byte check, command migration, atomic swap, prune;
      `installed()` walks the cache (survives a quarantined record), `read`,
      `uninstall(id, purge_data)`, `data_root`, `content_hash`;
      `install.json` (version, format, source, content hash) tolerant per
      field, quarantined when unreadable. Marketplaces: `load_marketplace`,
      `list_marketplaces(roots)`, `configured_marketplaces(records)`,
      `add_marketplace(request, configured)` / `upgrade_marketplace` /
      `remove_marketplace` returning the `[marketplaces]` record for the
      caller to write; `git` through the `ExecutorManager`. Config:
      `AgentSettings.plugins` (`PluginsSettings`, `_default`, per-package
      `enabled` and `mcpServers.<name>` overlay) and `.marketplaces`
      (`MarketplaceRecord` + `enabled`), merged per entry across layers;
      `package_enabled(id)`; writes `set_package_settings` /
      `set_marketplace` (`None` removes the entry); `config get/set` reach
      the keys. `AgentStoragePaths.plugins_root`. Docs: architecture
      "Installed plugins", config-reference "Plugins"/"Marketplaces",
      config-layers, data-directory, plugin-rules, contributing.
      Open: no manifest cache (nothing reloads repeatedly yet); npm sources
      (decision 9); marketplace discovery from `$HOME`/the workspace repo
      not ported (callers pass roots; P3 decided: only `[marketplaces]`); `install.json` has no
      granted permissions yet (P5); `content_hash` has no cancellation (P2
      hook trust may want it); for Agent Plugins P2's loader must ignore
      `paths.hooks` and discover Skills as direct children (Codex
      `loader.rs:931,984`); no automatic marketplace upgrades; `-c` cannot
      quote a key, so a dotted plugin name cannot be overridden there; the
      writes are dedicated calls, not `AgentSettingsPatch` fields (like
      `disabledPlugins`). Deviations from Codex: wrong-typed manifest
      fields dropped rather than failing; `file://` accepted by
      `marketplace add`; `.git` not copied into the cache; the content hash
      uses `/` separators; marketplace `lastRevision` in the record instead
      of a metadata file in the checkout.
      Hanafubuki: the data directory gains `plugins/`
      (`AgentStoragePaths.plugins_root`); `AgentSettings` gains `plugins`
      and `marketplaces` (a settings import reads them through
      `read_settings_document`; a settings page can ignore them); config
      now depends on the new published crate `hanafubuki-agent-packages`;
      no facade API change yet.
- [x] P2: content sources: `plugin:` Skill roots, plugin-owned MCP servers
      (`.mcp.json`, Agent Plugins `mcp.json`), `HookSource::Plugin` with
      package-content trust, the "available plugins" section, live reload.
      Done: the facade's `crates/agent/src/packages` (after Codex's
      `load_plugin`) loads installed ∩ enabled in key order into each
      domain's own plugin source; a bad package or component is an issue of
      that package only. Skills: `SkillScope::Plugin`, ids
      `plugin:<p@m>:<path>`, names (and an `openai.yaml` display name)
      `<manifest name>:<name>`; roots = manifest `skills` else `skills/`,
      plus the migrated-command root (not for Agent Plugins, whose Skills
      are direct children only); `SkillService::set_plugin_roots`;
      `skills_list`'s `authority` enum unchanged (goldens). MCP:
      `hanafubuki-agent-mcp::{plugin_mcp_servers, agent_plugin_mcp_servers}`
      (`src/plugin_config`, ported from `codex-mcp`): both `.mcp.json`
      shapes, Claude keys (`type`, `headers`, `oauth`), relative `cwd` and
      command joined to the root; Agent Plugins strict by its field lists
      (no `deny_unknown_fields`), `./` command and `cwd` resolved and
      contained through `foundation::paths`; the overlay
      (`enabled`/`enabledTools`/`disabledTools`); precedence user
      `mcp.toml` > first plugin in key order, losers are package issues;
      `AgentMcpApplication.plugin_servers` (never in `mcp.toml`), included
      in `start_servers`, `mcp_connect`, Skill dependencies and the snapshot
      (`McpApplicationSnapshot.plugin_servers`); `set_plugin_servers`
      connects added/changed and disconnects removed ones (not a name the
      user's server took over). Hooks: `HookSources.plugins`
      (`PluginHookSource { id, root, data, declarations }`),
      `HookSource::Plugin { id, root, data }`, keys
      `<p@m>:<relative path>:<event>:<g>:<h>` (`plugin.json#hooks[i]` inline)
      so state survives updates; trust = definition hash + package content
      hash (`trusted_content_hash` in `hooks-state.json`), `current_hash`
      `decl/content` in a listing (`HooksEngine::list`), re-hashed once per
      package per event before a run ("plugin contents changed; hook
      approval required"); env `PLUGIN_ROOT`/`PLUGIN_DATA`/`CLAUDE_PLUGIN_*`;
      Agent Plugins hooks ignored (issue); hooks now depends on packages
      (`content_hash`, LAYERS row). Model view: a new built-in row `plugins`
      (`crates/agent/src/plugins/packages`, Capability, after `hooks`)
      freezes the enabled packages at admission and renders
      `available_plugins` (Capabilities slot) only for a plugin with Skills
      in the turn's catalog or MCP servers; golden
      `installed_plugin_request.json` added, every other golden unchanged.
      Reload: `AgentApplicationService::reload_packages()` (serialized; MCP
      connects awaited), run in the background after `write_settings`,
      `set_plugin_enabled`, `mcp_write_server`, `mcp_delete_server`; the
      construct loads the same set at start (MCP servers connect with the
      start). Status: `packages() -> Vec<AgentPackageStatus { id, version,
      format, display_name, description, enabled, content:
      AgentPackageContent { skills, mcp_servers, hooks }, issues }>`. Tests:
      skills/hooks/mcp unit and suite tests; facade `tests::packages`
      (lifecycle with Skill `$mention`, hook trust/deny/content change/off;
      MCP on/off, precedence and takeover, overlay; Agent Plugins rules; a
      broken package and component). Docs: architecture, internals, skills,
      mcp, configuration (hooks), config-reference, plugin-rules, embedding.
      Deviations from Codex: legacy `.mcp.json` expands
      `${CLAUDE_PLUGIN_ROOT}`/`${CLAUDE_PLUGIN_DATA}`/`${PLUGIN_*}` and sets
      the four variables on stdio servers (Codex passes them literally);
      Claude's `headers` read as `http_headers`; `bearer_token_env_var` and
      `env_http_headers` refused; `oauth` means an optional login; the
      section lists the plugins (Codex's generic text lists none, its
      summaries feed `@plugin` mentions); a package that cannot be hashed
      lists its hooks untrusted rather than dropping them.
      Open: (P3 reuses the facade's `PackageStore` and reloads after each
      lifecycle call: done) a hand edit of `[plugins]` applies at the next start or reload (no
      watch); the Skill watcher watches the plugin roots present at start
      only (reads re-stamp every root); (`/mcp`, `mcp list` and `/warnings`
      show plugin servers and package issues: done in P3); `content_hash` still
      has no cancellation (a hook run hashes on the blocking pool); the
      plan's §3.2 overlay key is `mcpServers` (camelCase), not
      `mcp_servers`; no manifest cache (each reload rereads).
      Hanafubuki, at its next pin: `HookSource` gains `Plugin { id, root,
      data }` (no longer `Copy`; serialized `{"plugin": {...}}`, the others
      unchanged); `HookSources` gains `plugins`; `HooksEngine::list` (hashes
      packages) beside `discover`; `McpApplicationSnapshot.plugin_servers`
      (`McpPluginServerSnapshot { plugin, server, status }`); new facade
      calls `reload_packages()` and `packages()` and the re-exports
      `AgentPackageStatus`, `AgentPackageContent`, `McpPluginServer`,
      `McpPluginServerSnapshot`; `plugin_status()` lists a new row `plugins`
      (and `disabledPlugins` accepts it); `SkillScope::Plugin` and
      `SkillId::plugin()`; the facade now depends on
      `hanafubuki-agent-packages`, and hooks does too; a host that writes
      `[plugins]` itself calls `reload_packages()`.
- [x] P3: lifecycle API (facade), `plugins` CLI, TUI `/plugins` (Codex's
      port), built-in rows with retry, `--safe-mode`, `package_changed`.
      Done: facade (`service/packages/{mod,lifecycle,marketplaces}.rs`, after
      Codex's `manager.rs` and `marketplace_cmd.rs`) over the one store
      (`AgentPackages::store()`; nothing else opens one): `package(id)`,
      `install_package(AgentPackageSource::{Marketplace{marketplace,
      plugin}, Directory{path, marketplace}})` (a directory is filed under
      `local` unless named) → `AgentPackageInstall { package, root,
      migrated_commands, issues, hooks_to_trust: Vec<AgentHook>, permissions
      (empty for content) }`, writes `enabled = true`; `uninstall_package(id,
      purge_data)` removes the `[plugins]` entry, an absent package is an
      error; `set_package_enabled` writes `enabled` alone (merge keeps
      `mcpServers`), refused while the marketplace is off; `update_package`
      (the added marketplace as it is now, else the recorded directory);
      `retry_package` (reload); `marketplaces()` (only `[marketplaces]`:
      the P1 "discovery" item is decided: none), `add_marketplace`,
      `upgrade_marketplace(Option<name>)`, `remove_marketplace`; each
      writes the config and awaits `reload_packages`. `AgentPackageStatus`
      gains `source` and `hooks_to_trust` (packages are hashed for trust:
      `HooksEngine::list`). Safe mode: `AgentPackages` lists the packages
      and loads no content (no Skills, servers, hooks). `package_changed`:
      `AgentPluginEventSink::package_changed(&AgentPackageChange { id, kind:
      installed|removed|enabled|disabled|updated|failed, error })`, default
      empty, after the reload. CLI `plugins list [--available] [--json] |
      add | remove [--purge-data] | enable | disable | update | info
      [--json] | retry` and `plugins marketplace add [--ref] [--sparse] |
      list [--json] | upgrade [NAME] | remove` (after Codex's
      `plugin_cmd.rs`/`marketplace_cmd.rs`); global `--safe-mode`
      (`AgentStartOptions::safe_mode`); `mcp list` now starts the Agent and
      prints the plugins' servers `(plugin ID)`. TUI `/plugins`
      (`commands/plugins/{mod,browser,pages}.rs`, Codex's catalog): tabs
      Installed / Marketplaces / Built-in (←/→); package page (toggle, trust
      hooks, update, retry, uninstall with or without data; source, version,
      format, Skills, servers, hooks, issues); marketplace page → install;
      install-from-directory and add-marketplace forms; Ctrl+U/Ctrl+R on a
      marketplace; trust page after an install with hooks; built-in rows:
      space toggles (`set_plugin_enabled`), `r` retries (`retry_plugin`).
      `/mcp` and `/mcp verbose` show plugin servers read-only, labelled;
      `/warnings` gains the source "installed plugins". Tests: facade
      `tests::packages::manage` (local marketplace lifecycle with the event
      sequence and the overlay kept; directory install, marketplace off;
      git marketplace from a bare repo: add, already added, install,
      upgrade after a push, update, remove), CLI `tests/plugins.rs`, TUI
      `app::tests::plugins` (install from a marketplace, trust, disable,
      enable, uninstall; built-in off, retry refused, on; `/mcp` row) and
      `commands::plugins::tests`. A pty smoke of the debug binary
      (`--no-alt-screen`, a 50x160 pty, a minimal VT emulator in Python, a
      Provider at an unused port, a fresh `HANAFUBUKI_AGENT_HOME`): `/plugins`
      → Marketplaces → add a local marketplace by typing its path, install a
      Claude Code plugin with a Skill, `/skills` lists `greeter:hello`,
      disable (gone from `/skills`), enable, uninstall, `/exit` (code 0). Docs: cli-reference, terminal-interface,
      architecture, embedding, config-reference (both languages).
      Deviations: `package_changed` is not in `exec --json` (its events are a
      session's; nothing changes packages during an `exec` turn) but on the
      plugin event sink, documented in architecture/embedding; no `-m` for
      `--marketplace` (it is `--model`); the browser's tabs are pages of one
      view (no `SelectionTab` here); the TUI list is not searchable; Codex's
      `plugin list` shows marketplace plugins by default, ours the installed
      ones (`--available` for Codex's); `update` is ours (Codex has none);
      marketplace removal keeps installed packages (as Codex).
      Open: `package_changed` has no consumer in the standalone binary (the
      TUI reads after its own calls); installs from git marketplaces have no
      cancellation (git timeout only); `plugins update` of a package whose
      marketplace no longer offers it fails rather than falling back to its
      recorded source; the CLI cannot trust hooks (points to `/hooks` and
      `/plugins`); `set_package_settings` for the `mcpServers` overlay has no
      UI (config edits); the status line does not show failed packages;
      `packages()` hashes every package with hooks on each call (trust),
      which `/plugins`, `/warnings` and `plugins list` pay; with the `hooks`
      row off, `/plugins` lists hooks to trust but trusting them fails
      (`FeatureUnavailable`, as `/hooks`); `mcp list` now starts the Agent,
      so it takes the data directory's lock.
      Hanafubuki, at its next pin: new facade calls `package`,
      `install_package`, `uninstall_package`, `set_package_enabled`,
      `update_package`, `retry_package`, `marketplaces`, `add_marketplace`,
      `upgrade_marketplace`, `remove_marketplace`; new re-exports
      `AgentPackageSource`, `AgentPackageInstall`, `AgentPackageUninstall`,
      `AgentPackageChange`, `AgentPackageChangeKind`, `AgentMarketplace`,
      `AgentMarketplacePlugin`, `AgentMarketplaceAdded`,
      `AgentMarketplaceUpgrade`, `LOCAL_MARKETPLACE`,
      `MarketplaceAddRequest`, `MarketplaceSourceType`, `PackageSource`,
      `ManifestFormat`, `McpServerStatus`; `AgentPackageStatus` gains
      `source` and `hooks_to_trust` (a struct literal needs them);
      `AgentPluginEventSink` gains `package_changed` with an empty default;
      in safe mode installed packages contribute nothing.
- [x] P4: kernel deadlines at the five untimed points (context, admission,
      tool source, prompt `prepare`, tool `permission`).
  - Done: `AgentHardLimits::plugin_call_deadline` (10 s) bounds each. A
    context or turn admission contributor past it is cancelled and left out
    with an issue on its plugin; a tool source fails (its trust decides); a
    prompt preparer's values are left out with a warning; a `permission`
    past it is the tool error "the tool's permission check did not finish
    within N s and was stopped". `collect_world_state` and
    `collect_turn_extensions` take the admission's token and the deadline;
    each call observes a child of that token, cancelled too when the
    collection is dropped (`bounded_call`). Contract tests:
    `crates/kernel/tests/loop_contract/deadlines.rs`. Goldens unchanged.
    The bounds per extension point are a table in plugin-rules ("Bounds on
    plugin calls").
  - Open: the facade's admission token is fresh per admission (nothing
    cancels an admission still being prepared but dropping it); the
    deadline is not a setting. The facade's feature calls
    (`service/features.rs`) are outside the kernel and have no deadline.
  - Hanafubuki, at its next pin: `AgentHardLimits` has a new field
    `plugin_call_deadline` (a struct literal needs it, or
    `..FIRST_BATCH_HARD_LIMITS`); `collect_world_state` and
    `collect_turn_extensions` take `&CancellationToken` and a `Duration`
    (`AgentService::limits().plugin_call_deadline`); new
    `AgentService::limits`. A host's context or admission contributor that
    takes over 10 s is now left out of the turn.
- [x] K1: Kardia (`cordis-rs`): public async guest transport, `Spawner`,
      async calls with timeouts and request ids, exit monitoring, meta from
      outside, command and args, stderr tail, frame limit, WASM engine
      limits; `kardia-guest/2` in kardia-guest and pykardia; the guest SDKs
      relicensed permissively; release (0.2.x if additive, after the user
      confirms publishing).
  - Done: Kardia 0.2.1 and 0.2.2 released on crates.io (0.2.2 made
    kardia-loader's `preserve_order` a feature, P5); pykardia's
    kardia-guest/2 and its relicensed guest SDK pushed. Open: pykardia's
    PyPI release (PyPI has 0.1.0, which predates kardia-guest/2) is the
    user's to publish; until then CI does not run the Python example.
- [x] P5: guest host `crates/plugin-packages`: the `plugins` row with a
      kardia-loader subtree, `Resolver`, `ExecutorManager` spawner, the
      sandbox's plugin profile, exit watcher, guest state and restart loop.
  - Kardia 0.2.2: kardia-loader 0.2.1 turned on `serde_json/preserve_order`,
    which features unify across the Agent's build (every `serde_json::Map`
    insertion-ordered: pinned fragments, a compression hash and "first in
    key order" precedence changed; five tests failed). 0.2.2 makes it the
    `preserve-order` feature of kardia-loader (and a pass-through of
    kardia-hmr), on by default; the guest host takes both with
    `default-features = false`, so `cargo tree -e features -i serde_json`
    shows no `preserve_order`. A member that adds kardia-loader or
    kardia-hmr must do the same.
  - Done: `hanafubuki-agent-plugin-packages` (`GuestHost`), mounted by the
    facade's `plugins` row as its child fiber `plugin-guests` (the row keeps
    P2's "available plugins" section; moving the section would need the
    facade's package data in the crate). Inside: kardia-loader's plugin over
    `GuestResolver` and `SettingsTree` (`TreeBackend`), and
    `plugin-guest-supervisor` (injects `loader`, `plugin-faults`). The
    facade hands the code packages (`GuestPackage`, from
    `LoadedPackage.guest`) to `GuestHost::apply` at every reload; the
    supervisor turns them into the tree with a group update (one entry per
    package, id = package key, name `<id>#<digest of version, recorded
    hash, runtime, asked/granted permissions, dirs>`, `isolate: {plugin:
    true}`, `disabled` unless on and allowed), which commits nothing, so the
    settings port (`GuestSettingsPort::set_enabled`, the facade's
    `PackageSettingsWriter`: tokio task, blocking pool, writes only a value
    that differs) sees only the loader's own write-back. The resolver caches
    one `ProcessGuest::plugin(GuestMeta::new(manifest name).provide("plugin"))`
    per entry name, renamed `guest:<id>`, timeout/handshake =
    `plugin_call_deadline` (10 s), frame limit 4 MiB. Spawner: trust check
    (content hash = recorded), data dir, command (`./` in the package, or an
    interpreter: absolute or on `PATH`), plugin profile, reserve owner,
    `start_reserved_pipe_executor` (new in crates/process: pipes handed out,
    no reader/writer, the session keeps the real `ExitStatus`;
    `wait_reserved_executor`), `GuestChild` adapter (kill = receipt
    terminate, wait = session end, then dispose = owner released; a drop
    before confirmation disposes on a contained thread). Env cleared but
    granted `env`, `PLUGIN_ROOT`, `PLUGIN_DATA` (+ `SystemRoot` on Windows);
    cwd = data dir. Sandbox: `SandboxPolicy::Plugin { readable_roots,
    writable_roots, network_access }`, helper `--restrict-reads
    --readable-root`, Landlock read rules on Codex's platform roots + roots,
    Seatbelt subpath reads + Codex's restricted-read platform defaults
    (`seatbelt_read_only_platform_defaults.sbpl`), Windows token unchanged
    (reads unrestricted); `SandboxError::PluginUnavailable`; guests use
    `Sandbox::detect()` whatever `[windows] sandbox` says and never start
    unconfined. State: `GuestState::{Stopped, Blocked, Starting, Running,
    Restarting, Failed}` from `StatusChanged` (Loading/Active/Failed/
    Disposed) on the entry's current fiber, handle from `GuestStarted`, exit
    report from `GuestExited` or the handle; a failure after `Active` is a
    fault: `RestartWindow` (RestartPolicy 2/10/30 s, 3 per 10 min) →
    `restarting`, the worker restarts with `update_with(config, true)` after
    the delay (epoch check), else `failed` until `retry`; a failed start is
    not restarted; nothing reaches `[plugins]`. `GuestHost::call(id, method,
    args, timeout)` calls the guest's `plugin` service (`live_as` in its
    private realm). Logging: host's `LogExporter` sends loggers named
    `guest:*` (`GUEST_LOGGER_PREFIX`) to `trace`, bounded; the guest host
    logs its decisions at info/warn without guest words.
    `bounded_trace_content`/`MAX_TRACE_CONTENT_BYTES` moved to
    `foundation::trace_content` (model-api re-exports them). Packages:
    `install.json` `grantedPermissions` (`InstallRecord.granted_permissions`,
    kept across reinstalls), `PackageStore::grant_permissions`,
    `PackagePermissions`/`WorkspaceAccess` serde (+ `Ord` on the access).
    Facade: `grant_package_permissions(id, permissions)` (records, reloads),
    `retry_package(id)`; `AgentPackageStatus.guest: Option<AgentPackageGuest
    { permissions, granted_permissions, status: GuestStatus }>`; re-exports
    `GuestState`, `GuestStatus`, `GuestExitReport`, `GuestExitCause`,
    `PackagePermissions`, `WorkspaceAccess`. xtask: `kardia-loader` and
    `kardia-hmr` restricted to the guest host; LAYERS row (contracts,
    foundation, host, packages, process, sandbox). kardia-core/macros 0.2.2.
    Tests: `crates/plugin-packages/tests/guests` (harness = false; the binary
    is the test guest via kardia-guest `serve` and the sandbox helper):
    start/disable/enable/uninstall, permissions blocked until granted,
    environment and cwd, owner group + `process-groups.json` + reap at next
    start + stop with the guest host, `kill -9` → restart, call deadline →
    killed and restarted, 4th fault → failed until retry, changed package
    refused, hanging/failing `apply` stays failed, guest stderr/log only at
    trace, the sandbox (Linux only): package read, outside and workspace
    unreadable with `none`, data writable, package read-only, workspace
    read after a new grant. Plus process (handed-out pipes), sandbox (helper
    flags, Seatbelt policy, Landlock end to end), packages (grant record),
    host (exporter), xtask (dependency rule), facade (`tests::packages::code`:
    blocked → grant → fails closed without a sandbox; retry). Stress: the
    `guests` binary 20 rounds × 4 parallel copies: 2/80 failed first (the
    test read `process-groups.json` the instant before the session forgot
    its group, which `finalize` does just after it announces the end; the
    check now waits), then 0/80.
  - Deviations from the plan: Kardia's `Failed` is used (the bridge fails a
    guest's fiber on exit; restart via `update_with(config, true)` since
    `restart()` keeps a failed fiber failed) instead of a state kept only
    by the guest host; an upgrade is a new entry name (rebuild), not
    `registry().replace`; the `plugins` row mounts the guest host rather
    than being it; the loader's own tree is the subtree (not
    `create_tree`); every guest must declare and provide the service
    `plugin` (its meta is checked); the trust check is at every spawn; a
    guest's stderr is logged at trace, not debug (the plan's §4.3 said
    debug; AGENTS.md's content rule wins).
  - Open: (P6 contributions, proxies calling `GuestHost::call`: done in
    P6) a guest can still provide other service keys in the root realm
    (only `plugin` is isolated; P6 should isolate or refuse the rest: done
    in P6, refused by failing the guest); `view`/`invoke` host calls are
    refused (empty `inject`), the allowlist is P6 (done in P6: `workspace`,
    `settings`); re-approving a changed package (today: reinstall); the
    settings port writes `enabled` only (P6's `configSchema` settings are
    the entry's `config`, `{}` today: done in P6, written by the facade's
    `set_package_settings`) and
    does not reload after its write; the executor's final environment
    overlay (proxy variables) reaches guests; Windows restricted token does
    not restrict reads and its guest path is untested on Windows CI beyond
    `cargo check` (the `guests` suite will first run there and on macOS
    in CI); the sandbox check is Linux-only, macOS's Seatbelt profile is
    untested on macOS; (kardia-hmr's `guest-stdin`/`guest-stdout`/
    `guest-stderr` threads were plain `std::thread`s, not contained: done
    in K2, Kardia 0.2.3 catches their panics and the guest host registers
    its containment check); `cargo about`
    was not run here (kardia-fs/-hmr/-loader are GPL-3.0-only, accepted);
    K1 above is not ticked here (another phase's); a guest's
    `GuestStarted` handle shows a dead process as `running` (no pid) until
    its fiber fails.
  - With P3: `retry_package` (P3's: reload, return the status) also
    retries a failed guest; `AgentPackageInstall.permissions` is now
    `Option<PackagePermissions>`, what a code package asks for and was not
    granted (`None` for content); `/plugins`' package page offers **Grant
    its permissions** (`PluginsCall::GrantPermissions`) and a **Code** row
    with the guest's state. The CLI has no grant command yet (P7's `plugins
    dev`/consent prompts, or a `plugins grant`: done in P7, both).
  - Hanafubuki, at its next pin: new published crate
    `hanafubuki-agent-plugin-packages` (the facade depends on it);
    `AgentPackageStatus` gains `guest`; new facade call
    `grant_package_permissions`; `retry_package` also retries a guest;
    `AgentPackageInstall.permissions` is `Option<PackagePermissions>`; re-exports above; the
    program must call `helper_dispatch()` for guests to run (else they fail
    closed); twins: `hanafubuki-foundation` takes `trace_content`, and
    `hanafubuki-module-contracts`' `logging` the `GUEST_LOGGER_PREFIX`
    branch (additive); `hanafubuki-process` takes
    `start_reserved_pipe_executor`, `ExecutorPipes`,
    `wait_reserved_executor` and the session's `exit_status` (additive);
    kardia-core/macros 0.2.2 (additive); a Hanafubuki crate that takes
    kardia-loader or kardia-hmr takes it without `preserve-order`.
- [x] P6: contribution proxies (tools as sourced tools, context sections,
      prompt fragments, lifecycle and continuation hooks, observers), the
      host-function allowlist, settings from `configSchema`.
  - Done: `crates/plugin-packages/src/contribute/` builds proxies from the
    manifest's `contributes` (`GuestPackage.contributes`) and registers them
    in the supervisor's `StatusChanged(Active)` listener, before the guest is
    marked `running`, through the guest fiber's own context (`plugin-guests`
    now injects the seven registries and `PluginFaults`), so each is that
    fiber's effect (owner `guest:<id>`) and retires with every failure, stop
    and restart. Tools: one `Optional` source `plugin:<id>`, tools
    `<plugin>__<tool>` (namespace = plugin name), provider
    `plugin.<plugin>.<marketplace>`, generation = hash of entry name and
    declaration, `Exclusive`/`Mutating`/`PollutesContribution`, approval
    `ApprovalActionKind::PluginToolCall` (new, high risk, always
    `ReviewRequired`, re-checked in `invoke`), guest call `tool` within the
    manifest tool's new `timeout` (seconds, ≤ 3600; `ContributedTool.timeout`)
    or the call deadline, kernel bound + 2 s; a string answer is text, else
    structured; a failure is the call's error with the guest's words (model
    only). Context: a contributor + renderer per kind (≤ 7 per guest; valid,
    not a known kind; WorldState, developer, versioned, a no-authority
    preamble), `context` within 80% of the deadline, cut to 4 KiB. Prompt:
    renderer of kind `plugin_<name>` in the declared slot; one contributor of
    the guest host (`GuestPrompts`, `plugin-guests`) prepares every running
    guest's `prompt` at once, each within 80% of the deadline. Hooks: a tool
    lifecycle hook (`tool-use`) and a continuation hook (`continuation`,
    `Ignore`), priority 200, `applies` while running. Observers: kinds named
    after the notifications (`turn_finished`, …), a 64-event queue, a scoped
    worker of the guest fiber calls `observe`. Errors told to the kernel
    (which logs them at warn) are the Agent's words; the guest's at `trace`.
    Calls observe cancellation (50 ms check). Issues of left-out
    declarations + the kernel's issues for `guest:<id>` →
    `GuestStatus.issues`. Host functions: a host-services loader entry
    `<id>#host` beside each guest's provides `workspace` (`{access, roots}`,
    roots only when readable, read at each view) and `settings`, isolated
    under the shared label `guest:<id>`; the guest's entry isolates them the
    same and `inject`s them (kardia-loader's outer fiber), so `view` reaches
    exactly these; checked access finds nothing else viewable or invokable.
    Provisions: the supervisor listens to `ServiceChanged`; a binding in the
    shared realm whose provider is a guest fiber fails that fiber
    (`Fiber::fail`, provisions withdrawn): during start like a failed
    start, while running a fault (restart). Settings: `PluginSettings.settings`
    (`[plugins."p@m"].settings`, a table; tolerant, a non-table is a repair;
    not serialized when unset, so entry writes keep it),
    `AgentConfig::set_package_code_settings` (replace: removes the old table
    first, since writes merge), both loader entries' `config` (so a change
    restarts the guest), facade `set_package_settings(id, Value)` (object or
    null; checked against `configSchema` with the kernel's schema check;
    refused for a package without code), `AgentPackageGuest.{settings,
    config_schema}`; `/plugins` code page **Settings** row → a JSON form
    (`Prompt::Settings`, `PluginsCall::SetSettings`). Fixture observer kind
    `turn_completed` → `turn_finished`. Tests: guests suite +4 checks
    (contributions through every registry incl. approval, left-out
    declarations and issues; a hanging/crashing guest's contributions retire
    and return, its context/prompt left out in time, a tool exit is the
    call's error; views of workspace/settings vs refused keys and invoke;
    settings reach apply and restart it; a provided service fails the
    guest), facade `tests/plugin_guests.rs` (harness = false, the binary is
    the guest and the sandbox helper; Linux only, skips without a sandbox):
    install, grant, settings refused by `configSchema` then set, a turn whose
    request has the section and fragment, the model calls the tool (user
    approval `plugin_tool_call`), the next request has the output and the
    hook's context, `turn_finished` observed, a crashing tool is the call's
    error and the guest restarts; packages (tool timeout), config (settings
    decode, replace, kept by entry writes), TUI (settings row). Docs:
    plugin-rules "Code plugins" (contributions, host calls, the guest
    protocol), architecture, internals, embedding, config-reference,
    terminal-interface, json-events (`plugin_tool_call`), both languages.
  - Deviations: registration in the Active listener, not inside `apply`
    (kardia-hmr's bridge owns `apply`); one prompt contributor for all
    guests (`TurnExtensions` is keyed by type); the allowlist is by
    construction (realms + `inject`), not a host-call filter, and a refused
    `view` reads as kardia's access error; another provision is refused by
    failing the guest after kardia bound it (a brief window in the shared
    realm), not before; context kinds are the declared names (not
    namespaced), a colliding one is refused; `renderer` is read but unused;
    approval for every guest tool call (no read-only hint in the manifest);
    the settings UI is a JSON text field, not a form from `configSchema`;
    no Python guest test (pykardia is not in CI; P7's example).
  - Open: (kardia-hmr's `GuestHost::provide` could honour the declared
    meta upstream, closing the window: done in K2, Kardia 0.2.3 refuses an
    undeclared key and the watcher is gone); the calls of one guest are serialized
    (a long tool call makes its context/prompt wait and fail); the kernel's
    64 KiB world-state total can still be exceeded by large sections of
    several plugins together (pre-existing, host kinds too); a hook's deny
    reason and a tool's failure carry the guest's words to the model (as
    MCP's do); (no CLI for settings: done in P7, `plugins settings`);
    the sandboxed facade path is checked on Linux only; the
    P5 open items (Windows reads, Seatbelt untested on macOS) stand (hmr's
    plain threads: done in K2); the kernel's issues kept for `guest:<id>` are not
    cleared when a package is uninstalled; two packages with one manifest
    name (different marketplaces) derive one prompt kind, so the second's
    prompt is refused with an issue. (Resolved: the first facade run under
    `permissionMode = "full_access"` waited for an approval because the
    test's `start_turn(id, text.into())` did not read `permissionMode`: an
    input from text was `Manual`, replaced only by the session's own mode,
    while `submit_turn_with_sink`, the TUI's and `exec`'s path, read it.
    Fixed after P9 (a breaking facade change the user approved): the
    facade has its own `AgentTurnInput { text, images, skills,
    permission_mode: Option<AgentPermissionMode> }` (the kernel's stays
    strict, with a concrete mode), `From<String>`/`From<&str>` give `None`,
    and every facade turn resolves its mode in one place
    (`AgentSessionDriver::turn_permission_mode`: the requested mode, else
    the session's, else the configured one); `start_turn` runs through the
    driver's `run_turn`, as the plugins' child turns do. A plugin tool is
    approved exactly as an MCP tool that asks. `plugin_guests.rs` checks
    each mode: `Manual` given in the input asks the user, the session's
    `AutoReview` the reviewer, configured `FullAccess` no one, all through
    `start_turn`; `sessions/permissions.rs` pins the order.)
  - Hanafubuki, at its next pin: `AgentTurnInput` is now the facade's own
    type, `{ text, images, skills, permission_mode }`, with
    `permission_mode: Option<AgentPermissionMode>` (a literal gives
    `Some(mode)` or `None`; `turn`, `world_state`, `origin` and
    `provisional_title` are gone, the facade sets them). `start_turn` with
    text now runs in the session's own mode, else the configured
    `permissionMode`, not `Manual`: a host relying on `Manual` must say so
    (`permission_mode: Some(AgentPermissionMode::Manual)`). A mode given in
    the input now wins over the session's own, which used to replace it.
  - Hanafubuki, at its next pin: `ApprovalActionKind::PluginToolCall`
    (`plugin_tool_call`; an exhaustive match needs an arm);
    `PluginSettings.settings` (a struct literal needs it) and
    `AgentConfig::set_package_code_settings`; `ContributedTool.timeout`;
    `AgentPackageGuest.{settings, config_schema}` and `GuestStatus.issues`;
    new facade call `set_package_settings`; `GuestPackage.{contributes,
    settings}`; the guest host now depends on approval and tool-contracts
    and its plugin injects the registries (a host mounting it must provide
    them, as the kernel does).
- [x] P7: developer story: `plugins new|dev|validate`, Rust and Python
      examples, the "Writing plugins" page.
  - Done: CLI (`crates/cli/src/plugins/` now a directory: `mod.rs` the
    P3 commands, `scaffold.rs`, `develop.rs`, `code.rs`). `plugins new NAME
    [--lang rust|python] [--dir DIR]` writes from `crates/cli/templates/
    plugins/` (`.tmpl`, so neither cargo nor xtask reads them) a project
    whose installable plugin is its `package/` (the build tree stays out of
    what an install copies and hashes): Rust (kardia-guest 0.2.2; a tool
    `greet`, a section `<name>_status` that views `workspace`, an observer
    `turn_finished`, permissions, `configSchema`) or Python (pykardia's
    guest SDK exported as a zipapp run by `python3` with `interpreter:
    true`; a tool `count_words` and a prompt fragment). `plugins validate
    DIR [--run] [--json]`: the facade's new free function
    `validate_package(dir) -> AgentPackageValidation { name, version,
    format, manifest, code, issues }` (manifest issues, declared paths that
    do not exist, and plugin-packages' new `check_package`: runtime kind,
    protocol, the program in the package (executable on Unix) or the
    interpreter on the `PATH`, `./` arguments, contributions as
    registration would leave them out, `configSchema` an object and within
    the tool-schema bounds); `--run` composes the Agent over a throwaway
    home (config and data in a temp dir: `Standalone::open_in`,
    `with_agent_in`), installs, grants, waits until the guest settles and
    prints its state, issues, exit and stderr; exit 1 on any issue or a
    guest not running. The registration rules are shared: `contribute/
    check.rs` uses `context::guest_kind`/`MAXIMUM_GUEST_KINDS`,
    `prompt::{slot, prompt_kind}`, `observers::EVENTS`; spawn's
    `resolve`/`in_package`/`interpreter` are used by `check_package`.
    `plugins dev DIR [--grant]`: install as `<name>@local` (on), ask on the
    terminal to grant (non-terminal without `--grant`: usage error), print
    the guest; `notify` 8 watches the directory (not `.git`), 400 ms
    debounce, then `install_package` again (a new content hash → a new
    entry name → the guest starts anew; grant and settings kept; new
    permissions ask again), until Ctrl+C (`tokio::signal::ctrl_c` on the
    Agent's runtime). `plugins grant ID` (running it is the consent;
    prints what it granted and the guest), `plugins settings ID [--get |
    --set JSON]` (`null` removes; prints `{settings, configSchema}`).
    `plugins add` of code says to grant; `plugins info` shows the code's
    state, error and issues. Examples: `examples/plugins/hello-rust` (with
    its `Cargo.lock`) and `hello-python` are `plugins new`'s output
    (a test compares them); the workspace `exclude`s `examples`. Tests:
    CLI `tests/plugin_development.rs` (scaffold contents for both
    languages, bad name, non-empty dir; examples equal `new`'s output;
    validate good and bad: missing program, protocol, tool name, timeout,
    duplicate, kinds, slot, observer, unknown contribution, schema type,
    `--json`, no manifest; add/info blocked/grant/settings refused by the
    schema, bad JSON, set/get/null, a content plugin has no code),
    `tests/plugin_examples.rs` (Unix: builds hello-rust with the
    workspace's cargo `--locked` into `target/plugin-examples`, validates,
    and on Linux `--run` → running unless the machine has no sandbox;
    hello-python exported and run the same when
    `HANAFUBUKI_AGENT_TEST_PYTHON` names an interpreter with pykardia; this
    is P6's deferred Python guest check: it starts only when its metadata is
    exactly `{name, provide: ["plugin"]}`), facade
    `tests/plugin_guests.rs` (dev's reload: the changed directory installed
    again with `update_package` restarts the guest with a new section, grant
    and settings kept, the next request carries it). Run here: the Python
    test with `/root/micromamba/envs/py311/bin/python` (pykardia `main`),
    guest started under Landlock with `/usr/bin/python3`. Docs: new page
    "Writing plugins" (`writing-plugins(.en).mdx`, after `skills` in both
    metas): start, package layout, manifest reference, the guest (Rust,
    Python, licences, the wire for other languages), validate, dev,
    settings/permissions/sandbox, failures (§5.1), publishing via a
    marketplace; cli-reference (five commands, `info`), plugin-rules
    (links from `#writing-a-plugin` and `#guest-protocol`; the protocol
    table stays there), architecture (Code plugins), embedding
    (`validate_package`), config-reference, building-testing; NOTICE and
    the notices policy follow `plugins.rs` → `plugins/mod.rs`.
  - Deviations: `validate --run` installs into a throwaway home through the
    real Agent rather than a bare handshake (the process stays the
    executor's, and the sandbox, metadata check, `apply` and registration
    all run); `plugins dev` holds the data directory's lock, so the
    terminal interface cannot run on the same home meanwhile (documented:
    stop it, or a scratch `HANAFUBUKI_AGENT_HOME`); it reinstalls with
    `install_package`, not kardia-hmr's `reconcile`; the example READMEs
    are Markdown files outside the site (the scaffold's README, which the
    task asked for); the tool-name rule in `contribute/check.rs` restates
    `ToolName::parse` + `ToolId::parse` instead of calling
    `contribute/tools.rs` (left untouched while the approval investigation
    works there; fold it in later); Python CI skipped (PyPI's pykardia
    0.1.0, 2026-11-06, predates kardia-guest/2 and the guest SDK's
    relicence, 2026-11-10); no `/plugins` form from `configSchema` (the JSON
    field stays).
  - Open: the example test is Unix only (a manifest names `./<name>`, not
    `.exe`; Windows would need its own command or an `.exe` probe in
    `in_package`); `dev` stops at a first install that fails (later
    failures are printed and it keeps watching); `dev` does not offer to
    remove the package on exit; `validate --run` runs no turn (a tool call
    end to end is the facade test's); the configSchema check is a shape
    check (object, bounds), not a JSON Schema meta-validation; publishing
    a compiled guest means a binary per platform in the package (no
    per-platform `runtime.command`); a published pykardia release with
    kardia-guest/2 would let CI run the Python example; no per-tool
    read-only hint in the manifest (every guest tool call still asks); a
    guest of an enabled package reads `stopped` for a moment while a
    reinstall replaces its fiber (the supervisor's `Disposed` handling, by
    design), which the CLI's `settled()` waits through but `/plugins` and
    `packages()` callers can see.
  - Hanafubuki, at its next pin: new facade function `validate_package`
    and type `AgentPackageValidation` (additive);
    `hanafubuki-agent-plugin-packages::check_package` (additive). Nothing
    else changes for a host.
- [x] P8: WASM guests with fuel/epoch and memory limits.
  - Done: kardia-hmr 0.2.2's `wasm` feature in the guest host (`process`,
    `wasm`, still `default-features = false`: `cargo tree -e features -i
    serde_json` shows no `preserve_order`); wasmtime 34 + cranelift, ~20 s
    more of a clean build here (96 s CPU). Manifest (`crates/packages`):
    `PackageRuntime` is now an enum, `Process(ProcessRuntime { command,
    interpreter, protocol })` | `Wasm(WasmRuntime { module, limits:
    WasmRuntimeLimits { fuel, timeout, memory_bytes } })`, `kind() ->
    RuntimeKind` (now `Serialize`); `runtime: { type: "wasm", module:
    "./x.wasm", limits?: { fuel?, timeoutMs?, memoryBytes? } }` read per
    field (fuel ≥ 1; timeoutMs 1..=3600000; memoryBytes 64 KiB..=4 GiB,
    default 256 MiB, `DEFAULT_WASM_MEMORY_BYTES`; out of bounds = issue +
    default); a wasm runtime without `module` is ignored with an issue;
    a wasm package's `permissions` are ignored with an issue and normalized
    to nothing (no network, workspace `none`, no env). Guest host
    (`src/wasm.rs`): the resolver keeps a `OnceCell` per entry name that
    its first resolution fills on the blocking pool: content check, module
    `in_package`, `WasmSource::with_limits` (time always set: `timeoutMs`,
    else max(call deadline, longest tool `timeout`); memory; fuel if set),
    a `ModuleGraph` of that one module (`deps` refused), `import` compiles
    and reads the meta from a first instance; name/inject/provide checked
    against the manifest (as kardia's own check for process guests), then
    renamed `guest:<id>`; a load failure resolves to a plugin whose start
    fails with the reason (a failed start, not stuck `starting`). Faults:
    `GuestHost::call` matches a WASM call's error text (`wasm trap:`,
    `forcing trap when growing`, `the guest was stopped`, `error while
    executing at wasm backtrace`) and `fiber.fail`s the guest, so the P5
    restart loop/window applies and its contributions retire and return; a
    guest's own error (status ≠ 0) is only the call's. No grant and no
    sandbox: `GuestPackage::needs_grant()` (process only), `refusal()` only
    for process guests, the `workspace` view says `none` for a module
    whatever the package carries. Facade: `AgentPackageGuest.runtime:
    RuntimeKind` and `ungranted()` (none for WASM), used by the install's
    `permissions` and `/plugins`' grant row; re-export `RuntimeKind`. CLI:
    `plugins grant` of a WASM plugin fails ("needs no grant"); `plugins
    info`, `validate --run`, `dev` and `/plugins`' Code row add
    `(WebAssembly)` to the state; `plugins new NAME --target wasm` (Rust
    only; `--lang python --target wasm` is a usage error) writes a `cdylib`
    crate with `kardia_guest::export!(Plugin)` (templates `rust-wasm/`:
    a tool, a section that does not view the workspace, an observer;
    `memoryBytes` 64 MiB), and `examples/plugins/hello-wasm` (with its
    `Cargo.lock`) is its output; built here to a 110 KB module that
    `validate --run` starts. `plugins validate`: the module is in the
    package and starts with `\0asm`; a tool `timeout` longer than
    `limits.timeoutMs` is an issue. CI: `targets: wasm32-unknown-unknown`
    on the toolchain step (all three OSes) for the example test.
    `about.toml` accepts `Apache-2.0 WITH LLVM-exception` (wasmtime,
    cranelift-*, regalloc2, target-lexicon, pulley-*; checked: generation
    fails without it); `cargo about generate --locked --fail` passed with
    the release workflow's pinned cargo-about 0.9.2. Tests: guests suite
    +4 checks with text-format modules assembled by `wat` (dev-dependency,
    already in the graph via wasmtime): a module runs with the sandbox
    unavailable, no process and no grant, its tool runs as its `call`; a
    meta mismatch is a failed start; its `workspace` view is `none` even
    when the package asks for `write`; a guest's own error does not
    restart it; an endless call stopped by time (300 ms deadline,
    `interrupt`) and by fuel (`fuel`), each restarts once while a third
    guest keeps running; memory growth past 1 MiB traps (`growing
    memory`) and restarts, the default lets it grow; a trapping tool is the
    call's error, the guest is `restarting` with `unreachable`, its tool
    source and context retire and return, and its trapping context is left
    out. Packages (wasm runtime, limits out of bounds, no module), facade
    (`tests::packages::code`: a WASM package installs with nothing to
    grant and runs where the process guest fails closed), CLI (scaffold,
    python+wasm refused, the example equals `new`'s output, validate: not a
    module, missing module, timeout over the limit, memory bound,
    permissions issue; add prints no grant hint, grant refused),
    `plugin_examples` (hello-wasm built for wasm32 with `--locked`, then
    `validate --run` → `running (WebAssembly)`; skipped without the
    target). Docs (both languages): writing-plugins (WebAssembly section,
    manifest fields, scaffold, validate, permissions, failures, publish),
    architecture (code plugins, a stale known limit replaced), plugin-rules
    (crate row, threads, code plugins, view, protocol, logging), internals
    (WebAssembly guests), embedding (`runtime`, `ungranted`, wasmtime's
    signal handlers), cli-reference (`--target`, grant), building-testing.
  - Deviations: the meta is read from the guest at load (one
    instantiation, no host calls), not given from outside: kardia-hmr's
    WASM `GuestCode` is private, so there is no `guest_plugin(code, meta)`
    for it; the guest host checks it against the manifest instead. A WASM
    guest's failure is found from the call's error text (kardia-hmr hands
    traps over as `Error::msg`), not a typed signal or an exit report.
    (Both replaced in K2: `WasmGuest::plugin` with the manifest's meta,
    `GuestFailure::of`.) WASM
    permissions are ignored with an issue rather than refused, and a WASM
    guest needs no grant at all (the plan's consent was for what a process
    may reach). The default time limit is the longest call the guest may
    take, not the call deadline, since `WasmLimits::timeout` caps every
    call (a caller's deadline can only shorten it).
  - Open: a WASM call whose caller dropped it (a cancelled turn) is not
    followed: its stopped instance is failed by its next call, which fails;
    (each running module version kept kardia-hmr's `wasm-epoch` thread, a
    plain `std::thread`: done in K2, one engine and thread per set of
    limits, and the thread contained); wasmtime installs its signal
    handlers process-wide at the first WASM guest (documented for hosts in
    embedding; the Mach port on macOS is off since K2); (an upstream
    kardia-hmr could expose the WASM `GuestCode` (meta from outside, no
    load-time instance) and a typed "instance failed" error: done in K2,
    Kardia 0.2.3); kardia-hmr could honour `CallControl` cancellation in
    WASM; no module size cap (compiling a
    huge module costs the blocking pool); WASI modules (pykardia's WASM
    export) do not load (no WASI imports); the release binary's size
    growth from wasmtime was not measured here.
  - Open (CI, first run of P8): on macOS (arm64) the `guests` suite hung
    for 35 minutes in its first WASM check, after every process check
    passed, while the facade's WASM package test and the CLI's
    `hello-wasm` example ran on the same runner. Not reproduced on Linux
    (also with 1 to 3 CPUs). The suite now runs each check under a 120 s
    watchdog that prints its steps (fixture start, `apply`, waits, calls),
    its latest log records (the guest host logs each module's load at
    `debug`) and, on macOS, `sample`'s stacks of every thread, then exits;
    CI jobs time out after 60 minutes. Candidates the next run should
    settle: four engines compiled at once (the others one), the `liar`
    module dropped while others load, wasmtime's Mach exception port in a
    process that ran guest processes before; if it is the Mach port,
    `Config::macos_use_mach_ports(false)` needs kardia-hmr to take a
    wasmtime `Config` (an upstream change). K2 turned the Mach port off
    (Kardia 0.2.3's `set_macos_mach_ports(false)`), shares one engine
    per set of limits (the suite's four default-limit modules now compile
    on one engine) and no longer instantiates a module at load (the
    `liar` module's mismatch is found at its own start), so all three
    candidates moved: a recurrence points at something new. The watchdog
    stays until a macOS run passes repeatedly (unverified here: Linux
    only).
  - Hanafubuki, at its next pin: `PackageRuntime` is an enum
    (`Process(ProcessRuntime)` | `Wasm(WasmRuntime)`), new
    `ProcessRuntime`, `WasmRuntime`, `WasmRuntimeLimits`,
    `DEFAULT_WASM_MEMORY_BYTES`, `PackageRuntime::kind()`, `RuntimeKind`
    is `Serialize`; `AgentPackageGuest.runtime` (a struct literal needs
    it) and `AgentPackageGuest::ungranted()`; re-export `RuntimeKind`;
    a WASM package's install returns
    `permissions: None`. The guest host now pulls wasmtime/cranelift
    (`Apache-2.0 WITH LLVM-exception`, which Hanafubuki's licence policy
    must accept), and the first WASM guest installs wasmtime's signal
    handlers in Hanafubuki's process: its own handlers for SIGSEGV, SIGBUS,
    SIGILL or SIGFPE, if any, must be installed first or chain.
- [x] P9: end-to-end crash tests (panic, hang, kill -9, abort, garbage,
      oversize frame, restart loop) and a pty smoke test.
  - Done: `crates/agent/tests/plugin_crashes/` (harness = false; the binary
    is the guest, a stdio MCP server and the sandbox helper; each check
    under a 150 s watchdog with its steps), one check per row of the plan's
    §5.1, each over the whole Agent and the scripted model and each ending
    in a turn that completes with the other plugins' contributions: a
    content package whose installed manifest is broken, one whose
    `.mcp.json` and hooks file do not read (its Skill loads; the hooks
    file is a hooks diagnostic) and a code package whose program is
    missing (failed start), beside healthy ones; a plugin's stdio MCP
    server that exits mid-call (the call is a tool error, the turn ends,
    the supervision reconnects it, the next turn's call works); trusted
    hooks that exit 1, name no program, time out and print bad JSON (no
    effect; the tool runs); a guest panic (the call's error with the
    guest's words; same process, no restart); a tool past its 1 s timeout
    and a context that never answers (killed, restarted; the section left
    out, the turn goes on); abort, `SIGSEGV` (the handler reset to the
    default first: Rust's runtime swallows a raised one) and
    out-of-memory (`RLIMIT_AS` 256 MiB, then a 1 GiB reservation, nothing
    touched: an allocation failure, `SIGABRT`), each in a guest of its own,
    and `kill -9` from outside (the OOM killer's way); protocol garbage on
    stdout and a 5 MiB answer (`Killed`, "invalid message from guest",
    "exceeds the limit"); a restart loop (three exits restarted after 2,
    10, 30 s, a fourth fault `failed`, a turn meanwhile without it,
    `retry_package`); a panic of the `plugins` row (`Failed`, then its
    restart; the guest comes back as a new process with no fault of its
    own; Skill and MCP server untouched, the server not restarted). Linux
    with a sandbox runs all; elsewhere the guest checks skip and the
    content ones run (hooks: Unix). The row's panic goes through
    `AgentApplicationService::panic_in_plugin` (test-support): a real panic
    inside the host's fault guard, owned by the row. Stress: 10 rounds × 4
    parallel copies, 40/40 passed (about 90 s a copy).
  - Fixed: `/warnings` did not show a guest that failed (§5.1 says it
    does): it now lists, under "installed plugins", a package whose code
    is `failed` (`its code failed: …`) and its guest's issues (TUI test
    with a WebAssembly package whose module does not load).
    `plugins info` and `plugins retry` printed the guest's state the
    moment the command's own Agent started, so an enabled, granted guest
    always read `stopped`; both now wait as `grant` and `dev` do
    (`settled`; `plugin_examples` checks `info` shows `running` for the
    Rust and WASM examples). `settled` itself waited its whole 30 s for a
    guest that cannot start, in safe mode or with the `plugins` row off
    (`grant`, `dev` and now `info`); it waits on `stopped` only while that
    row is pending or active (`plugin_development` times `--safe-mode
    plugins info`).
  - Smoke (debug binary, a fresh `HANAFUBUKI_AGENT_HOME`, a 50x160 pty, a
    scripted Responses server): `plugins new hello`, `cargo build`, the
    program copied into `package/`, `plugins validate`, `plugins add`,
    `plugins grant`; the TUI: `/plugins` lists `hello@local`, its page's
    Code row `running`; the model calls `hello__greet`, approved with `y`,
    the turn answers; `kill -9` of the guest from outside: the Code row
    `restarting: the guest process exited (signal: 9 (SIGKILL))`, then a
    new process and `running`; three more kills (after 10 s and 30 s
    restarts) leave it `failed`, which `/warnings` shows; **Retry plugin**
    runs it again; Space turns it off (its process gone), **Uninstall
    plugin**, `/exit` with code 0 and no guest process left.
  - Docs (both languages): a new "Plugins" page (installing, consent,
    lifecycle, failures) after Skills; architecture's "Not built" says
    a Skill is not installed on its own (it comes with its plugin), npm
    is "not supported"; `/warnings` (terminal-interface, architecture,
    writing-plugins); `disabledPlugins = ["plugins"]` also stops the
    code (config-reference); `AgentPackageGuest`/`GuestStatus` fields
    (embedding); `info`/`retry` (cli-reference); the `renderer` wording
    (plugin-rules); the new suite (building-testing); the index.
  - Open: a hooks file that is JSON of the wrong shape (`{"hooks": 7}`)
    is ignored without a diagnostic; a bad hooks file is a hooks
    diagnostic, not one of its package's issues; the plan document can be
    retired now that every phase is done (AGENTS.md keeps only an active
    plan under `docs/`).
  - Hanafubuki, at its next pin: `AgentApplicationService::panic_in_plugin`
    (test-support only, additive). Nothing else changes for a host.
- [x] K2: Kardia 0.2.3 (crates.io): typed guest failures, `WasmGuest`,
      contained kardia-hmr threads, refused undeclared provisions, Mach
      ports off on macOS.
  - Done: kardia-core/-macros/-loader/-hmr and the dev kardia-guest at
    0.2.3 (kardia-loader and kardia-hmr still `default-features = false`;
    `cargo tree -e features -i serde_json` shows no `preserve_order`); the
    `plugins new` templates and `examples/plugins/hello-{rust,wasm}` take
    kardia-guest 0.2.3 (their `Cargo.lock`s updated). Faults:
    `GuestHost::call` finds a `GuestFailure` in the call's error
    (`GuestFailure::of`) instead of matching four strings, and fails the
    fiber on every kind but a process guest's `Exited`, `Killed` and
    `Timeout` (its process ended or was killed, so kardia-hmr's exit
    watcher fails the fiber with the exit report; failing it from the call
    too would race that report); a guest's own error stays the call's; the
    call's text is unchanged. WASM: the module is read (`system()`) and
    compiled with `WasmSource::guest` and made a plugin with
    `WasmGuest::plugin(GuestMeta::new(name).provide("plugin"))`: no
    instantiation at load, kardia-hmr checks each instance's meta at its
    start (the `liar` module is still a failed start); `deps` are no longer
    read (the `ModuleGraph`/`OneModule`/`check_meta` are gone). Engines:
    `WasmEngines` in the resolver keeps one `WasmSource` per set of limits
    (fuel, the computed time limit, memory), so guests with identical
    limits share an engine and its `wasm-epoch` thread; pruned at each
    `follow` to the limits a package needs (a running guest keeps its own
    engine alive through its module). Mach ports: the guest host calls
    `set_macos_mach_ports(false)` before each new engine (idempotent; an
    error, a host engine that fixed Mach ports first, is a warning, the
    guests keep Mach ports): signals are Linux's tested path and hosts
    already chain them, Mach ports are a second process-wide mechanism and
    the suspect of P8's macOS hang. Every process that runs guests takes
    the same value, the tests' included, since the call is in the guest
    host. Panic seam: foundation's `register_containment_check(fn() ->
    bool)` (one per process, the first stays) is ORed into
    `is_panic_contained`, so both the Agent's hook and a host's hook that
    asks `hanafubuki_agent::panic::is_panic_contained` see it; the guest
    host registers `kardia_hmr::is_panic_contained` in `GuestHost::build`,
    before any kardia-hmr thread exists (foundation stays free of kardia).
    Provisions: the supervisor's `ServiceChanged` watcher is removed:
    kardia-hmr refuses an undeclared key before anything is bound, so
    there is nothing to withdraw; the refusal is the guest's error like
    any host call's (an `apply` that returns it fails the start; a call
    that returns it fails only that call and the guest keeps running).
    Tests: guests suite `a_guest_that_provides_another_service_is_refused`
    (was `…_is_stopped`: the start fails with `cannot provide "stolen"`,
    no restart; while running the call fails, same process, no restart,
    no binding, the next call works); new
    `a_panic_on_a_guest_thread_is_contained_under_the_agents_hook` (the
    binary re-runs itself as a child with the Agent's panic hook; the test
    logger panics on a marker line the guest writes to stderr, on
    kardia-hmr's `guest-stderr` thread; the child checks the guest was
    killed with `the guest-stderr thread panicked`, restarted, and the
    hook recorded the panic as contained; without the registration the
    child aborts with SIGABRT, checked by hand); foundation unit test of a
    registered check. Stress (10 rounds × 4 parallel copies): `guests`
    40/40, `plugin_crashes` 40/40 (no check skipped). Docs (both languages): plugin-rules (where Agent
    code runs, a guest provides only `plugin`), architecture, internals
    (WebAssembly guests, a failed instance, provisions), embedding (Mach
    ports, kardia-hmr's threads), writing-plugins, contributing (the
    `panic` twin), building-testing.
  - Open: the Mach-port decision is unverified on macOS (the next CI runs
    are the test; the watchdog stays); a host whose wasmtime engine fixed
    Mach ports first leaves the guests on them (warned, not refused); a
    second `register_containment_check` is ignored (one registrant today);
    with engines shared, an engine whose `wasm-epoch` thread died (it
    only increments and sleeps) fails every timed call of every guest with
    those limits, restarts included, until no package needs them (no code for
    it: unlikely).
  - Hanafubuki, at its next pin: Kardia 0.2.3 (additive; a Hanafubuki
    crate on kardia-hmr takes it without `preserve-order`, as before); the
    panic-hook bridge is unchanged (`is_panic_contained` now also covers
    kardia-hmr's guest threads, no third check); `hanafubuki-foundation`'s
    `panic` twin need not take `register_containment_check` (Agent only,
    noted in contributing); a wasmtime engine Hanafubuki makes itself must
    use `macos_use_mach_ports(false)` on macOS (wasmtime panics on a
    mismatch); a guest that provided an undeclared service is no longer
    failed and restarted but refused.

## Plan: background compaction, hierarchical compression, a choice between them (2026-11-09)

The plan is [docs/plan-context-compaction.md](docs/plan-context-compaction.md).
All phases (P1–P11, with P8b) are done, and P12 after the plan; the
open items below stay.

Every §8 decision was accepted as recommended, except decision 12: the
previous plan stays and AGENTS.md allows one plan per piece of work. The
sources are Codex `57a808a392` and billion-context `4be21b08e` (acp-kernel
0.0.189). Each phase is validated, documented in both languages, committed
and pushed, and notes what Hanafubuki must change at its next pin.

- [x] P1: split `service/compaction/` along responsibilities (source and
      plan, a pure replacement build, install, status machine); no
      behavior change, goldens unchanged. Done: `source`, `replacement`,
      `status` beside `admission`, `prepare`, `execute`, `commit`;
      `commit_compaction` 75 lines. Nothing for Hanafubuki: kernel-internal,
      public API unchanged.
- [x] P2: the `[compaction]` setting (`strategy`, `background`,
      `hierarchical`), `AgentSessionConfig.compaction_strategy`, applied
      at turn admission, every transition; the `remoteCompaction` bug
      fixed; the Anthropic empty-`tools` case checked. Done: config
      `settings/compaction.rs` (per-field decoding, the summary-length
      pair checked once both are read), `/config` row, docs. The facade's
      `activate_provider_session` (every turn, `/compact`, plan
      implementation) applies the strategy (`set_session_compaction_strategy`)
      and `activate_model_session` takes a changed `remoteCompaction` into
      the session's `compaction_capability`; both only while idle, both
      persisted without a revision bump (no snapshot shows them, so a
      caller's revision read before the call stays current). Children copy
      the parent's, ephemeral ones run `standard`, `childSessions = false`
      gives a durable child `standard` (applied by the driver at creation
      and at each activation). Tests: the corpora decode as `standard`
      (`persistence_contract::session_config`), the bug's facade regression
      (`sessions::compaction`), the switch between turns. Goldens unchanged.
  - Open: the sub-tables reach no kernel code yet; P3/P7 carry what they
    need as a kernel type in `TurnExtensions` from `collect_turn_extensions`
    (as the sandbox reaches a turn). The `background.thresholdPercent`
    warning is a log line at creation and when the strategy changes.
    `agent_settings_fields()` is the web tools' form (rendered in
    Hanafubuki's Tools panel, flat patch keys), so `compaction` has no
    entry there: it is a nested table patched whole, like `memory`.
  - Done (after P12): the Anthropic Messages local summary. It sent
    history `tool_use` / `tool_result` blocks with no `tools` key
    (`compaction_request` set `tools: []`, and `anthropic_messages/
    encode.rs` writes `tools` only when non-empty). Every compaction
    request now declares the step's tools and their parallel setting
    (`tool_plan.model_specs()`, as the turn's request) with
    `ModelToolChoice::None`, encoded `"none"` (Responses, Chat
    Completions) and `{"type":"none"}` (Anthropic), which every protocol
    already wrote; the prompt and the output handling are unchanged (a
    tool call in the answer is still a protocol error), and the provider's
    cached prefix of instructions and tools stays the turn's. The refusal
    ("Requests which include `tool_use` or `tool_result` blocks must
    define tools") is not stated in the published docs (checked: the tool
    use overview and handle-tool-calls pages); `tool_choice` `none` with
    tools is documented, and the docs' own replay recipe keeps the tools
    and sets `none` to the same end. Not checked against the live API,
    so the docs and comments give as the reason the replay pattern (the
    tools kept, calls forbidden) and the cached prefix, not a refusal.
    The facade's fake server now speaks Anthropic Messages for a provider
    of that protocol (`anthropic_assistant_events`,
    `anthropic_tool_events`, `CapturedRequest.path`) and answers a
    `/messages` request with tool blocks and no tools with that 400,
    taking no answer. Tests: facade `sessions::compaction::an_anthropic_
    local_summary_declares_the_turns_tools_and_calls_none` (fails before
    the fix with the fake's 400); the kernel's compaction contracts
    assert the turn's tools and `None` (the background fake tells a
    compaction by `None` now). Goldens regenerated on purpose:
    `compacted_request.json` and `background_compacted_request.json`,
    whose `compaction_request` gains the next request's 30 tools and
    `parallel_tool_calls: true` (was `[]` and `false`); nothing else
    changed. Docs: architecture (both languages).
  - Hanafubuki, at its next pin (the local summary): goldens to copy,
    `compacted_request.json` and `background_compacted_request.json`. A
    host fake that tells a compaction from a turn by its empty `tools`
    now tells it by `tool_choice` `None`. The facade's test-support
    `CapturedRequest` has a `path` field.
  - Hanafubuki, at its next pin: `AgentSettings.compaction`
    (`AgentCompactionSettings`: `strategy`, `background`, `hierarchical`)
    and `AgentSettingsPatch.compaction` (the table whole, strict); new
    re-exports `AgentCompactionSettings`, `AgentCompactionStrategy`,
    `AgentBackgroundCompactionSettings`,
    `AgentHierarchicalCompactionSettings`; a strategy select belongs with
    memory in the behavior panel, with `settings.fields.compaction*` keys
    of its own. `AgentSessionConfig.compaction_strategy`
    (`#[serde(default)]`, the corpus unchanged); kernel calls
    `set_session_compaction_strategy` and `session_compaction_strategy`.
    `activate_model_session` may now commit (persisting a changed remote
    compaction). Its own constructions of `AgentSessionConfig` add the
    field.
- [x] P3: background compaction in the kernel (slot, launch, adoption,
      `wait_for_pending`, the rebase install, cancellation, shutdown,
      usage, `AgentCompactionPhase::Background`). Done:
      `service/compaction/background/` (`mod`: the slot and the task
      registry; `launch`; `adopt`). The slot is `SessionRuntime.
      background_compaction`, never persisted and not an active operation;
      its run is shared by `Arc` (outcome behind its own lock) so a
      rolled-back commit cannot lose what the task wrote. The task is
      `tokio::spawn`ed and registered in `managed_background` (its own
      `Condvar`) before the record's lock is released; `stop_operations`
      cancels every slot and waits for the set, archive and delete cancel
      and wait for the session's tasks (5 s), a strategy change and a
      committed `remoteCompaction` change cancel. The threshold reaches a
      turn as `AgentBackgroundCompactionThreshold` in its extensions
      (`collect_turn_extensions`, one settings read with the sandbox).
      Launch: in `maybe_auto_compact` on the pre-turn step (with the step's
      estimate) and after each sampling request (with the provider's count
      only, no second estimate). Adoption: non-blocking before every
      automatic check, waiting (`wait_until`, woken by
      `request_turn_cancellation` advancing `terminal`) when a context-limit
      compaction is required and before `/compact`; one in-line pass when
      still over the threshold. `commit_compaction` takes `InstallMode::
      {Cas, Rebase}`. Usage is committed (no revision) when the run ends
      with a sample, so the install adds none and a discarded result is
      counted once. Tests: `compaction_contract/background/` (launch gates,
      a failed attempt, pre-turn and mid-turn adoption with work and world
      state after the snapshot plus a fork after it, the wait's success,
      failure, interrupt and still-over-threshold, `/compact` reuse and its
      manual fallback, a stale window refused at install, usage once and no
      turn budget, shutdown waiting, restart, archive, v1 and v2), and the
      facade golden `background_compacted_request` (HTTP fake, local
      summary, reopened from the JSONL history). `standard` goldens
      unchanged.
  - Deviations: the attempt key is (window, items before the current turn,
    model, implementation): a failed run is retried once another turn has
    completed, not only in a new window. The rebased world-state checkpoint
    is the state where the uncompacted part begins, not the live history's:
    the suffix's own updates (a removal included) must still apply, or
    validation refuses a stale removal. "Stale (model changed)" is covered
    by the window case and the strategy change: the kernel fakes cannot
    change a session's model without a real transport. No Codex code was
    copied, so no notices entry.
  - Done (after P12): the pre-turn in-line install's stale removal. Its
    world-state checkpoint was the whole history's, so a current turn
    whose initial world-state update removed a section failed validation
    (`stale world-state removal`). `compaction_source` now takes the
    checkpoint of the history before the compaction boundary (the current
    turn's `TurnStart` for a pre-turn or background compaction; the whole
    history for the others, unchanged), as the rebase does. What a
    pre-turn install writes changes only for the sections the turn's own
    update touches, which a request renders at their latest update
    anyway. Test: kernel `compaction_contract::automatic::a_pre_turn_
    compaction_keeps_the_turns_world_state_removal` (failed before with
    the validation error). Goldens unchanged (none has a pre-turn
    install). Docs: internals (both languages). Nothing for Hanafubuki.
  - Done (after P12): the rebasing install's boundary. It cut the history
    at `plan.source_items.len()` for its world-state checkpoint and its
    suffix, but `source_items` is the history before the boundary
    projected over the active compression blocks (a session that left
    `hierarchical` with a block, before its first standard compaction),
    which has fewer items: the install then kept compacted items in the
    suffix. The plan now carries `source_boundary`, the number of history
    items before the boundary, which the suffix starts at, and the rebase
    takes the plan's world-state checkpoint (the state at that boundary,
    since the pre-turn fix). The in-line install already used the raw
    boundary (`current_turn_suffix`, the checkpoint). `CompactionSummary.
    compacted_item_count` still counts the items the compaction read (not
    model-visible). Test: kernel `compaction_contract::background::
    adoption::a_result_read_from_a_projected_history_is_installed_at_the_
    history_s_boundary` (a fold of `m00001–m00002`, a switch to
    `background`, a turn launching the compaction and the next installing
    it; before the fix the folded turn's answer stayed after the summary).
    Docs: internals (both languages). Goldens unchanged. Nothing for
    Hanafubuki.
  - Done (after P12): the `/compact` and turn-admission race. A turn
    admitted while `/compact` waited for a background run could take the
    result first; `/compact` then found it spent and compacted with the
    generation and revision it had read (`StaleGeneration`). The run's
    outcome gains `Installing` (taken by a checkpoint, under the outcome's
    lock, in the same check that saw it ready) and `Installed(outcome)`:
    the taker's `InstallClaim` sets `Installed` after its commit and
    `Spent` however else it stops, then advances `terminal`. A waiter
    waits through `Installing`, and one that finds `Installed` gets
    `AdoptedCompaction::InstalledElsewhere`: `/compact` returns that
    outcome (the turn compacts further when it must). `/status` shows a
    result being installed as ready. Test: kernel `compaction_contract::
    background::wait::compact_reports_the_install_of_a_turn_admitted_
    while_it_waited` holds `/compact` at its wait by polling it only from
    the test (current-thread runtime), lets the compaction end and a turn
    install it, then resumes `/compact` (failed before with
    `StaleGeneration`). Docs: internals and terminal-interface (both
    languages). Nothing for Hanafubuki: kernel-internal, public API
    unchanged; a host's `/compact` that waits can now return a background
    outcome a turn installed (`phase: background`).
  - Open: nothing observable tells a test that a turn is waiting on the
    background compaction, so the wait tests hold the compaction until the
    turn is admitted and assert the outcome (one compaction request, the
    background install, no in-line request after an interrupt) whichever
    checkpoint took it.
  - Hanafubuki, at its next pin: `AgentCompactionPhase::Background`
    (`phase: "background"` in `compaction_updated`, an appear);
    `AgentSessionRuntimeSnapshot.background_compaction`
    (`Option<AgentBackgroundCompactionState>`: `running`, `ready`;
    `#[serde(default)]`); new kernel types `AgentBackgroundCompactionThreshold`
    (a turn's extensions carry it from `collect_turn_extensions`; a host
    admitting turns through the kernel itself adds it) and
    `AgentBackgroundCompactionState`. Shutdown's stop step now also waits
    for background compactions. Golden to copy:
    `background_compacted_request.json`.
- [x] P4: background compaction in the interface and docs. Done: the
      terminal interface shows `Context compacted` from every
      `CompactionUpdated` that is `completed` (manual, pre-turn, mid-turn,
      new context, context error, background); `/compact`'s reply carries
      only a failure, so neither a manual compaction nor one `/compact` takes
      from the background shows twice; the cell waits behind a streaming
      message like tool updates. `/status` has a `Compaction` row from the
      snapshot it reads as it runs (`compacting in the background`, or ready
      and installed before the next request). Docs: json-events
      (`compaction_updated`'s `phase`, `reason`, `implementation`, `usage`;
      a background run's one `completed` event), terminal-interface (the
      cell, `/compact`, `/status`), internals (the bridge), architecture
      ("a third compaction path" dropped from "Not built": the three
      implementations and the background strategy are built; the Codex
      row). `with_input_tokens` moved into the facade's `test_support`.
      Tests: `app/tests/compaction.rs` (a background compaction installed
      in the turn that took it shows once, between the question and the
      answer, and once on resume; `/status` while it runs; `/compact` shows
      one cell, replayed once). Goldens unchanged.
  - Done (after P12): a compaction that fails during a turn showed
    nothing in the terminal interface. The bridge turns a
    `CompactionUpdated` that is `failed` with trigger `automatic` into
    `TurnUpdate::CompactionFailed`: a warning cell (`Context compaction
    failed: <error>`, deferred behind a streaming message like the
    compacted cell) kept in `/warnings` under a new source, `compaction`.
    `/compact`'s failure stays its reply only (trigger `manual`), and a
    cancelled compaction shows nothing. The turn that ran it still ends
    with its own failure. A background run that fails sends no event, so
    it still shows nothing (the turn compacts in line when it must). The
    facade re-exports `AgentCompactionTrigger`. Test: TUI `app::tests::
    compaction::a_compaction_that_fails_in_a_turn_shows_a_warning_and_
    compacts_reply_shows_once` (a summary that calls a tool fails both a
    `/compact`, whose reply shows and no warning, and the next turn's
    pre-turn compaction: one warning, listed as `[compaction]`). Docs:
    terminal-interface (the warning, the source) and internals (both
    languages).
  - Open: the status line has no background item; `/status` is the only
    view.
  - Hanafubuki, at its next pin (the failure warning): the facade
    re-exports `AgentCompactionTrigger`; a host can show the same warning
    from `compaction_updated` with `status: failed` and `trigger:
    automatic`.
  - Hanafubuki, at its next pin: the facade re-exports
    `AgentBackgroundCompactionState`; `test_support::with_input_tokens` is
    new. A host's compacted marker belongs on `compaction_updated` with
    `status: completed` (every phase, `background` included), and a status
    view reads `backgroundCompaction` from a fresh snapshot (it changes
    without a revision).
- [x] P5: `crates/compression` core (model, refs, boundaries, parse,
      project, protect, apply, prune, render) with the ported tests;
      notices and attribution. Pure (serde and serde_json only); 361
      ported tests; texts verbatim; tags never on tool-call arguments
      (`TextOnly`, `TextAndToolResults`, `None`); host predicates became
      data (`Message.protected`); `NOTICE`, the notices policy and the
      contributing page credit billion-context and acp-kernel.
  - Open (P5): recommend, nudge, the status line and `processTurn` are
    P6; `rebuild`, the decompress primitives and MEDIA REFS came with
    P9's crate half; absorb and rules with P10's crate half; timestamps
    with P12. The kernel's dependency row allows the crate
    in P7. No ledger `seq`, `base#part` ids or SKILL.md paths (the
    Agent has one item per message part).
  - Hanafubuki, at its next pin: a new published crate,
    `hanafubuki-agent-compression`; `NOTICE` gains billion-context.
- [x] P6: recommend, nudge, truncation, latches, the per-request
      pipeline, prompts (byte-exact golden), reports. Done: `recommend`,
      `nudge` (decision, low-water stamps, texts), `truncate` (latched
      emergency clips, terminal escape), `latch` (view latch), `turn`
      (`process_turn` in the kernel's step order), `prompts` (the five core
      tool schemas, `build_compress_system_prompt`, the rule texts and the
      opt-in language rule), `report` (`acp_status`, recap, status panel,
      `acp_cache` report and fold economics), `Config::validate`. 700 tests
      (315 new) with the kernel's `prompt-function-default.txt` byte-exact
      and eight more texts byte-exact against the kernel's own output.
  - Open (P6): relocation is not in `process_turn` (not ported, plan
    §9); timestamps came with P12. The content store, absorb, crush and
    filters came with P10's crate half, and the view latch's digest holds the settings
    of those that are on. Ported with P9's crate half: `open-objectives`' decompress
    assertion, the decompress primitives of `decompress-report`, the
    search and preview cases of `truncate-surrogates`. Not ported: host-set `dangerous`
    ranges, checkpoint carriers and media gaps, packs and section
    overrides, tokenizer injection, the text and hybrid protocols.
    Screening cost units × messages (release: 0.33 s at 10k messages,
    1.26 s at 20k, against the kernel's 0.57 s): each unit's integrity
    gate walked every turn group and tool call of the history, and its
    boundaries every active block's messages. Fixed with P8's profile:
    the gate visits only the groups and pairs a unit's ids touch, and the
    blocks' positions are computed once per call (see P8). Tool
    parameters are `serde_json::Value`, so their keys are sorted. The
    cache report shows times as UTC plus an offset the host passes; the
    kernel's fold-attribution offset quirk is kept; the per-tool listing
    sorts lowercase names instead of the locale's collation.
  - Hanafubuki, at its next pin: nothing to change; the compression crate
    gains public items only.
- [x] P7: `hierarchical` in the kernel (projection, the five core tools,
      the nudge, fallbacks, terminal escape, state), its goldens. Done:
      kernel `service/compression/` (`adapter`: history items to the
      crate's messages, ids `<source session>:<item>`, `CompactionSummary`,
      `ProviderCompaction` and a compacted prefix protected, the view back
      to items with tags, compacted carriers, clips and anchors as summary
      items; `tools`; `receipts`, ported from billion-context's host with
      the marker). `regular_model_request` projects while the strategy is
      `hierarchical` or the session has active blocks (`CompressionMode::
      {Off, Carry, Hierarchical}`); the static prompt after the Plan suffix,
      the nudge or status line as the last input and the five core tools
      only under `hierarchical`. Only `begin_sampling` keeps the state the
      projection returns (in memory, persisted by the next commit); the
      estimate and the post-install check build the same request and drop
      it. `PersistedAgentRuntimeState.compression` (`#[serde(default)]`,
      skipped when none, an unreadable one dropped with a warning); the
      runtime keeps it behind an `Arc`; fork at head copies it. Settings:
      the facade puts `AgentHierarchicalCompression` in each turn's
      extensions (P3's route); a turn without it, and `/compact`, use the
      defaults; no known window, no pressure. `compress`/`decompress` run on
      the record for their result and are applied again under the commit's
      lock (`commit_history_with`), so the state and its receipt are one
      commit and two calls of a step apply in order. Thresholds off under
      `hierarchical`; the context-error retry, `/compact`, `new_context` and
      the terminal escape (from the estimate's projection) run a standard
      compaction of the projected view; after it the blocks deactivate,
      and a session that switched away drops its state. A projection panic
      sends the full history and sets the turn's `compression_fallback`
      (standard for the rest of the turn); a tool panic is its error. The
      crate's state types derive `Eq` (the one change in
      `crates/compression`). Tests: `compaction_contract::hierarchical`
      (projection, tier 2, the read-only tools, refusals, the nudge, the
      threshold off, the context error, the terminal escape, panics,
      switching both ways, a restart), and the facade golden
      `hierarchical_requests` (nudge with prompt, tools and tags; after a
      fold; after a tier-2 distillation and a `decompress`; after the switch
      to `standard`). `standard` goldens unchanged; corpora decode.
  - Deviations: no warning event for a projection panic, only the log
    line (the event kinds come with P11). The receipts keep billion-context's
    texts but not its proxy's loop guards, breaker, recovery hints or
    degenerate-fold notes. `decompress` restores inline only (no `toFile`,
    range or image restore); `search_context` searches the block summaries
    (substring, as billion-context's host) without plan steering or the
    folded originals, with a fresh `DocCache` per call; `acp_cache` has one
    sample (the last response) and no cached tokens: the model API reports
    none. Budget pressure still compacts under `hierarchical`. A
    `[Compression FAILED`/refused receipt is an error output (`is_error`).
  - Open: the state is persisted with every checkpoint: measured in P8,
    see its deviations. The projection ran twice per step under
    `hierarchical` (the estimate and the request): done after P12, the
    request takes the estimate's projection (see P8). Forks, children
    and an unreadable state: done in P8. The emergency voice nudges from
    `maxContextLimitPercent`; post-install `ContextBudgetExceeded` still
    applies to a terminal-escape compaction. The panic-containment test's
    debug-only overflows: replaced in P8 by a test-support marker. The
    architecture page's "Not built: a third compaction path" is P4's.
  - Hanafubuki, at its next pin: `PersistedAgentRuntimeState.compression`
    (its own constructions add `compression: None`); new kernel types
    `AgentCompressionState` (the crate's `State`) and
    `AgentHierarchicalCompression` (a turn admitted through the kernel itself
    adds it to the extensions for `[compaction.hierarchical]`);
    `AgentCoreTool` has five more variants, `ALL` is 8, `STANDARD` and
    `COMPRESSION` split it; `AgentToolCatalogContext.compression_tools`;
    `RESERVED_TOOL_NAMES` has 9 names (`compress`, `decompress`,
    `search_context`, `acp_status`, `acp_cache`). Golden to copy:
    `hierarchical_requests.json`. `NOTICE` names
    `crates/kernel/src/service/compression`.
- [x] P8: forks, resume, children, `rebuild`; state size; switching away.
      Done: kernel `service/compression/inherit.rs` (`inherited_state`). A
      fork before a turn (so `edit_from_turn`) and a child forked before
      its parent's running turn adopt the source's state through the
      crate's new `adopt_fork_state` (billion-context's
      `src/fork-adoption.ts`, `seedAllRefs` always on): a block whose
      covered messages are all present is kept active, with the blocks a
      tier block consumed as inactive records; expanded, consumed and
      straddling blocks are dropped; every present message keeps its ref
      and token snapshot; the parent's highest ref is reserved
      (`REF_FLOOR_ID`, a `by_raw` entry naming no message); the orphan,
      truncation, deferral and view latches carry over; the nudge, stats,
      terminal streak, absorbed records and rules start fresh. A child at
      its parent's head copies the state (as a fork at head does, so a
      `/side` runs `Carry`); an empty child has none. With no source
      state, and when a session is loaded without one (none written, or
      undecodable and dropped by `tolerant_compression_state`), the state
      is rebuilt from the window's `compress` calls (`rebuild_state`) with
      the source's last turn's tuning (the defaults at load) and the
      compress limits and recent zone relaxed; the crate's `rebuild_state`
      now skips a call whose receipt is `[Compression FAILED`/`refused`/
      `no-op` (a change to the ported function). A window holding a
      `CompactionSummary` or `ProviderCompaction` is never rebuilt; a
      window without `compress` calls gets no state (a `standard` fork
      stays `None`). The panic-containment test now panics through
      `test_support::COMPRESSION_PANIC_MARKER`: a view-latch digest the
      algorithm never writes, checked first inside the projection's and
      the tools' `catch_panic` closures under `cfg(any(test, feature =
      "test-support"))` (per session, no record field or global, nothing
      in production builds); the crate has no panic a malformed state
      reaches in release (its `u32` counters only wrap there). It passes
      in release. Tests: crate `fork` (7, ported from
      `fork-adoption-plan.test.ts`) and a `rebuild` case; kernel
      `compaction_contract::hierarchical::forks` (a fork before the second
      turn keeps `b1` and drops `b2`, keeps `m00001`/`m00002`, reserves the
      parent's highest ref and tags its own message above it, and shows
      the summary without its carrier; a fork before the fold turn keeps
      both; a lost parent state is rebuilt with the same blocks and refs,
      and a fork before the folds gets none; children at head and empty;
      a lost state rebuilt at restart, an undecodable one read as none);
      facade `tests::compression` (switching to `standard` end to end: the
      `compress` call stays as a carrier with tags and the summary, no
      prompt, tools or nudge, then a standard compaction ends the blocks;
      `edit_from_turn` after a fold keeps it). The kernel's hierarchical
      tests pass in release too. `standard` and `hierarchical`
      goldens unchanged (the golden's helpers moved to
      `tests/compression.rs`).
  - Measurements (release, a scratch harness in `crates/compression/tests`
    run with `cargo test --release -- --ignored --nocapture` and then
    removed: rounds of a short user message, three `read` calls with
    ~1.5 KB results and a ~1.6 KB answer, each round folded by a
    `compress` in the next): the state is about 89 B per message (the ref
    map both ways) plus about 560 B per block (ids and a 120-character
    summary): 42 KB at 500 messages and 49 blocks, 85 KB at 1 000/99,
    174 KB at 2 000/199, 353 KB at 4 000/399, 709 KB at 8 000/799.
    Counted in the kernel (a temporary trace in `conversation_commit`): a
    sampling step writes one to three full runtime checkpoints carrying
    the same state, and the state changes at every step (the request mints
    the new messages' refs); at a few blocks it is under a third of a record, at
    1 000 messages over 90 %. The rollout JSONL is append-only, so the
    state's share of a session's disk growth is O(steps × state).
    `process_turn` per call: folded 3.8 ms at 1 000 messages, 7.8 ms at
    2 000, 16 ms at 4 000, 40 ms at 8 000; unfolded 16 ms at 1 000,
    38 ms at 2 000, 175 ms at 5 000, 677 ms at 10 000, with or without
    the nudge's screening; cloning the state 0.2–2 ms. A step projected
    twice (its estimate and its request; once since, see the deviations).
  - Deviations: the state is still written with every checkpoint. The
    measurement says it matters for long sessions, but writing it only in
    checkpoints after a change removes only the one-to-three duplicates
    per step (it changes every step) and leaves the O(steps × state)
    growth, while needing a tri-state field (absent = unchanged, none,
    a state), a rollback-safe marker in the kernel and the history store
    carrying the last state forward in `runtime_json`, materialization
    and lag catch-up: a persisted-format change across two crates, left
    for a decision (done in P8b, incrementally). The double projection was
    fixed after P12: the projection is kept on the session
    (`service/compression/cache.rs`, in memory) with its inputs (the
    history items, the state and the content store by identity, the
    settings, token count, mode and interrupt anchor by value; the
    clock's reading is left out, so with timestamps on a reused
    projection keeps the estimate's stamps), and the step's request takes
    it; the estimate counts tokens without cloning the history, and the
    projection hands the history, the state and the store on by `Arc`.
    Measured end to end (a scratch kernel harness, removed: a session
    seeded through the history store with the rounds above, one-step
    turns, release; 880 messages is 990 items, about the most a window
    holds under `FIRST_BATCH_HARD_LIMITS`' 1 024 items and 4 MB, so the
    kernel never projects the 10 000-message windows the crate is measured
    at): a one-step turn under `standard` 1.2 / 2.2 / 3.7 ms at 200 / 480 /
    880 messages; under `hierarchical`, unfolded 6.7 / 16.1 / 32.0 ms
    before, 4.2 / 8.5 / 15.1 with the crate's fixes alone, 2.8 / 5.6 /
    9.5 after; folded (every round but the last two) 2.6 / 4.6 / 7.2 ms
    before, 1.6 / 2.5 / 3.8 after.
  - Done after P12: the crate's per-call cost was superlinear in the
    window (677 ms at 10 000 unfolded messages, without the nudge too).
    Profiled (`tests/compression/scale.rs`, an ignored release harness:
    the rounds above, with the first half or all but the last two rounds
    folded, quiet and at 80 % of a 200 000-token window): screening was
    units × messages (the integrity gate walked every turn group and tool
    call per unit, the boundaries every active block's messages per
    unit); the rest was token counts, UTF-16 lengths and tool-pattern
    lowercasing allocating per message. Now the gate is indexed by id,
    the blocks' positions are computed once per call, ASCII text and
    tool names take a fast path, and the recent zone counts tokens only
    for the tail it walks. The views are the same bytes (the harness
    prints a digest of each output: identical before and after). Release,
    quiet (under pressure the same within noise), before → after:
    unfolded 12.1 → 4.8 ms at 1 008 messages, 34 → 10.6 ms at 2 008,
    168 → 29 ms at 5 008, 603 → 62 ms at 10 008, 2 424 → 142 ms at
    20 008; half folded 8.5 → 3.6, 24.7 → 7.4, 124 → 20.5, 455 → 42,
    1 835 → 98 ms; folded 1.8 → 1.5, 3.4 → 3.0, 8.2 → 8.1, 16.8 → 15.8,
    38 → 38 ms (linear already: a few passes over the history). A child forked before its
    parent's running turn has no test of its own (it shares
    `inherited_state` with the fork before a turn). The rebuild at load
    uses the default tuning, so a session with `protectedTools` may number
    a replay differently; an approximation, as the crate's doc says.
  - Hanafubuki, at its next pin: nothing to change; persisted format and
    public kernel API unchanged. New public items: the compression
    crate's `adopt_fork_state` and `REF_FLOOR_ID`, and the kernel's
    `test_support::COMPRESSION_PANIC_MARKER`. Behavior: forks before a
    turn, `edit_from_turn` and children now carry the hierarchical state
    (a child at head, `/side` included, projects the parent's blocks), and
    a session whose state is missing or unreadable gets it back from its
    `compress` calls at load.
- [x] P8b: the hierarchical state persisted incrementally. Done: a chain
      of links (`kernel model/compression/chain.rs`), one in each runtime
      checkpoint that changed the state, as
      `PersistedAgentRuntimeState.compression_update`
      (`AgentCompressionUpdate`: `snapshot`, `delta`, `dropped`, and
      `unreadable` for one that cannot be read; `#[serde(default)]`,
      skipped when none, so `standard` records are unchanged). A delta
      (`AgentCompressionDelta`) carries its `seq` after the snapshot, the
      ref bindings minted or changed, the blocks made or changed other than
      in age (whole), the age of the blocks that only aged (`survived`),
      the token counts added and dropped, and the other fields whole when
      one changed. A snapshot writes the ref map one way (`by_raw`;
      `by_ref` derived on read, without `BLOCKED` and `REF_FLOOR_ID`): 15 %
      smaller. The kernel diffs against `SessionRuntime.compression_head`
      (`next_link` in `conversation_commit`): staged with the commit and
      restored with the runtime on rollback, so it is the last durable link
      once the commit settles (staged commits are serialized by
      `persisting_revision`). No head after a load, a fork or a child: the
      first link is a snapshot; so is a change a delta cannot carry (a
      binding or block gone, blocks reordered), and so is the link once
      the deltas since the snapshot would outweigh it. A checkpoint that
      changed nothing carries no link. The history store keeps the links
      as rows of `agent_compression_links` (a snapshot or `dropped`
      replaces the session's rows; deleted with the session), strips the
      state from `runtime_json`, and folds the rows
      (`AgentCompressionChain`) into `compression` when it loads a runtime
      (`load_runtime_state`, recovery); the index is schema 6, so an older
      index is rebuilt from the rollouts once, a record from before the
      chain reading as a snapshot. `EphemeralAgentHistoryStore` folds the
      same way. A chain that does not fold (no snapshot, a skipped `seq`,
      a delta naming a block or token count the state lacks, an unreadable
      link) loads as none, and P8's load-time rebuild replays the
      `compress` calls. Tests: kernel `model::compression::chain` (10: the
      first link a snapshot written one way, many steps folding back to the
      exact state with periodic snapshots, a delta carrying only the
      change, nothing for an unchanged state, snapshots for removals,
      `dropped`, broken chains repaired by the next snapshot, a delta for
      another state, the ref floor); `persistence_contract::
      compression_chain` (a failed commit rolls back the head: the next
      one is delta 1 and the store folds to the live state); facade
      `tests::compression` (a folded session over 14 turns: the rollout
      holds snapshots and more deltas, a restart reads back the exact
      state, so does a schema-5 index rebuilt from the rollouts, and with a
      delta row deleted the state is rebuilt from the `compress` calls; a
      fork from `edit_from_turn` reads back after a restart; a `standard`
      session writes no `compression` key and no row). Goldens unchanged.
  - Measurements (release, the P8 recipe as a temporary kernel test:
    rounds of ten messages, each folded by the next round's `compress`,
    `process_turn` per round, one step per round): the state P8 wrote
    whole into every checkpoint was 92 KB at 1 000 messages, 187 KB at
    2 000, 378 KB at 4 000, 759 KB at 8 000 (one-way refs: 78, 159, 321,
    645 KB); the link a step now writes once is 3.0 KB, 4.1 KB, 6.3 KB,
    10.7 KB, growing with the active blocks' ages (~13 B each; every block
    stays active in this recipe, the worst case). Over 500 steps the state
    took 117.7 MB of rollout per checkpoint copy before (×1–3 a step);
    now 4.9 MB in all, 23 snapshots and 477 deltas included (24× less
    before the per-step multiplier).
  - Open: the `survived` ages are O(active blocks) a step; a session that
    keeps hundreds of tier-1 blocks active pays ~13 B each per step.
  - Hanafubuki, at its next pin: `PersistedAgentRuntimeState.
    compression_update` (its constructions add `compression_update:
    None`); `compression` is now what a store loads (the folded chain)
    and what records written before the chain carry. New kernel types
    `AgentCompressionUpdate`, `AgentCompressionDelta`,
    `AgentCompressionChain` (a host store folds its links with it). The
    history index is schema 6 (`agent_compression_links`); an older index
    is rebuilt from the rollouts at the first open.
- [x] P9: decompress, search, media refs. Done (kernel half):
      `decompress` (kernel `service/compression/tools/decompress.rs`) has
      billion-context's host's four modes: the whole block inline (one tier
      up or `full`; an active block flagged restored inline, the re-fold
      hint), a range (`startId`/`endId`, the block's messages whose refs
      fall in it, read from the history, no state change), `toFile` (the
      text written to the session's export file, the output its path) and
      `imageRef` (`list`, an `mNNNNN` ref or a MEDIA REFS handle: the
      message's images attached to the output). `full` and range restores
      attach their messages' images too. The schema gains the host's
      `startId`/`endId`/`imageRef` and its image note
      (`core_tools/compression_spec.rs`, a derived file). Exports go through
      the new `AgentHistoryStore::write_session_export` (default: refused):
      the file store writes `history/attachments/<session>/exports/
      decompress-<block>[-full|-<start>-<end>].txt` (staged, published,
      replaced by name; the path rendered `native()`), so session deletion
      removes them with the attachments and the blob sweeps never see them;
      `execute_compression_tool` writes with the record's lock released,
      and the commit keeps that output (`CompressionToolRun.export`) while
      re-running the call only for its images and state. Restored images
      are the history's own `AgentImageAttachmentRef`s, written into the
      committed `ToolOutput.images` (`CompressionToolRun.images`), so a
      request loads them like any tool output's and a model without image
      input gets them stripped (an image restore is refused then).
      `search_context` (`tools/search.rs`) searches the summaries and then
      the folded originals (`folded_message_inputs` and `message_docs`),
      both `Substring` with minimum score 1e-9 as the host: one literal
      occurrence is a hit, none is no match (the hybrid's fuzzy n-grams make
      weak matches of garbage queries). Rendering is the host's: covered
      ref spans, the 【】-marked evidence snippet around the rarest term,
      the truncation note, the narrowing tip, the original-text section
      (one line per block no summary line listed, with its best message's
      ref). The `DocCache` is per session in `SessionRuntime.
      compression_search` (shared `Arc<parking_lot::Mutex>`, never
      persisted, 8 Mi units). MEDIA REFS: the adapter already passes a
      message's attachment ids as its image handles, so a fold lists them;
      a kernel test pins it and the re-access. `acp_cache` is unchanged: the
      model API's `ModelUsage` has `input_tokens` and `output_tokens` only.
      The crate's `Role::name` is public now. Tests: kernel
      `compaction_contract::hierarchical::tools` (range and its refusals;
      search over folded originals, summary hits first, no match; a folded
      image in MEDIA REFS, listed, restored by ref, by handle and by `full`,
      its pixels in the next request; `toFile` on an ephemeral store fails
      with the content inline), history `an_export_is_replaced_by_name_and_
      deleted_with_its_session`, facade `tests::compression::decompress_to_
      a_file_writes_under_the_session_and_goes_with_it` (real store: the
      path under the data directory in the output, the file's text, a
      `toFile` path ignored, removed by `delete_session_confirmed`). The
      kernel cannot see the data directory (the history crate depends on
      it), so the toFile contract is tested in the store and the facade.
      Golden `hierarchical_requests` regenerated on purpose: only
      `decompress`'s schema and description in `nudged_request` changed (its
      `b1` restore is unchanged); `standard` goldens unchanged.
  - Deviations from the host: a range is returned as the call's output
    (the host acks and injects the text into the next request) and needs no
    content store (the history holds the originals); `toFile` is a switch,
    not a path (writing where the model names would be an unapproved
    write), and the output says so; no automatic spill past 10 000
    characters (the request's tool-output cap bounds a large inline
    restore); images come back attached, not as files to `read`; no
    plan-aware steering (off by default in the host behind `search.
    planAware`, and the Agent has no such setting), no derived or foreign
    session search, no archived-block receipts.
  - Open: an export path under the data directory is readable by the
    `read` tool only where the data directory is not a protected root
    (standalone: none are).
  - Done: a restore's images fit the turn's tool-output budget. The commit
    charges the budget with the output and its images and fails the turn
    past it, so `decompress` (`fit_images`, given what is left of the
    budget by `compression_tool_output`) attaches a `full`, range or
    `imageRef` restore's images in history order only while they fit with
    the output, and a note ahead of the body names the rest by handle (at
    most 50) to restore one at a time by `imageRef`; an `imageRef` restore
    none of whose images fits is refused and the turn goes on. The commit's
    re-run of a compression tool now bounds its inline output with
    `canonicalize_tool_output` and records that truncation, where it kept
    the unbounded output and the first run's truncation. Tests: kernel
    `compaction_contract::hierarchical::tools::a_restore_attaches_only_the_
    images_the_turn_has_room_for` (five 100 kB images, a 256 000-byte
    budget: two attached, three named, the third refused in that turn and
    restored by handle in the next) and `a_restore_committed_is_bounded_as_
    any_output`. Goldens unchanged.
  - For P10: `apply_retrieve`'s `Export { file_name, .. }` can go through
    `write_session_export` as `decompress`'s does (`ToolExport`); the
    content store's originals need files of their own (§4.6), which
    `attachments/<session>/` holds and deletion removes.
  - Crate half (before the kernel half): decompress, search, media,
    rebuild. `decompress` (`collect_block_content` one tier up or
    full, `mark_block_restored_inline`, `expand_block`, which also sets
    `expanded` since `sync_blocks` reactivates any other block, and the
    kernel's lineage helpers), `search` (`search_blocks` over `block_docs`
    and `message_docs`; hybrid, BM25, fuzzy, substring; a `DocCache` the
    caller keeps per session instead of process-wide state;
    `folded_message_inputs` gives the folded originals), `media` (MEDIA
    REFS appended to a block's summary at a fold and carried through
    merges and refolds; a message's handles are its `images`, inline
    `data:` and signed or expiring URLs excluded), `rebuild_state`
    (replays the `compress` calls; refs are minted with the view's
    protection, unlike the kernel, so a protected `compress` call takes no
    number and later refs stay where the model saw them). 88 new tests.
  - Unfaithful: no dictionary segmenter, so a CJK run of two or more
    characters is its bigrams and a one-character run itself (the
    kernel's ICU words and its all-OOV singles are not reproduced; the
    tests pinning ICU's words, `试验证明` against `验证`, and the `缓存`
    rescue's fuzzy-only cap are not ported); bigrams are of characters,
    not UTF-16 units; a system message weighs as a tool's. Not ported:
    the algorithm registry, semantic and asynchronous search, the V8
    160K-document case, media pinning (no message is unrecoverable: the
    host keeps the full history).
  - Hanafubuki, at its next pin: `AgentHistoryStore::write_session_export`
    (a default that refuses; a host store that keeps files implements it
    to make `toFile` work, and must remove the files with the session).
    If Hanafubuki's `protected_roots()` covers its app data directory (the
    Agent's test support models a host so), the model cannot `read` an
    export there: check at the pin, and allow the session's `exports/` for
    reads or expect `toFile` to be useless. The
    `decompress` schema changed (model-visible): golden to copy,
    `hierarchical_requests.json`. The compression crate's `Role::name` is
    public. `NOTICE` names `crates/kernel/src/core_tools/compression_spec.rs`.
- [x] P10: the opt-in features (absorb, content store, crush, rules,
      filter), off by default. Done (kernel half): `[compaction.
      hierarchical]` `absorb`/`contentStore`/`crush`/`rules` reach the
      kernel as four fields of `AgentHierarchicalCompression`, which turn
      on the crate's `Config` sections of those names (their other settings
      keep the crate's defaults; no new keys). `project_request` runs
      `process_turn_with` with the session's content store (cloned only
      while the feature is on) and no filters (the Agent has none, so the
      filter step stays empty and `messageFilters` is not a setting). Three
      more `AgentCoreTool`s, `Absorb`, `AcpRetrieve`, `AcpRule` (schemas
      from the crate; `ALL` 11, `RESERVED_TOOL_NAMES` 12), planned after the
      five core ones only while their feature is on (`planned_tools`;
      `AgentToolCatalogContext.compression_tools` is now that list); all
      three are applied again at their commit like `compress` (`acp_retrieve`
      counts its retrievals in the state). `absorb` (`parse_absorb_input`,
      `apply_absorb`; "already absorbed" changes nothing), `acp_rule`
      (`execute_rule`), `acp_retrieve` (`apply_retrieve`, `note_retrieval`;
      an `Export` goes through `write_session_export` as `exports/<ref>.txt`
      with `build_retrieval_pointer`'s path, and comes back inline, framed,
      with a note when the store keeps no files). `ToolExport` carries its
      kind (`tools/export.rs`). A `compress` that folds calls
      `store_covered_originals` with the blocks it made or refolded, at its
      commit. Instructions under `hierarchical`: the static prompt, then
      `build_absorb_system_prompt` while `absorb`, then `RULES_USAGE_PROMPT`
      and `format_rules_for_prompt` (nothing when empty) while `rules`.
      The content store lives in `SessionRuntime.compression_runtime`
      (`CompressionRuntime`, never in the records); only `begin_sampling`
      keeps the one the projection returns (when it grew); after
      `begin_sampling`'s and a compression tool's commit the kernel writes
      it whole, lock released, when it has more entries than the copy
      (`persist_content_store`, entries only ever added) through the new
      `AgentHistoryStore::write_compression_store` (default refuses: memory
      only) and reads it at load with `load_compression_store` (default
      none). The file store: `attachments/<session>/compression-store.json`
      (staged and published like the exports; the blob sweeps read only
      `blobs/`; removed with the session); an unreadable one is renamed to
      `compression-store.unreadable-<millis>.json` and reads as none.
      `EphemeralAgentHistoryStore` keeps a map. At load, a fork and a child,
      an entry is kept only while the state binds its ref to the same
      message (a rebuilt state may number differently). Consistency: the
      store is a cache of the history; a placeholder is built from the
      original alone, so a lost entry is stored again byte for byte at the
      next projection; only folded originals the window no longer holds
      are lost (`acp_retrieve`: "not found"). A user interrupt of a
      `hierarchical` turn (`stage_terminal`) keeps the token count its last
      request was projected with; later projections call
      `NudgeState::absorb_aborted_turn_growth` until it books (in memory
      only). The features act only under `Hierarchical`: a session that
      switched away (`Carry`) keeps absorbed pairs hidden, shows stored
      originals again and gets no demand. Tests: kernel
      `compaction_contract::hierarchical::opt_in` (8:
      defaults change nothing; absorb asks, hides and records; crush folds
      JSON so absorb does not ask; the content store's placeholder is the
      same bytes on every request, retrieval inline when the store cannot
      export, a made-up ref, the store read back after a restart; a fold
      stores its covered originals and one is retrieved inline; a rule in
      the instructions; an interrupt booked only once the count passes it;
      after a switch to `standard`, no placeholder or demand),
      history `a_content_store_is_read_back_set_aside_when_unreadable_and_
      deleted_with_its_session`, facade `tests::compression::the_content_
      store_is_written_under_the_session_and_goes_with_it` (the file, the
      export and its path, both removed by `delete_session_confirmed`).
      `standard` and `hierarchical` goldens unchanged (no new golden: the
      contract tests pin the texts).
  - Open: `absorbed` and `rules` ride in the chain's `rest` on every
    change; no test restarts through the chain with a record or a rule in
    it (the chain tests are generic). The projection clones the content
    store (all originals) while the feature is on, once a step since the
    request takes the estimate's projection (P8; it was twice); taking
    the store by reference and returning only what was added would be a
    crate API change, not measured as needed; the file is rewritten whole when it grows, synchronously
    on the turn's worker (as the exports). A fork or child writes its copy of the store only at its
    first request. The abort anchor is not persisted (a restart forgets
    an interrupt). A `/compact` outside a turn plans no opt-in tool (no
    turn extensions: the defaults). The status report's store counters are
    not ported (the crate's note).
  - Crate half (before the kernel half). `absorb` (the `[ACP absorb]` demand, the
    candidate rules, `apply_absorb`, `parse_absorb_input`, hiding the
    pair, the system-prompt section), `store` (`ContentStore`: plain data,
    SHA-256-keyed texts and per-ref entries, persisted by the host apart
    from the state; arrival-time placeholders, re-projected byte for byte
    when raw bytes come back; `store_covered_originals` for folds;
    `apply_retrieve` answers inline or `Export { file_name, .. }`, after
    which the host writes the file and builds the pointer with the path
    it chose), `crush` (json-fold, code-trim, log-select; the two-tier gate
    `evaluate_tool_result`), `rules` (`acp_rule` records with
    `execute_rule`, the host's one-call API, and the prompt section),
    `filter` (a `MessageFilter` trait the host passes per call; no
    built-ins), the three tool schemas verbatim (`acp_rule`'s is
    billion-context's host's). `process_turn_with(input, TurnExtras {
    content_store, filters })` returns `(TurnOutput, ContentStore)`;
    `process_turn` is unchanged. `NudgeState::absorb_aborted_turn_growth`
    books an interrupted turn (#5415's host side). `Config` gains
    `absorb`, `crush`, `content_store`, `rules`, `message_filters`;
    `State` gains `absorbed`, `rules`, `next_rule_id`; `Stats` gains
    `absorbed_tokens`, `stored_count`, `retrieval_count`. New
    dependencies `regex` and `sha2`, and serde_json's `float_roundtrip`
    (unified across the workspace): crush writes numbers as JavaScript
    does, which needs them parsed exactly. 130 new tests, one a golden
    recorded before the port that pins that defaults change no turn; crush was
    checked byte for byte against the kernel's own code over a generated
    corpus (783 inputs).
  - Unfaithful: tool names are fixed (`absorb`, `acp_retrieve`), not
    settings; a target an active block covers is refused as not visible
    (the host passes the full history); tokens are the crate's estimate
    throughout; crush keeps an object's keys in document order where
    JavaScript moves integer-like keys first. Kept from the kernel: its
    code scanners never leave a string once a quote opens one, and its
    log selection keeps lines in the order they were chosen. Not ported:
    the crush and filter registries (and their plugin-contract tests),
    `restampContentStore` and `ccr-restamp` (no ledger restamp), the
    legacy `acp_retrieved_` injections, the status report's store counters,
    absorb's stale-flag age (the host's).
  - For the kernel half: offer `absorb_tool`/`retrieve_tool`/`rule_tool`
    only while each feature is on, with `build_absorb_system_prompt`,
    `RULES_USAGE_PROMPT` and `format_rules_for_prompt` in the
    instructions; persist `ContentStore` beside the state; call
    `store_covered_originals` after a fold and `note_retrieval` after a
    hit.
  - Hanafubuki, at its next pin (crate half): nothing to change; the
    compression crate gains public items and fields (`Config`, `State` and
    `Stats` are built with `Config::new` and `Default`).
  - Hanafubuki, at its next pin (kernel half): `AgentHierarchicalCompression`
    has four more fields (`absorb`, `content_store`, `crush`, `rules`; a
    struct literal adds them, mapped from its `[compaction.hierarchical]`);
    `AgentCoreTool` has three more variants (`ALL` is 11);
    `RESERVED_TOOL_NAMES` has 12 (`absorb`, `acp_retrieve`, `acp_rule`);
    `AgentToolCatalogContext.compression_tools` is a `Vec<AgentCoreTool>`;
    `AgentHistoryStore::write_compression_store`/`load_compression_store`
    (defaults: refuse / none, so the store lives in memory): a host store
    that keeps files implements them to keep the content store across
    restarts, and removes the file with the session; new re-export
    `AgentContentStore`; `AgentService::test_content_store` (test
    support). Goldens unchanged. `NOTICE` already names
    `crates/kernel/src/service/compression` (two new derived files in
    `tools/`).
- Open (CI, seen once at P8b): on a loaded Windows runner
  `plugins::subagents::tests::the_depth_and_running_limits_refuse_a_spawn_with_a_tool_error`
  failed "agent turn cancellation was not confirmed": interrupting a
  running child missed the kernel's 5 s confirmation deadline (the test
  binary took 173 s). The re-run passed.
- [x] P11: `/compression`, status line, `context_compressed`, docs,
      attribution; pty smoke. Done: `AgentEventKind::ContextCompressed
      { call_id, blocks_created, tokens_compressed, tier }` (a `compress`
      that folded: `CompressionToolRun.fold`, carried by `CompressionChange`
      and recorded by `commit_history_with` right after the output's
      `HistoryCommitted`, before `ToolCompleted`; a no-op or refused call
      records none; `tier` is the highest of the blocks made or refolded)
      and `ContextCompressionFailed { message }` (no warning kind existed:
      the smallest one, recorded by `begin_sampling` when the projection
      panicked, once a turn since the fallback then turns the mode off;
      the message bounded at 4 KiB). `AgentSessionRuntimeSnapshot.
      compression: Option<AgentCompressionSummary>` (`active_blocks`,
      `tokens_compressed`; skipped when none, so `standard` snapshots are
      unchanged). `AgentService::compression_status(session, settings)`
      (kernel `service/compression/status.rs`): copies the window, state,
      content store and usage under the lock, then with it released decides
      the nudge again on a copy (`process_turn_with`, no tags, as
      billion-context's `/acp`), `build_status_panel` (no system-prompt
      count, no cache line) and the cache report in full (`acp_cache`'s
      builder, now shared); commits nothing; a panic is
      `AgentError::Internal` (no plugin row, so not a `feature()` call);
      `None` when the session neither runs `hierarchical` nor has a state.
      The facade's `compression_status(session_id)` reads
      `[compaction.hierarchical]` as a turn would. `compress_call_ranges`
      (the tool's lenient parser, panic-contained) for the interface. TUI:
      `/compression [cache]` (`commands/compression.rs`, a `SessionCall`
      run on a thread of its own; the panel then `Stats:` and the content
      store's size; "not using hierarchical compression" without a state
      or a session); the `compression` status item (`3 blocks, 42K saved`,
      from the snapshot, refreshed by the fold's `HistoryCommitted`); the
      `compress` cell folded (header `compress: m00002–m00005 topic`, the
      receipt's first line; the rest of the receipt and the summaries in
      the detailed transcript), live and replayed; no cell for
      `context_compressed` (the tool cell shows the fold); a warning cell,
      kept in `/warnings` (source `panics`), for `context_compression_
      failed`. `exec`'s text output prints a fold and the warning. The
      `/config` row's stale "until they are built" description replaced.
      Docs (both languages): json-events (both kinds, the order, an
      appear), terminal-interface (`/compression`, the status item, the
      folded cell, the warning; new `#compression`), configuration (new
      `#compaction`: the three strategies and the billion-context and
      acp-kernel credit with links), config-reference (`messageTimestamps`
      said to have no effect, no "yet"), architecture and internals (the
      events, the snapshot field, the status; `tools/`), contributing and
      NOTICE (the opt-in tools' texts, `/compression`). Tests: kernel
      `compaction_contract::hierarchical::events` (one event after the
      fold's commit and before `ToolCompleted`, none for a no-op, the
      snapshot summary; the status panel and counters, state and revision
      unchanged by the read, none for `standard`), the panic test asserts
      one `context_compression_failed`; TUI `app::tests::compression` (the
      folded cell live and resumed, `/compression`, `cache`, a bad
      argument, `/statusline compression`, a `standard` session),
      `commands::compression::tests`, the folded cell in
      `history_cell::tool::tests`, the status item's snapshot. Goldens
      unchanged. Module size: `kernel/src/activity.rs`'s ceiling 644 to
      646 (the two new arms of its exhaustive dispatch).
  - Smoke (pty, the real binary, `cargo build -p hanafubuki-agent-cli`):
    a scripted Responses server (Python, in a scratch directory), a home
    set up with `providers add`, `providers default --model`, `config set`
    (`compaction.strategy hierarchical`, the recent zone and the range
    minimum lowered, `titles` off, Scrollback) and `contextWindowTokens =
    100000` added to `providers.toml` by hand (no command sets it). Driven
    in a 160×50 pty: a long answer reported at 80 000 tokens, so the next
    request carried the nudge; the model's `compress` folded `m00002`; the
    folded cell (`Called compress: m00002–m00002 Greeting`, the receipt's
    first line); `/statusline model compression` (`1 block, 9.75K saved`);
    `/compression` (the panel, `Blocks: 1 active / 1 total`, `Stats:`);
    `/compression cache`; `/config` → Compaction to `standard` (saved); a
    turn whose request had no compression tools, prompt or nudge but kept
    the summary; `/exit`, status 0.
  - Decided: `messageTimestamps` stays and acts, the timestamps ported
    (P12).
  - Open: `absorb` sends no `context_compressed` (it folds no block). The
    status panel has no system-prompt row (the Agent does not measure the instructions
    and tools apart) and no cache line (no cached tokens reported). The
    panel's nudge is decided with the settings as they are now, which a
    running turn may not have yet.
  - Hanafubuki, at its next pin: `AgentEventKind` has two more variants,
    `ContextCompressed` and `ContextCompressionFailed` (`context_compressed`,
    `context_compression_failed`: appears; an exhaustive match adds them;
    a host can show a fold's tier and savings, and the failure as a
    warning). `AgentSessionRuntimeSnapshot.compression`
    (`Option<AgentCompressionSummary>`, `#[serde(default)]`, skipped when
    none; its constructions add `compression: None`). New facade call
    `compression_status(session_id)` and function `compress_call_ranges`;
    new re-exports `AgentCompressionStatus`, `AgentCompressionStats`,
    `AgentContentStoreStats`, `AgentCompressionSummary`,
    `AgentCompressRange`; kernel `AgentService::compression_status`.
    Goldens and the persisted format unchanged.
- [x] P12 (after the plan): message timestamps, so
      `[compaction.hierarchical] messageTimestamps` (default true) acts.
      Done: billion-context's #4313 (`createStampTimestampsNode`,
      `tsStamps`, `render-refs.ts`'s `time` attribute, `FUNCTION_ACP_TAGS_
      TIMESTAMPED`, the host's `blockCoverageTimes` span on the fingerprint
      line). Crate: `LocalTime { at, utc_offset_seconds }`, `State.
      ts_stamps` (message id to stamp, never changed or pruned),
      `TurnInput.clock` (`Some` turns the timestamps on: before the refs are
      minted, every id the ref map does not know is stamped with it; an id
      it knows without a stamp is never backfilled), `render_refs(…,
      timestamps)` (` time="yyyy-mm-dd hh:mm:ss"` after `type`, read from
      the stamps only), `format_tag_time` (civil-from-days, no clock),
      `build_compress_system_prompt(prompts, message_timestamps)` (the
      time-aware ACP TAGS section verbatim), `adopt_fork_state` keeps the
      present messages' stamps. Kernel: `AgentHierarchicalCompression.
      message_timestamps` and `utc_offset_seconds` (the facade reads the
      local offset with `chrono::Local` at turn admission); the projection
      passes `now` in that offset (`message_clock`); two cached prompts; a
      fold's receipt shows `[start – end]` per block from the post-fold
      state (none when a covered message or a range end has no stamp);
      `acp_cache` and `/compression`'s cache report use the offset instead
      of 0; the chain's delta carries new stamps in `ts_stamps` (a changed
      or removed stamp takes a snapshot; `rest` never holds them). Tests:
      crate `timestamps` (12, ported from `issue4313-message-timestamps.
      test.ts`: the format at +08:00, first sight once and stable, a later
      arrival later, the offset kept, no backfill, the render reads only
      the stamps and no clock, a fold keeps them, off stamps and shows
      nothing then stamps only new ids, stored stamps hidden while off,
      the prompt variants, a fork's stamps); kernel `compaction_contract::
      hierarchical::timestamps` (the same tag bytes on every request and
      after a restart through the chain; a fixed clock per turn: an
      answer takes the next turn's time and the receipt's span; off: no
      stamp, no attribute, the plain prompt, no span; a fold over a message
      shown before timestamps were on has no span) and the chain tests
      (stamps in deltas, a changed or removed stamp a snapshot). Golden
      `hierarchical_requests` regenerated on purpose with a fixed clock
      (`AgentService::test_fix_message_clock`, test support, per session in
      `CompressionRuntime`): the time attributes, the ACP TAGS section, the
      receipts' spans and the nudge's breakdown counting the longer
      receipt; `standard` goldens unchanged.
  - Decisions: the time is a first-sight stamp (history items record no
    time; billion-context's memory-only lanes do the same), made when a
    request first shows the message, so a turn's final answer takes the
    next turn's time, and messages shown before the setting was on, or
    numbered by a `rebuild_state` repair, show none. The offset is kept
    per stamp, read once per turn: a tag already sent never changes when
    the offset does (daylight saving, travel), and new ones show the new
    local time.
  - Deviations: no message carries a time of its own (the kernel's
    `CoreMessage.ts`), so its case is not ported; the lean pack's TIMELINE
    ("Occurred:") rule is not ported (the Agent sends the default pack,
    which has none). A stamp costs ~70 B per message in a snapshot.
  - Hanafubuki, at its next pin: `AgentHierarchicalCompression` has two
    more fields, `message_timestamps` (from `[compaction.hierarchical]
    messageTimestamps`) and `utc_offset_seconds` (the local offset when
    the turn is admitted); a struct literal adds them. New kernel
    re-export `AgentLocalTime`; `AgentService::test_fix_message_clock`
    (test support). The compression crate: `TurnInput.clock`,
    `render_refs`' and `build_compress_system_prompt`'s new flag,
    `State.ts_stamps`, `LocalTime`, `format_tag_time`. Persisted:
    `AgentCompressionDelta.ts_stamps` and the state's `ts_stamps` (both
    `#[serde(default)]`; older records read as no stamps). Golden to copy:
    `hierarchical_requests.json`.

## Plan: Codex's new terminal interface, its commands, configuration in the interface (2026-11-09)

The plan is [docs/plan-codex-tui-port.md](docs/plan-codex-tui-port.md).
Every §7 decision was accepted as recommended. The source is Codex `57a808a392` (no stable tag has the new
interface). Every phase is validated, documented in both languages,
committed and pushed. Each phase notes what Hanafubuki must change when it
moves its pin.

- [x] P0a (TUI): `[[derived]]` in the notices policy takes `markers`,
      and `crates/tui/src` accepts "Modified from OpenAI Codex 57a808a392
      (Apache-2.0)." beside rust-v0.93.0's; NOTICE names both. The commands
      are one static table (`commands::COMMANDS`, `CommandSpec`: name,
      aliases, description, `inline_args`, `during_task`, `needs_session`,
      `dispatch`); `/quit` is an alias of `/exit`; `/name rest` is split
      and the rest goes to a command that takes it (none yet). A command's
      dispatch and its views (`bottom_pane::module_view`) run under
      `catch_panic`: one that panics is closed, reported, turned off for
      the run and recorded in `ChatWidget.warnings` (for `/warnings`). The
      bridge starts without a session; the first message (or a
      `needs_session` command, through `Op::EnsureSession`) creates it,
      and a failure (no Provider) is an error cell, the interface goes on;
      `/new` returns to no session; `-m` applies to every session the
      interface creates. The terminal interface page, "Slash commands".
  - Limits: the approval and question overlays are chat core, not
    wrapped; a contained panic's report still goes to standard error over
    the interface (as before, todo 5a); a session whose `-m` failed is
    left with no turn.
- [x] P0b (facade): `check-plugin-panics` also covers
      `crates/agent/src/plugins/` (no violation there);
      `hanafubuki_agent::panic::{catch_panic, catch_panic_async}`
      re-exported (the embedding page, "The panic-hook bridge").
  - Moved, as AGENTS.md adds nothing before its first user: the
    feature-call helper (`ModuleHost::service`, `PluginFaults::guard_async`,
    `AgentError::FeatureUnavailable`) comes with the first feature plugin
    (P7's worktree or P8's recap, whichever lands first); the `[tui]`
    section comes with its first keys in P2, and grows with each command.
  - Hanafubuki, at its next pin: nothing to change (two re-exports).
- [x] P1: the provider screen (`/providers`, first-run setup when there is
      no default provider or the model is empty, `test_provider_draft`,
      request headers) and the MCP manager and form (`/mcp`, `/mcp verbose`,
      `/mcp login`, secret rows as credentials, OAuth loopback
      `mcp_oauth_login`, CLI `mcp login`, MCP ids validated on read).
  - [x] The facade's and the CLI's half: `test_provider_draft` (the same
        probe as `list_provider_models`, nothing saved); `mcp_set_credential`
        and `mcp_remove_credential` (atomic under the MCP configuration
        gate, so no revision); `McpCredentialKey::for_server_secret`
        (`<server>-<NAME>`); `mcp_oauth_login` (a loopback on
        `127.0.0.1:0`, 5 minutes, cancellable; derived from Codex's
        `perform_oauth_login.rs`); CLI `mcp login ID`, `mcp add --oauth`,
        `providers add --header NAME=VALUE`; a server id is checked on read
        (a bad one is dropped with a repair) and on write. Fixed on the way:
        a removed credential stayed in `mcp.toml` (the writer merges; removed
        keys are now written as null). Still: a credential entry the
        read-time repair drops stays in the file.
  - [x] The interface's half: `/providers` (list with add, edit, delete,
        enable, default; a form with masked token and header rows; Test on
        the draft; after a save, the model picker writing the model policy)
        and first-run setup (no default Provider, or no model and no `-m`:
        the setup opens and a message waits for it); `/mcp` (manager: add,
        edit, delete, enable, connect, refresh, OAuth login and logout;
        stdio or Streamable HTTP; public or secret rows, a secret stored
        as a credential; Advanced fields), `/mcp verbose`, `/mcp login ID`.
        The form view (`bottom_pane/form/`, `textarea/single_line.rs`).
        Not ported: `SelectionSecondaryAction` (the screens handle their
        keys); no browser is opened for OAuth (the URL is shown).
  - Hanafubuki, at its next pin: re-exports `McpApprovalPolicy`,
    `McpExposurePolicy`, `AgentProviderSettings`.
  - Hanafubuki, at its next pin: new calls `test_provider_draft`,
    `mcp_set_credential`, `mcp_remove_credential`, `mcp_oauth_login`;
    re-exports `McpCredentialKey`, `McpOAuthPhase`, `McpOAuthPolicy`,
    `AgentProviderHeaderSettings`; `mcp_write_server` now refuses an
    invalid id; `mcp_write_credentials` really removes a dropped key. The
    goldens and the corpus are unchanged.
- [x] P2: the new interface (Codex's owned transcript, layers A–G):
      terminal extensions, the `HistoryCell` extension, `TranscriptView`,
      the owned shell, the clipboard, keymap core; owned mode by default;
      Ctrl+T in both modes (live tail); `/tui`; `/raw`.
  - [x] P2.1, layers A–C: `tui/alternate_screen.rs` (enter and leave with
        the keyboard flags, optional mouse capture), mouse events as
        `AppEvent::Mouse`; `HistoryCell` gains `raw_lines` (every cell),
        presentations (compact, expanded, Rich/Raw), activity ids and
        disclosure, `copy_source`, animation ticks; `ChatWidget`'s live
        tail; `transcript_view/` (anchored positions, layout cache, live
        tail, follow-latest, scrolling, mouse and keyboard selection, F3
        search, F4 disclosure, footer). The Ctrl+T pager already shows a
        tool call's whole output. Not ported: hyperlinks and wrap
        provenance, copy mode (needs layer E), bookmarks, images.
  - [x] P2.2: fullscreen (owned) when `tui.fullscreenTranscript` and the
        alternate screen is allowed (`--no-alt-screen`,
        `tui.alternateScreen` auto|always|never, auto off in any SSH
        session: stricter than Codex); cells kept as `Arc`s and drawn by
        the view, `ChatWidget` stops drawing its live cells; older pages
        read on demand (`read_transcript_page`, spliced after the header);
        Ctrl+T toggles detail in place, or in scrollback mode opens
        `pager_overlay::TranscriptOverlay` (live tail, scrollback held
        until close); `/tui`; `/raw [on|off]`; `clipboard.rs` (arboard,
        OSC 52 fallback; `BSL-1.0` accepted in `about.toml` for arboard's
        Windows dependencies); `[tui]` typed (`AgentTuiPreferences`,
        facade `tui_preferences` / `write_tui_preferences`, no revision,
        like `write_settings`).
  - Open from P2: a tool call whose output was on the newer page shows
    without its output (5b); select mode has no default key (`/keymap`,
    P5); `app/mod.rs` is near the file limit.
  - Hanafubuki, at its next pin: `AgentStandaloneSettings` has a new
    public field `tui` (a struct literal needs it); re-exports
    `AgentTuiPreferences`, `AgentTuiPreferencesPatch`,
    `AgentAlternateScreen`.
- [x] P3: Esc to edit an earlier message (`app_backtrack`) over
      `fork_session_before_turn`; facade `edit_from_turn`.
  - Done: `edit_from_turn(session, turn)` forks before the turn and
    archives the original (refused while a turn runs, for an archived,
    child or ephemeral session); `app/backtrack.rs` (prime, browse, edit,
    in place in fullscreen and in the transcript overlay in scrollback);
    `agent_bridge/edit.rs` maps a prompt, counted from the latest, to its
    turn by matching the saved messages (a steering message is refused).
    Terminal interface, "Edit"; sessions, "Edit"; embedding,
    "edit_from_turn".
  - Open: browsing does not start from an open Ctrl+T view; with no
    prompt loaded in fullscreen, the second Esc does not load older
    pages; images are not restored to the composer (it has none).
  - Hanafubuki, at its next pin: the call `edit_from_turn`, the re-export
    `AgentEditFromTurn`.
- [x] P4: `/rename /archive /delete /resume [id|title] /fork /clear /export
      /copy /ps /stop /pwd /init`; `export_session_markdown`; `cwd` and
      `branch` in session summaries.
  - Done: one call family (`AgentCommand::Session`, `agent_bridge/
    sessions.rs`); `/rename` with the suggestion (Tab takes it); `/archive`
    and `/delete` confirmed, then no session; `/resume ID|TITLE`; the
    picker's Branch and Directory columns, Tab for the archived list,
    Ctrl+A to archive or restore; `/fork [name]`; `/clear [name]`;
    `/export [path]` (copy or save; never overwrites) over the facade's
    `export_session_markdown` (Codex's layout, thinking left out);
    `/copy` (the answer, code blocks, quotes; owned mode: select);
    `/ps`; `/stop` and `/clean`; `/pwd` and `/cwd`; `/init` (Codex's
    prompt); `/recap`. Terminal interface, "Session commands"; sessions,
    "Export".
  - Open: `/rename`, `/archive`, `/resume` are off during a turn (Codex
    allows them); `/resume TITLE` and the picker's archive run on the
    interface thread; the export is written in place (no temp file);
    the picker has no Ctrl+T preview.
  - Hanafubuki, at its next pin: the call `export_session_markdown`.
- [x] P5: `/theme` (syntect and two-face, also syntax highlighting),
      `/keymap`, `/title`, `/statusline`, `/presets`, `/config`,
      `/warnings`, `/memories`.
  - [x] P5a: highlighting of fenced code and diffs (syntect 5.3, two-face
        0.5 with Oniguruma; the grammars' and themes' licences in
        `crates/tui/syntax-licenses.txt`, appended to the release's
        `THIRD-PARTY-LICENSES.txt`); `/theme` (bundled and
        `<data>/themes/*.tmTheme`, live preview; default `ansi`);
        `/statusline` (model, reasoning, directory, branch, context,
        tokens, session id and title, task progress, permission mode,
        goal) and `/title` (OSC 0, sanitized, the title stack pushed and
        popped); `tui.theme`, `tui.statusLine`, `tui.statusLineColors`,
        `tui.terminalTitle`. Not ported: Codex's model-named themes, a
        background query for the default theme, a streaming highlighter.
  - [x] P5b: the keymap (`crates/tui/src/keymap/`: contexts global,
        chat, composer, editor, transcript, pager; context → global →
        defaults; chords; leader Ctrl-X; conflicts refused; `tui.keymap`)
        and `/keymap [debug]`; `/presets` (`tui.presets` slots 0–9, run
        through the slash parser, "from current settings" with
        `/permissions`, `/title`, `/statusline`); `/config`; `/warnings`
        (the configuration's layer warnings and recoveries, plugins, MCP,
        hooks, panics, the interface's own; "N warnings: /warnings" at
        start, which closes the D2 item on lost first-read warnings);
        `/memories`. The `TextArea::input` module-size exception is gone.
  - Open (P5b): popup, list, form, select-mode, search and browsing keys
    stay fixed as in Codex; `config set tui.keymap.<context>` is refused
    (only the whole table); a slot a profile or `-c` overrides is
    reported after the save, not refused.
  - Hanafubuki, at its next pin: `AgentTuiPreferences` has `keymap` and
    `presets` (struct literals); new `AgentTuiPreset`,
    `AgentConfig::layer_warnings`; facade `config_recoveries`,
    `config_layer_warnings`; re-exports `AgentMemorySettings`,
    `AgentMemoryStatus`, `AgentMemoryContributionMode`.
- [x] P6: per-session permission mode (`set_session_permission_mode`,
      `turns.rs` no longer overrides it), `/permissions`, `/approve`
      (`approve_denied_action`, a model-visible note: goldens).
  - [x] The facade's half: `set_session_permission_mode` and
        `session_permission_mode` (runtime state beside the allowances; not
        persisted, forks and recoveries start with none; a running turn
        keeps the mode it started with); `approve_denied_action(session,
        review_id)` for one of the session's 10 latest denied reviews,
        between turns only: a `ContinuationContext` item (provider `kernel`,
        hook `approved-denied-action`) with Codex's approved-action wording,
        which the model and the next auto-review see (`user approval`
        evidence). No golden or corpus change; the note's text is pinned in
        `kernel/tests/loop_contract/approved_action.rs`. Permissions,
        "Session mode" and "Approving a denied action".
  - [x] The interface's half: `/permissions [mode]` (full access asks
        first; the session only; `/status` and the status line show the
        mode in effect); `/approve` (the session's latest ten denied
        reviews, searchable; the retry is still reviewed).
  - Hanafubuki, at its next pin: the re-export `PermissionReviewStatus`;
    new calls `set_session_permission_mode`,
    `session_permission_mode`, `approve_denied_action`; kernel variant
    `PermissionReviewEvidenceRole::UserApproval`; the review prompt gained
    one sentence (no golden covers review requests). Differences from
    Codex: refused during a turn; a compaction drops older denials.
- [x] P7: a mutable standalone workspace, `WorkspaceFsPolicy` rebuilt,
      `exec_command`'s default workdir (the `-C` bug), `/cd`;
      `hanafubuki-agent-worktree` and `plugin-worktree`, `/worktree`, the
      worktree choice in `/fork`.
  - [x] The facade's half: `AgentRuntime::set_working_directory` (standalone
        only; refused while a turn, a compaction or a background terminal
        runs; the filesystem policy is replaced in place and the commands'
        directory moves; live sessions stay); `exec_command` without
        `workdir` runs in the first workspace root, a relative `workdir`
        against it (the `-C` bug); no context note is needed (the
        `workspace_roots` section is rebuilt every turn).
        `hanafubuki-agent-worktree` (`<data>/worktrees/<repo>-<8 hex>`,
        git through the instance's `ExecutorManager`, the owner in the
        worktree's git metadata) and the `worktree` row; facade calls
        `worktree_create`, `worktree_list`, `worktree_bind`,
        `worktree_remove` (feature calls).
  - [x] The interface's half: `/cd [path]` (says the project
        configuration stays the one read at start); `/worktree` (continue
        this conversation, start a new one, browse: resume, copy, remove);
        `/fork` asks "here" or "into a new worktree" in a Git repository.
        `hanafubuki_agent_tui::run` takes `&Arc<AgentRuntime>`.
  - Open: the project configuration layer (trust level, the project's
    `config.toml`) does not follow `/cd`; it stays the project read at
    start (switching it needs a new instance). The worktree service type
    lives in the plugin crate, not in contracts (plugin-rules says so).
  - Hanafubuki, at its next pin: `exec_command` without `workdir` now runs
    in the first workspace root as read at the instance's start (it ran in
    the process's directory); new crates and a `worktree` row (Capability);
    `AgentStoragePaths::worktrees_root`; `AgentExecutionService::
    set_working_directory`; `WorkspaceFsService::replace_policy`.
- [x] P8: the shared structured request (from the titles plugin), the
      `recap` plugin, `/recap`, `suggest_session_title` in `/rename`.
  - [x] The facade's half: `plugins/structured_request.rs` (the titles
        plugin uses it, unchanged); the feature-call helper
        (`service/features.rs`: resolve the row's service per call, run it
        under `PluginFaults::guard_async`, `AgentError::FeatureUnavailable`
        when the row is down or the call panicked); the `recap` row
        (`RecapService`, the last 8 exchanges within 8192 estimated tokens,
        Codex's prompt) and the `titles` row's `TitleService`; the narrow
        port `AgentConversationReader`. Embedding, "Feature calls".
  - [x] The interface's half: `/recap`, the suggestion in `/rename` (P4).
  - Hanafubuki, at its next pin: `AgentError` is now exported and has
    `FeatureUnavailable { feature, reason }` (an exhaustive match needs an
    arm); new calls `recap_session`, `suggest_session_title`, type
    `AgentRecap`; a new plugin row `recap` (Capability, after `titles`;
    host rows now mount after it). Goldens and the history corpus are
    unchanged.
- [x] P9: child and ephemeral sessions (`parent`, `origin`,
      `AgentSessionDriver`, archive and delete cascade; a new history
      fixture in both repositories), `/side` and `/btw`, the workspace
      panes.
  - [x] The facade's half: lineage (`parent_session_id`, `origin` Side |
        Subagent | Review, `cwd`, `branch` from `.git/HEAD`) in the
        persisted runtime state (optional fields, format version unchanged)
        and in `AgentSessionSummary`; `AgentSessionListRequest.parent`
        (`TopLevel` by default, so every list now hides children, or
        `ChildrenOf(id)`), cursor version 2, index schema 5 (an older
        index is rebuilt from the rollouts once); ephemeral sessions in an
        in-memory store behind `SessionStores` (never listed or
        recovered); kernel `create_child_session`,
        `close_ephemeral_session`, `ephemeral_children`;
        `AgentSessionDriver` (`session_driver.rs`; `run_turn` and
        `last_answer` wait for P10); `open_side_conversation` and
        `close_side_conversation` (Codex's side policy and boundary as
        `ContinuationContext` items); archive and delete cascade over
        persisted descendants (a refused deletion deletes nothing). New
        corpus `golden/history-lineage/` with
        `recorded_lineage_history_transcript.json`, and the request golden
        `side_conversation_request.json`.
  - [x] The interface's half: `/side [message|close]` and `/btw` (a
        second conversation: docked on the right in fullscreen at 100
        columns or more, with a divider, zoom and the leader's pane keys;
        a switch with a banner in scrollback; Codex's command scope; Esc
        returns); the resume picker's cwd and branch came in P4. No tab
        strip (one side conversation per session).
  - Open: a compacting parent refuses `/side`; no `/side` before a first
    turn; restore does not cascade; a child does not inherit the
    session's permission mode (P10 passes it).
  - Hanafubuki, at its next pin: copy `history-lineage/`,
    `recorded_lineage_history_transcript.json` and
    `side_conversation_request.json` byte for byte, with the twin of
    `recorded_lineage_history_reopens_and_continues`; every session list
    without `parent` now hides children; the index reindexes once (schema
    5); kernel signatures changed: `create_model_session` and
    `create_unconfigured_session` take an `AgentSessionWorkspace`,
    `prepare_session_deletion` takes `descendants`,
    `PreparedAgentSessionDeletion` holds a list and is not `Copy`; new
    facade calls `open_side_conversation`, `close_side_conversation`;
    re-exports `AgentSessionOrigin`, `AgentSessionParentFilter`.
- [x] P10: the `subagents` plugin (Codex's v2 tools, orchestrator prompt:
      goldens), `/subagents`, `/agents`.
  - [x] The facade's half: the `subagents` row (after `goal`): tools
        `spawn_agent`, `send_message`, `followup_task`, `wait_agent`,
        `list_agents`, `interrupt_agent` over `AgentSessionDriver` (Plan
        mode: `wait_agent`, `list_agents`); children are durable sessions
        of origin `Subagent` with the parent's effective permission mode;
        depth 1, at most 3 running per parent, waits up to 5 minutes; the
        `subagent_guidance` prompt fragment (the tail of Codex's
        orchestrator.md) for top-level Default-mode sessions; facade
        `subagents(parent)`, `interrupt_subagent`. Ten request goldens
        regenerated (Default 30 tools, Plan 15); corpora unchanged.
  - [x] The interface's half: `/subagents` (status cell, a live picker,
        showing a child with its journal followed and its turn adopted);
        a child's pending approval shown in the parent ("Subagent NAME:")
        and answered on the child; `/agents` (sessions with their children
        nested, filters, open, new, fork, archive or restore, delete,
        interrupt).
  - Fixed after P14: an interrupt that lands while the tool catalog is
    built ends the child's turn Cancelled (it was Failed).
  - Open: `wait_agent` pollutes memory
    contribution, so a coordinating session never contributes; archiving
    a parent while a child runs is untested.
  - Hanafubuki, at its next pin: re-exports `AgentSubagentStatus`,
    `AgentSubagentState`; calls `subagents`, `interrupt_subagent`; a
    `subagents` row; every request golden changes again.
- [x] P11: the `review` plugin (rubric, read-only child session, JSON
      findings), `/review`.
  - [x] The facade's half: the `review` row (after `subagents`); a review
        is a durable child of origin `Review` (empty, Default mode) whose
        first message is Codex's request for the target (uncommitted
        changes, base branch with its merge base, commit, custom); the
        rubric is a prompt fragment for review sessions only; the
        `review.read-only` approval rule denies every action that needs
        approval in a review session except `git --no-pager
        status|diff|show|log|blame|rev-parse|merge-base|ls-files` with
        plain arguments (contracts: `ApprovalRule::evaluate` takes the
        session's origin; `AgentSessionOrigin` moved to contracts); the
        review's turn runs in FullAccess; the answer is read tolerantly;
        the parent gets Codex's exit message as context. Facade
        `start_review`, `cancel_review`, `review_targets` (git queries in
        `crates/worktree/src/queries.rs`); `AgentReviewTarget` is a strict
        IPC request. New golden `review_request.json`; no other changed.
  - [x] The interface's half: `/review [instructions|cancel]` (a picker
        of targets; refused before a first message; a "Reviewing" status
        and Esc to cancel; the review cell with the verdict and findings;
        messages sent meanwhile wait for the findings).
  - Open: read-only tools, allowed MCP tools and the Agent's own tools
    still run in a review; `request_user_input` could stall one; no
    `review_model`; a failed review is reported as interrupted; with the
    plugin off, an earlier review session is no longer kept read-only.
  - Hanafubuki, at its next pin: every `ApprovalRule` implementor takes
    `origin`, and so does `ApprovalRuleSnapshot::verdict`; new calls
    `start_review`, `cancel_review`, `review_targets` and their types; a
    `review` row; kernel `add_continuation_context`; copy
    `review_request.json` byte for byte.
- [x] P12: the `goal` plugin (tools, context, the first continuation hook,
      goal templates), `/goal` and its status-line item.
  - [x] The facade's half: `AgentGoal` (objective ≤ 4,000 characters,
        status active | paused | blocked | budget_limited | complete,
        origin, token and time budgets, usage) as an optional field `goal`
        of the persisted runtime state (a fork copies it; children and
        ephemeral sessions never have one); kernel `session_goal`,
        `update_session_goal` (compare-and-swap); the `goal` row: tools
        `get_goal`, `create_goal`, `update_goal` (Plan mode: `get_goal`
        only), a `goal` world-state section, and the continuation hook
        `goal/continue` (Codex's continuation, budget-limit and
        objective-updated templates, within the same turn); facade `goal`,
        `set_goal`, `clear_goal`, `pause_goal`, `resume_goal`; event
        `session_goal_updated`. Ten request goldens regenerated (the three
        tools); new corpus `golden/history-goal/` with
        `recorded_goal_history_transcript.json`. `activity.rs`'s ceiling
        644. Plugin rules' tool counts corrected (they were already off:
        Default 24, Plan 13).
  - [x] The interface's half: `/goal [objective [--tokens N]|clear|pause|
        resume|edit]` (a menu with usage and budgets; "Replace goal?"),
        the status line's goal item; setting or resuming starts no turn.
  - Open: continuation is within the turn (Codex starts a new turn), so a
    goal without a budget is bounded by a turn's limits; `resume_goal`
    starts no turn; `/goal edit` (keeping usage) needs an edit path;
    tokens count cached input.
  - Hanafubuki, at its next pin: copy `history-goal/`,
    `recorded_goal_history_transcript.json` and the ten regenerated
    request goldens byte for byte, with the twin of
    `recorded_goal_history_reopens_and_continues`; `PersistedAgentRuntime
    State` struct literals need `goal`; exhaustive event matches need
    `SessionGoalUpdated`; a `goal` row after `recap`.
- [x] P13: `hanafubuki-agent-hooks` and `plugin-hooks`, the
      `ToolLifecycleHookRegistry` extension point, hook events, `/hooks`.
  - [x] The facade's half: `crates/hooks` (Codex's `[hooks]` and
        `hooks.json` from the home and a trusted project; tolerant;
        matchers, with `Bash`/`Write`/`Edit` aliases; command handlers
        through the executor; trust by sha256 of the definition and
        enablement in `<data>/hooks-state.json`); `crates/plugin-hooks`;
        `ToolLifecycleHookRegistry` in contracts (deny-only before a call,
        context after the step's outputs, fail-open); Stop as the first
        continuation hook; SessionStart and UserPromptSubmit as a
        `hook_context` section; `AgentEventKind::Hook{Started,Completed}`;
        facade `hooks_list`, `set_hook_enabled`, `trust_hooks`. `[hooks]`
        is no longer reported as an unknown configuration key. No golden
        changed; `kernel/src/activity.rs`'s ceiling rose from 641 to 643
        (its exhaustive event match).
  - [x] The interface's half: `/hooks` (events, hooks with trust and
        on/off, toggle, trust, trust all; the configuration's skipped
        items); at start, "N hooks need review" and a warning.
  - Open: dropped events (model request, permission request, compaction,
    session end, subagent start/stop, interrupt) and handlers (`mcp_tool`,
    `prompt`, `agent`, async) are diagnostics; a project with only
    `hooks.json` is not seen until the configuration exposes the project
    root; hooks of one event run in turn; UserPromptSubmit cannot block;
    `trust_hooks` hashes the definition on disk at call time.
  - Hanafubuki, at its next pin: `ContextRequest` has a new field
    `user_message` (a struct literal needs it); new events `hook_started`
    and `hook_completed`; `AgentContinuationHook::applies` (default true);
    new crates and a `hooks` row; facade `hooks_list`, `set_hook_enabled`,
    `trust_hooks`; `AgentStoragePaths.hooks_state_file`.
- [ ] P14 (decision 4): `hanafubuki-agent-sandbox`, sandbox presets in
      `/permissions`, `/setup-default-sandbox` on Windows.
  - [x] P14a, Linux: `crates/sandbox` (`SandboxPolicy` read-only |
        workspace-write | full-access); the model's commands run through
        the binary as a helper (`--hanafubuki-agent-sandbox`: no_new_privs,
        Landlock writes only to the writable roots, `/dev/null`,
        `/dev/tty`; seccomp blocks the network unless allowed), Codex's
        Landlock + seccomp path rather than bubblewrap; fail closed (a
        mode the machine cannot enforce refuses the command, before
        approval). `sandboxMode` (unset: workspace-write on Linux,
        full-access elsewhere until P14b/c), `[sandboxWorkspaceWrite]`
        `writableRoots`, `networkAccess` (default false); per-session
        `set_session_sandbox_mode` (a subagent inherits it); `doctor`
        shows the sandbox. `crates/process` unchanged.
  - [x] P14b, macOS: Seatbelt through `/usr/bin/sandbox-exec -p <policy>
        -DWRITABLE_ROOT_N=…` (Codex's three `.sbpl` files, unchanged but
        for a header and `/dev/tty`); writes only under the writable roots,
        an existing `.git`, `.agents` and `.hanafubuki-agent` read-only;
        network only when allowed; available when `sandbox-exec` exists,
        else fail closed; workspace-write is now the macOS default.
        Verified on Linux (policy and command-line tests) and by a macOS
        target check of the sandbox crate only; `tests/macos.rs` first
        runs on the macOS CI job.
  - Open (P14b): on macOS in workspace-write, `git commit` and anything
    writing `.git` fail (as in Codex) until P14d's escalation.
  - [x] P14c, Windows, unelevated: Codex's restricted-token sandbox
        (`crates/sandbox/src/windows/`) through `helper_dispatch`: a
        write-restricted token whose restricting SIDs are one capability
        SID per writable root (SHA-256 of its `PathKey`, no state file),
        write ACEs on the roots, deny-write ACEs on an existing `.git`,
        `.agents`, `.hanafubuki-agent`; a private desktop; a kill-on-close
        job; TEMP a writable root on every platform. Opt-in by
        `[windows] sandbox = "unelevated"`; unset or `"elevated"` refuses
        sandboxed modes; the Windows default stays full-access.
        `/setup-default-sandbox` (Windows only) explains. Compiled and
        linted for Windows only; `tests/windows.rs` and the CLI test first
        run on the Windows CI job.
  - [ ] P14e, Windows, elevated: sandbox users from a UAC setup, network
        blocked by firewall rules, the command runner, readiness; a real
        `/setup-default-sandbox` writing `windows.sandbox = "elevated"`;
        then the Windows default. Left open by decision (2026-11-09):
        port it only once asked.
  - Open (P14c): unelevated does not block the network (Codex's offline
    environment variables only; the `sandbox` section then adds
    `"networkEnforced": false`); a background process ends with its
    command. `sandbox_status()` now applies `windows.sandbox` and returns
    a `Result` (Hanafubuki: the signature changed).
  - Hanafubuki, at its next pin (P14c): `AgentSettings::windows`
    (`AgentWindowsSandbox`); a settings table whose values are all unset
    is no longer written.
  - [x] P14d: `exec_command`'s `sandbox_permissions` (`use_default` |
        `require_escalated`) and `justification`; an escalated command is
        approved like any action (Manual asks, AutoReview's reviewer sees
        `sandboxEscalation`, FullAccess refuses it, as Codex's `never`),
        its digest differs from the sandboxed one, and an approved one
        runs unwrapped; a session allowance for it covers that command
        only (the overlay says so). A `sandbox` world-state section tells
        the model the mode, writable roots, network and how to escalate.
        `/permissions` presets: Read only, Default, Auto-review, Full
        access (both modes for the session). Eleven request goldens
        regenerated; corpora unchanged.
  - Open (P14d): `/permissions manual|auto-review|full-access` set the
    mode only (the preset names collide); the status line does not show
    the sandbox; Codex's `prefix_rule` and `with_additional_permissions`
    are not ported.
  - Hanafubuki, at its next pin (P14d): `ExecCommandRequest.
    sandbox_escalation`, `ApprovalAction.sandbox_escalation` and
    `SandboxEscalation` (re-exported); `ContextSectionKind::SANDBOX` (six
    known kinds), `MAXIMUM_AGENT_WORLD_STATE_SECTIONS` 12; approval
    payloads may carry `sandboxEscalation` (show the justification); copy
    the eleven request goldens.
  - Open: on Linux the network is off by default for the model's
    commands (P14d's escalation lets one command out, approved); test support's
    `configure_test_provider` also writes `sandboxMode = "full-access"`
    (test binaries cannot be the helper); `.git` under a writable root
    stays writable (Landlock cannot carve sub-paths); review sessions are
    not backed by a read-only sandbox yet.
  - Hanafubuki, at its next pin: call `hanafubuki_agent::sandbox::
    helper_dispatch()` first in `main` (else every command fails on Linux
    by default, or set `sandboxMode = "full-access"`); `AgentSettings`
    has `sandbox_mode` and `sandbox_workspace_write`;
    `ExecCommandRequest.sandbox`; `set_working_directory` is now
    `set_workspace_roots`; a new crate.
- [x] CI on macOS and Windows (red since P2.1, fixed after P14): platform
      assumptions in tests (line endings, absolute paths, the alternate
      screen, the Windows SID, PowerShell's wording); a narrow description
      column stacks; a stop that overtakes a background terminal's wait
      finds it gone; a confirmed cancellation whose commit outlasts the
      grace is no longer aborted (its sink heard nothing); a subagent
      shown after it asked for an approval shows it. Tests wait on
      outcomes (`TEST_WAIT`, 60 s) and give shutdowns that are not their
      subject `TEST_SHUTDOWN_DEADLINE` (10 s); tests whose time limit is
      the point measure it relative to a far larger one. Reproduced and
      cleared under CPU and disk load on Linux.
  - Open: `workspace_commands::a_denied_action_is_approved_between_turns_
    and_refused_during_one` uses a 10 s model delay as its window and is
    `#[ignore]`d until it holds the turn open itself (a TODO at the test;
    `cargo test -p hanafubuki-agent-tui -- --ignored` runs it);
    a turn thread blocked on the record's lock at the moment the abort is
    decided can still be aborted while staging its end (a narrow window,
    left as is); `mcp_oauth`'s refused-port wait rests on an unproven
    cause (another test's child holding the listener until it execs).

## Plan: visible thinking and the documentation site (2026-11-09)

The plan was `docs/plan-thinking-and-site.md` (removed once its
repository work was done; git keeps it); every recommendation in its §5 was accepted (store thinking in history; drop
Hanafubuki's separate thinking timer; a bilingual site, zh-CN by default;
`extraction-plan.md` not carried over; GitHub Pages; one new history-corpus
fixture recorded deliberately; Giscus follows the site's theme and locale).
The next release with it is 0.2.0 (the headline event is removed).

- [x] T1: thinking in the model API (three protocols, `reasoning.summary`)
      and the kernel (three events, `ConversationItem::Reasoning`, its own
      budget), the headline removed (the site's architecture page,
      "Thinking", and its exec --json events page, "Thinking"). Each item is cut
      at `AgentHardLimits::maximum_reasoning_text_bytes` (256 KiB) and no
      longer spends `maximum_model_output_bytes`; a response that stops
      early completes its open items. No request golden sets an effort, so
      none changed; the new corpus is `golden/history-reasoning/` with
      `recorded_reasoning_history_transcript.json`, and `golden/history/` is
      unchanged.
  - What T2 and T3 build on (serialized as the facade serializes them):
    - `AgentEventKind::ReasoningStarted { reasoning_id, sampling_step }` →
      `{"type":"reasoning_started","reasoningId":3,"samplingStep":1}`;
      `ReasoningDelta { reasoning_id, delta }` →
      `{"type":"reasoning_delta","reasoningId":3,"delta":"…"}` (Markdown,
      concatenated as is: a new summary part's delta starts with its blank
      line); `ReasoningCompleted { reasoning_id, duration_millis }` →
      `{"type":"reasoning_completed","reasoningId":3,"durationMillis":1200}`.
      Started → deltas → completed, all before the step's
      `history_committed`; an item may have no delta (a provider that sent
      no text). `AgentReasoningId` (a `u64`, unique in the session, drawn
      from the message counter) is re-exported by the facade. A turn that
      fails mid-stream sends no completion: end live blocks at the turn's
      terminal event.
    - `ConversationItem::Reasoning { text, duration_millis, sampling_step,
      truncated }` →
      `{"type":"reasoning","text":"…","durationMillis":1200,"samplingStep":1,"truncated":false}`,
      committed with the step's other items in the model's order (normally
      ahead of the step's tool calls and answer). It carries no reasoning id: a live
      block is replaced by the items of the `history_committed` that follows
      its completion. Exhaustive matches on `ConversationItem` need an arm.
    - Removed: `AgentEventKind::ReasoningHeadlineUpdated`
      (`reasoning_headline_updated`) and `AgentTurnSnapshot::reasoning_headline`
      (`reasoningHeadline`). `ReasoningUpdated` and `AgentReasoningSnapshot`
      (the turn's thinking time) are unchanged.
    - Model API: `ModelEvent::ReasoningSummaryDelta` is now
      `ReasoningDelta { item_id, kind: ModelReasoningTextKind, index, delta }`
      and `ModelOutputItem::Reasoning` has `content: Vec<String>` beside
      `summary`; a Responses request with an effort sends
      `"reasoning":{"effort":…,"summary":"auto"}`, which changes a host's
      request goldens only where they set an effort.
    - `AgentHardLimits` has `maximum_reasoning_text_bytes`: a struct
      literal without `..FIRST_BATCH_HARD_LIMITS` needs it.
      `history_committed`'s `itemCount` counts the step's reasoning items.
    - Test support: `reasoning_tool_events` and `reasoning_answer_events`
      (Responses streams whose reasoning summary streams).
  - Open: reasoning items count toward the history bounds
    (`maximum_history_bytes`, 4 MiB, and `maximum_history_items`, 1024, one
    more item per thinking step) but not toward the token estimate that
    triggers compaction. Compaction drops them, so a session that compacts is
    unaffected; a long session that thinks a lot and uses little context
    can now reach `LimitExceeded { "history bytes" }` sooner. Decide whether
    compaction also triggers on the history bounds, or whether a reasoning
    item's bytes leave `retained_bytes`.
- [x] T2: the TUI thinking cell (`history_cell/thinking.rs`), Ctrl-O,
      Working without the headline; `exec`'s `· thought for 12s` on
      standard error and `sessions show`'s `thought for 12s` line
      (the site's terminal interface page, "Thinking", and architecture
      page, "The terminal interface and the command line").
  - What T3 mirrors: collapsed by default; live `Thinking 5s` counting up
    from the cell's own start, completed `Thought for 12s` from the
    kernel's `durationMillis` (seconds, rounded down; `(truncated)` when
    the item says so); an empty text shows the header only; the turn's
    terminal event completes a live block with the time so far; one
    toggle for the live block and every later one; a reasoning update
    that arrives while a message streams is held until that message
    completes, so the transcript keeps the model's order. The live text
    is the streamed text; for an item that streamed both a summary and
    raw text the committed item (the summary) is what a reload shows.
  - Open (found in T2, not changed): `sessions show` prints its items
    latest first, in the page's order, where a person reads a transcript
    oldest first.
- [ ] T3 (Hanafubuki): the thinking block, Working without the headline or
      the thinking timer, the Task Center showing the configuration's
      recoveries.
  - Copy `crates/agent/src/tests/golden/history-reasoning/` (one session,
    `active/2026/11/09/session-2026-11-09T04-11-30-1.jsonl`) and
    `recorded_reasoning_history_transcript.json` byte for byte into the
    integration's `src/tests/golden/`, with the twin of
    `recorded_reasoning_history_reopens_and_continues` (`golden.rs`): it
    opens with both reasoning items' text and the next request carries
    none of it. The recorded prompt is "List the Skills." and its tool
    `skills_list`.
- [x] D1: the `site/` framework, tooling, Giscus, Umami, `site.yml`.
      Hanafubuki's site at its versions, without its home and download
      pages; a small home page; Giscus starts from the given attributes
      and follows the site's theme and language once its frame has loaded
      (its first message); `cargo xtask version` also scans
      `site/content/docs/**/*.mdx`. Under the Pages base, TanStack Start's
      static server functions fetched from the origin root, which broke
      client-side navigation: `patches/` prefixes Vite's base
      (AGENTS.md, "Site"). The router links with a trailing slash, so
      giscus's `pathname` term is the same for a page reached by URL or by
      in-site navigation.
- [x] D2: the content rewrite, bilingual; `docs/` removed; links re-pointed.
      Thirty-two pages in zh-CN and English, every heading with an
      explicit id shared by both languages; the Development section's
      `internals` page keeps the as-built detail of the old spec. AGENTS.md
      cites the pages' English sources, the README the live site, code
      comments and NOTICE `site/content/docs/<slug>.en.mdx`; `cargo xtask
      version` scans the README and the site only. A link to a heading on
      another page passes its fragment to the router apart, so the
      trailing slash goes before the `#`. Only the active plan is left in
      `docs/`.
  - Found while writing the docs (documented as current behaviour, likely
    bugs):
    - `-C` does not change the process's working directory, so
      `exec_command` without `workdir` runs where the binary started
      (`crates/execution/src/shell.rs:68`, `Path::new(".")`).
    - `mcp.toml` server ids are not validated on read.
    - `.git/` may show in `@` mention results.
    - Configuration warnings from the first read are lost for commands
      that do not start the Agent (the interface keeps them since P5b).
    - Every start of the terminal interface creates a session that
      `sessions list` shows (see Phase 5b below).
    - `skills/src/service/mutations.rs`: `source_folder_name`'s comment
      prefers the frontmatter name, the code the source folder's name.
    - Provider requests ignore `[network] proxy`: they read the process's
      proxy variables. (Done 2026-11-12: they take it; see "Model Providers
      through the proxy".)
    - An unusable `config.toml` rewritten with defaults drops profiles,
      projects, `logLevel` and `[network]` from the live file (they are
      kept only in `.recovery/`).
    - The golden update variables trigger on any value
      (`var_os(..).is_some()`), `0` and the empty string included.
    - `check-plugin-panics` covers only `crates/plugin-*` (fixed in P0b).
    - `sessions show` lists its items latest first (see T2 above).
- [ ] D2b (Hanafubuki): its links into this repository's docs re-pointed.
- [x] D3: GitHub Pages. Pages is enabled (source: GitHub Actions) and
      `site.yml`'s `deploy` job publishes every push to main
      (`actions/upload-pages-artifact` 5.0.0, `actions/deploy-pages`
      5.0.1); the site is https://licyk.github.io/hanafubuki-agent/. On
      the live site a page reached by URL and by in-site navigation gives
      the same Giscus term (`hanafubuki-agent/docs/<slug>/`, one thread),
      and every page carries the Umami script with its website id.
- [x] 0.2.0 released (tag `v0.2.0` at `feba8a1`, release run
      42576971888): all 24 crates on crates.io, the four archives and their
      sums on the GitHub release.
- [ ] Yours: the visual checks of the thinking display in Hanafubuki and
      in the terminal interface on macOS and Windows terminals.

## Extraction from Hanafubuki

The phases were in the extraction plan (§14), finished and removed in D2
(git keeps it); Hanafubuki's `todo.md` tracks them with their done notes. Phases 1–3 are
done: the workspace, the decoupling inside Hanafubuki's tree, and the move.

- [x] Phase 4a: what the facade needs to run without a host: config
      layering (`AgentConfig::layered`, `ConfigLayers`, `-c` from Codex,
      `[network]`, `logLevel`, `AgentDirs`/`HANAFUBUKI_AGENT_HOME`), the
      data-dir lock (both modes), the standalone log file, the keyring
      store, the cwd workspace and `AgentRuntime::standalone`
      (the site's Configuration pages; architecture page, "The terminal
      interface and the command line").
- [x] Phase 4b: the CLI (`hanafubuki-agent-cli`, binary `hanafubuki-agent`,
      `clap`): the global flags, `exec` (text and JSON lines, `--approve`,
      questions on a terminal, exit codes, `-o`), `config path|get|set|edit`
      (`AgentConfig::value`, `set_user_value`, `layer_sources`), `providers`,
      `mcp`, `sessions`, `skills`, `memory`, `doctor`; the proxy in the
      Agent's processes' environment; the host-supplied identity
      (`AgentHostInstructions::identity`, `application`)
      (the site's command line reference page). Phase 4 is done:
      an `exec` turn against the scripted model runs end to end in CI
      (`crates/cli/tests/exec.rs`).
  - Deferred: `mcp add` sets no credentials (`credentialEnvironment`,
    `credentialHeaders`, OAuth login are edited in `mcp.toml`); `providers
    add` takes no request headers; `exec` resumes no session and attaches
    no images; `doctor` cannot say which process holds the data directory
    (the lock file names none); a management command that starts the Agent
    also starts its MCP servers in the background, as every start does;
    Ctrl-C during a command other than `exec` ends it without a shutdown
    (the lock goes with the process, the next start reaps its processes).
  - AGENTS.md discovery reads the working directory's and the Git root's
    own `AGENTS.md` (each a root), not the directories between them, since
    the runtime selects no target; Codex reads every level.
- [ ] Hanafubuki, when it moves its pin past Phase 4a: nothing to change for
      the lock (it is taken by the start marker); `AgentProxy` is now
      `hanafubuki_agent_config::AgentProxy`, re-exported where it was.
- [ ] Hanafubuki, when it moves its pin past Phase 4b:
  - `AgentHostInstructions` has two new fields (its struct literal in
    `hanafubuki-agent-integration/src/host.rs`, `application_instructions`,
    stops compiling): pass `identity: "You are the desktop application's
    coding and operations agent.".to_string()` and `application:
    "Hanafubuki".to_string()`, the strings the Agent used to have, and the
    integration goldens stay byte-identical (checked here by building the
    facade's goldens with those values); the history corpus is unchanged.
  - Hanafubuki's `docs/agent-integration.md`: the host instructions now
    carry the identity and the application name.
  - Nothing else: the facade's other changes are additions (re-exports of
    MCP, approval and user-input types, `probe_keyring`, the
    configuration's key API), and the CLI is a crate Hanafubuki does not
    depend on.
- [x] Phase 5a: the terminal interface's core (`hanafubuki-agent-tui`;
      the site's terminal interface and architecture pages): upstream ratatui
      0.30 and crossterm 0.29 (Codex's git forks dropped, the palette
      query too), the inline viewport and scrollback, the app loop, the
      chat widget and its history cells, markdown and streaming, the diff
      of a `PatchPreview`, the composer, the status line and key hints, the
      approval and question overlays, `agent_bridge.rs`; bare
      `hanafubuki-agent [PROMPT]` runs it on a terminal. NOTICE carries
      Codex's NOTICE and license and Ratatui's.
- [x] Phase 5b: the rest of the terminal interface (the site's terminal
      interface page, "Slash commands"): the slash commands with
      the composer's popup (`/model` from the Providers' model catalogs
      and `set_session_model`, `/mode`, `/skills`, `/new`, `/resume`,
      `/compact`, `/diff`, `/mention`, `/status`, `/mcp`, `/quit`,
      `/exit`), the transcript pager (Ctrl-T) and `/diff`'s on the
      alternate screen, the resume picker over the history store's session
      pages with the transcript replayed, `hanafubuki-agent resume
      [ID|--last] [PROMPT]`, `@` file mentions from Codex's `file-search`
      (in the TUI crate, on `nucleo` 0.5), and `cargo xtask check-notices`
      (`architecture/notices-policy.toml`). The facade gained
      `workspace_roots()` and re-exports `AgentCompactionStatus`. Phase 5
      is done: a pty run covers the popup,
      `/status`, a mention, a turn, Ctrl-T, `/diff`, `/model`, `resume`
      and `resume --last` against a fake Responses server.
  - Deferred from 5b: the permission mode changed from the interface (it
    is the configuration's; a per-session one is not in the facade);
    plan mode's "implement this plan?" prompt (switch back with `/mode`
    and ask); `/model`'s reasoning effort; the transcript pager's live
    tail of the running call and Codex's backtracking (Esc to edit an
    earlier message); a replayed tool call shows its name without the
    subject the live event carried (the history keeps the arguments,
    not the presentation); `/skills` lists the catalog, it does not
    toggle or insert a Skill; the sessions in the resume picker show no
    working directory or branch (the summaries have none); `exec`
    resumes no session; syntax highlighting (no `.tmTheme` is shipped).
  - Every start of the interface created a session (fixed in P0a: the
    first message creates it).
  - Still deferred from 5a: images in the composer (paste, attach); a
    session's earlier messages in the composer's history (only this
    run's); desktop notifications; Ctrl-Z suspend; the external editor;
    the time-worked separator counts from the status line's
    timer; a contained Agent panic writes its report to standard error,
    over the interface, as the Agent's hook does everywhere.
- [ ] Hanafubuki, when it moves its pin past Phase 5a: nothing to change.
      The facade gained re-exports only (`AgentTaskPlanSnapshot`,
      `AgentTaskPlanStep`, `AgentTaskPlanStepStatus`, `AgentToolPresentation`,
      `AgentUserInputOption`, `AgentUserInputQuestion`, `ApprovalAction`,
      `ApprovalActionKind`, `ToolOutput`, `ToolOutputContent`), and the TUI is
      a crate Hanafubuki does not depend on.
- [ ] Hanafubuki, when it moves its pin past Phase 5b: nothing to change.
      The facade gained one call, `AgentApplicationService::workspace_roots`
      (an addition over the host's `WorkspaceRootsPort`), and re-exports
      `AgentCompactionStatus`; the TUI and the
      CLI are crates Hanafubuki does not depend on, and nothing the model
      sees changed (the goldens and the history corpus are untouched).
- [x] Phase 6, the repository's part: clippy with `-D warnings` in CI
      (an `#[allow]` names its reason); `release.yml` (`plan`,
      `ci-result`, `checks`, `publish`, `licenses`, `binaries` for four
      targets, `github-release`), resumable by a manual run on the tag;
      the archives carry LICENSE, NOTICE and `THIRD-PARTY-LICENSES.txt`
      from `cargo-about` 0.9.2 (`about.toml`, `about.hbs`), which covers
      `nucleo`'s and the other MPL-2.0 crates; every crate but `xtask`
      packages and verifies (`cargo publish --workspace --dry-run`); the
      checklist is the site's releasing page.
- [ ] Phase 6, the user's step: `cargo xtask tag --push` on `main` for
      `v0.1.0` and watching the release run (about three and a half hours
      under crates.io's new-crate limit; resume with `gh workflow run
      release.yml --ref v0.1.0`). Done when the crates are on crates.io
      and the archives on the GitHub release.
- [ ] Hanafubuki, when it moves its pin past Phase 6: nothing to change.
      The clippy fixes boxed variants of internal enums only (the public
      `AgentModelContextBoundary` keeps its shape under an allow), and
      removed `ProcessOwnershipRegistry::len` and `ReservationSet::len`
      from `hanafubuki-agent-process`'s test support, which nothing
      called (its twin, `hanafubuki-process`, may keep them).
- [x] The configuration's recovery (Hanafubuki's open item from 2b): a
      repair whose original cannot be kept in `.recovery/` (or whose
      repaired file cannot be written) leaves the values in memory and the
      file read-only until restart, instead of failing the read; each
      repair is an `AgentConfigRecovery` (`AgentConfig::recoveries`), which
      `doctor` prints; a new file lists its keys in declared order. A hand
      edit is read at the next write or restart, by design (as Hanafubuki's
      Config and Codex).
- [ ] Hanafubuki, at its first pin past this (T3 does it): the recoveries in
      the Task Center's configuration recovery notice (the integration maps
      `AgentConfigRecovery` onto a `ConfigRecoveryReport` for the Agent's
      files); nothing else changes.

## Shared with the other repository

Twins: a fix to one is ported to the other by hand. Hanafubuki's `todo.md`
keeps the same list.

- `hanafubuki-agent-foundation` ↔ Hanafubuki's `hanafubuki-foundation`:
  `background_worker`, `blocking`, `cancellation`, `environment`,
  `json_normalization`, `module_faults`, `panic` (the host's hook takes a
  second containment check, the Agent's does not; the crash record's JSON is
  shared across the bridge: Hanafubuki's hook writes
  `data/agent/crash-record.json`, the Agent's start marker reads it),
  `paths`, `persistence`, `terminal_text`, `url_policy`, `user_agent`.
  Agent-only: `agent_settings`, `secrets`. Hanafubuki-only:
  `bounded_work_queue`, `concurrency`, `readiness`, `resolved`.
- `hanafubuki-agent-process` ↔ `hanafubuki-process`: `da_filter`,
  `executor`, `ownership`, `process_groups`, `terminal_size`,
  `test_support`, `tree`. Hanafubuki-only: `environment`.
- `hanafubuki-agent-host` ↔ `hanafubuki-module-contracts`: `error`,
  `fault`, `host` (without `RootServices::serve_bridge`), `logging`,
  `registry`, `restart`, `status`, `test_support`, `worker`.
  Hanafubuki-only: `bridge`.
- The integration goldens and history corpus (Hanafubuki's
  `hanafubuki-agent-integration/src/tests/golden/`) ↔ the facade's
  (`crates/agent/src/tests/golden/`): the history corpus is the same bytes in
  both; a history-format change re-records both.
