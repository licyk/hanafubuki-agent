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
the plan records them. M1, M2, M3 and M4 have landed; M5–M7 have not started.

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
      Open:
      - The "a pattern that matches nothing" warning: the sourced tools
        vary per step, so it needs the live registries at selection; not
        built.
      - `set_session_role` is not atomic when the role sets a model: a
        failed model selection leaves the new scope with the old model.
      - An older build reading a scoped session runs it unscoped (the
        plan's migration table accepts it); a deny pattern a later build
        writes that this one cannot parse is dropped, which widens.
      - The configuration center does not offer role choice yet; the TUI's
        `/role` and the CLI's `--role` do.
- [ ] M5: role-aware subagents.
      - `spawn_agent { agent_type }` (D7).
      - `AgentRoleCatalogReader`; children take the role's instructions, model and
        narrowed scope.
      - The guidance lists spawnable roles (at most 16, sorted).
      - Model-visible: regenerate the goldens on purpose, here and in Hanafubuki.
      Done: —. Open: —.
- [ ] M6: roles from packages.
      - The `agents` component of our format.
      - Claude Code `agents/*.md`, with the tool-name table.
      - `<plugin>:<role>` ids, present while the package is enabled.
      - Update `docs/plan-plugin-system.md` §3.3 and §10.
      Done: —. Open: —.
- [ ] M7 (D6): linked development packages.
      - `plugins link <dir>`.
      - A contained watcher that restarts the guest and re-reads its content.
      Done: —. Open: —.

## Plan: a configuration center for every Agent setting (2026-11-11)

The plan is [docs/plan-configuration-center.md](docs/plan-configuration-center.md);
the implementation handoff is recorded in
[docs/plan-configuration-center-handoff.md](docs/plan-configuration-center-handoff.md).
Each phase is implemented and validated separately; C1–C3 are committed and
pushed, while C4–C6 remain in the working tree pending authorization. The work
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
      pages prerendered. The changes are in the working tree, not committed.

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
      proxy variables.
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
