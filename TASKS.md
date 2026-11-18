# TASKS.md — Development plan

Status legend: `[x]` done or closed · `CLOSED` = deliberately out of v1 scope or
not verifiable here, with the reason inline. Phases 0–9 are complete, Phase 10 is
the historical v0.1.0 gate, Phase 11 is the server-control branch gate, and Phase
12 is the current read-surface branch.

## Phase 0 — Discovery and technical decisions

- [x] Verify MCP protocol requirements and Rust SDK capabilities.
- [x] Verify `rmcp` STDIO and Streamable HTTP examples.
- [x] Evaluate Rust Discord libraries for bot REST support.
- [x] Choose one Discord REST strategy; document the rationale.
- [x] Confirm the Discord API version and endpoint requirements.
- [x] Define minimum bot scopes and permissions for v1.
- [x] Record dependency versions in `docs/DEPENDENCIES.md`.
- [x] Create threat model and data-handling notes.

**Exit criteria:** Dependency and protocol decisions documented; no unsupported API assumptions.

## Phase 1 — Project foundation

- [x] Cargo project with library and binary targets.
- [x] Windows MSVC target in CI.
- [x] Module skeleton and architectural boundaries.
- [x] Configuration loading and validation.
- [x] `.env.example` with variable names only.
- [x] Typed error model with stable categories.
- [x] Tracing to stderr in STDIO mode.
- [x] Secret wrapper and redaction tests.
- [x] `Cargo.lock` committed.
- [x] `fmt`, Clippy and test CI jobs.

**Exit criteria:** Builds on Windows; configuration validation and baseline CI pass.

## Phase 2 — Discord client adapter

- [x] Bot-token authentication from the environment.
- [x] Typed `DiscordApi` abstraction; twilight stays in the adapter.
- [x] Snowflake ID newtypes with validated parsing.
- [x] Guild listing and lookup.
- [x] Channel listing and lookup.
- [x] Bounded message history with paging.
- [x] Single-message lookup.
- [x] Member listing and lookup (intent permitting).
- [x] Stable pagination cursors.
- [x] Per-request timeouts.
- [x] Rate-limit handling and bounded retries for reads.
- [x] Discord errors mapped to internal categories.
- [x] Mock-based adapter tests.

**Exit criteria:** Read-only Discord operations work against mocks and a real guild.

## Phase 3 — Security and policy engine

- [x] Read and write feature flags.
- [x] Guild and channel allowlists.
- [x] Deny-by-default tool risk classification.
- [x] Message-length and page-size limits.
- [x] Mention neutralisation; mass mentions rejected by validation.
- [x] Concurrency limit with backpressure.
- [x] Destructive-action approval abstraction *(CLOSED — removed before the server-control merge: no client-independent confirmation primitive, so `dry_run` plus the class flag, allowlist, budget and audit are the approval model)*.
- [x] Policy unit tests and content-free audit model.

**Exit criteria:** Every policy rule tested independently of MCP and Discord transport.

## Phase 4 — MCP tools and resources

- [x] MCP initialisation and capability negotiation.
- [x] Tool registry with strict JSON schemas.
- [x] Nine read tools: guilds, channels, messages, members and `discord_get_capabilities`.
- [x] Resource `discord://guilds`.
- [x] Prompts *(CLOSED — optional in the specification; nothing here needs one)*.
- [x] Schema and tool-contract tests.

**Exit criteria:** Read-only tools and resources discoverable and correct.

## Phase 5 — STDIO transport

- [x] MCP SDK STDIO transport integrated.
- [x] stdout carries protocol frames only; logs on stderr.
- [x] stdin closure and Ctrl+C handled.
- [x] initialise, list, call and shutdown tested.
- [x] Desktop-client launch *(CLOSED — not verified: no client installed here; the same stdio contract is tested through real pipes)*.
- [x] Sample client configuration in the README.

**Exit criteria:** A Windows MCP client can launch and use the server without protocol contamination.

## Phase 6 — Streamable HTTP transport

- [x] MCP SDK Streamable HTTP transport integrated.
- [x] Loopback bind by default.
- [x] Bearer authentication, required for any non-loopback bind.
- [x] `Host` and `Origin` validation.
- [x] Body-size, request-deadline, session-expiry and in-flight caps.
- [x] `GET /health` behind the same token, never contacting Discord.
- [x] Protocol tests for initialisation and every refusal.
- [x] Session lifecycle: creation, acknowledgement, termination, disconnect.
- [x] TLS/reverse-proxy deployment documented in `docs/SETUP.md` §7.
- [x] Unauthenticated public binding rejected.

**Exit criteria:** HTTP passes protocol tests and unsafe exposure is blocked.

## Phase 7 — Controlled write operations

- [x] `discord_send_message`, registered but refused while writes are off.
- [x] Writes off by default; enabling requires a channel allowlist.
- [x] Channel allowlist checked before any request is built.
- [x] Length limit enforced as `INVALID_ARGUMENT`, not truncation.
- [x] Mentions neutralised, plus an empty mention parse list on the request.
- [x] Pre-flight permission read *(CLOSED — not needed: Discord's `403` is authoritative and already audited)*.
- [x] One audit record per attempt: `allowed`, `denied` or `failed`.
- [x] Tests for policy-denied, permission-denied, rate-limited and success.
- [x] Edit / delete / react shipped on `feat/server-control` (Phase 11), each behind its own class flag.
- [x] Bulk deletion stays administrative-only and mass mentions stay off (Phase 11).

**Exit criteria:** Writes are opt-in, bounded, audited, and safe against accidental mentions.

## Phase 8 — Observability and operational hardening

- [x] Correlation ids and spans across tool calls and Discord requests.
- [x] Unconditional redaction of tokens and authorization headers.
- [x] Message content and tool arguments not logged by default.
- [x] Log rotation *(CLOSED — not applicable: stderr only, no file sink)*.
- [x] Actionable startup diagnostics via `--validate-config`.
- [x] Graceful shutdown: stop accepting, drain within the request deadline.
- [x] Malformed-response and network-interruption tests, including a truncated body.
- [x] Ambiguous writes: timed-out and `500` attempts are never resent.
- [x] Dependency advisory scan and written policy (1 unmaintained warning, accepted).

**Exit criteria:** Diagnostics are useful without leaking credentials or content.

## Phase 9 — Documentation and release packaging

- [x] README: install, quick start, client config, configuration, HTTP, docs index.
- [x] Developer Portal setup in `docs/SETUP.md` §2, with the `flags` check for intents.
- [x] Least-privilege install URL (`bot` scope, permissions `66560`).
- [x] Environment variables and token storage options, including Credential Manager.
- [x] STDIO client configuration for Windows.
- [x] HTTP configuration, exposure rules and the TLS proxy section.
- [x] `docs/SECURITY.md`, `docs/TOOLS.md`, `docs/DEPENDENCIES.md` complete.
- [x] `CHANGELOG.md`.
- [x] Windows x64 release build.
- [x] SHA-256 checksum, with a byte-reproducible build script.
- [x] Clean-checkout build and handshake from a fresh worktree of the tag.
- [x] Annotated tag `v0.1.0`, pushed, with the binary attached as a GitHub release asset.

**Exit criteria:** A new operator can configure a bot, install the server, connect a client and use the tools safely.

## Phase 10 — Release gate

Signed off on 2026-11-18 against the release artifact built from the tag with
`cargo build --release --locked --target x86_64-pc-windows-msvc` from an empty
`target/`. This gate covers the `v0.1.0` artifact only (151 tests at that tag);
the branch is gated by Phase 11 instead.

- [x] All 151 tests pass.
- [x] `cargo fmt --all --check` passes.
- [x] Clippy with `-D warnings` passes.
- [x] Windows MSVC release build succeeds.
- [x] STDIO stdout is protocol frames only.
- [x] HTTP refusals pass: no token and wrong token `401`, foreign `Host` and `Origin` `403`, valid token served.
- [x] Token-redaction tests pass.
- [x] No self-bot or user-token path exists.
- [x] Writes disabled by default.
- [x] Mass mentions disabled by default.
- [x] Rate limits respected.
- [x] Documentation matches shipped behaviour.

**Release decision:** Do not ship if any security or protocol acceptance criterion is unmet.

## Phase 11 — Server control (`feat/server-control`)

Everything below is implemented and tested on the branch; this is its gate.

- [x] `discord_get_capabilities`: enabled classes, autonomy limits and the authenticated bot.
- [x] Seven message writes: `discord_send_message`, `discord_edit_message`, the three reaction tools, `discord_pin_message` and `discord_unpin_message`.
- [x] Ten structure/control tools: `discord_delete_message`, channels (create/edit/delete), permission overwrites (set/delete), threads (create/edit) and invites (create/delete).
- [x] Six administrative tools: `discord_bulk_delete_messages` and five role tools (create, edit, delete, assign, remove). `ADMINISTRATOR` cannot be granted, role names are 1–100 characters locally, and `@everyone` is refused for delete, assign and remove.
- [x] Three mutation classes, each off by default and each requiring its own allowlist: `write_enabled` (channel), `control_enabled` (channel), `administrative_enabled` (guild).
- [x] Shared rolling budget (`policy.autonomy.max_actions_per_minute`) across every mutation, including `discord_send_message`, plus a per-channel send cap (`max_sends_per_channel_per_minute`); the window is persisted to `budget_state_file` so a restart cannot reset it.
- [x] `policy.autonomy.require_dry_run`: named tools refuse a real call; an unclassified name is a configuration error.
- [x] `dry_run` on every mutation except `discord_send_message`: all local checks, no Discord request, no budget, no audit record.
- [x] Exactly one content-free audit record per real mutation attempt (`allowed`, `denied`, `failed`); reads record allowances and policy denials.
- [x] Unconditional guard against editing or deleting the bot's own highest role, or removing it from the bot; no setting disables it.
- [x] Remote scope resolved before spending an action: a thread through its parent channel, an invite through its channel, roles through the hierarchy.
- [x] Bulk delete: 2–`max_bulk_delete` distinct IDs, none older than 14 days, checked from the snowflake without a request.
- [x] `--read-only` / `DISCORD_MCP_READ_ONLY=1` disables all three mutation classes for the session.
- [x] `policy.protected_channels` and `policy.protected_roles`: edit/delete/overwrite/thread-edit refusals enforced before the budget and independent of the allowlists.
- [x] Budget state file has a single process owner, enforced by an exclusive lock; a second instance with mutations enabled fails startup, and read-only instances do not open it.
- [x] Removed the unused `policy.require_confirmation_for_destructive` setting; configuration rejects it, and the docs describe `dry_run` plus allowlists as the approval model.
- [x] Documentation updated against the code: `README.md`, `docs/TOOLS.md`, `docs/SETUP.md`, `docs/SECURITY.md`, `CHANGELOG.md`.
- [x] All 230 tests pass (17 lib, 50 integration, 34 protocol, 129 unit).
- [x] `cargo fmt --all --check` passes.
- [x] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [x] Live role-tool checks in the test guild: create/edit/delete/assign/remove, dry runs, `@everyone` refusals, `ADMINISTRATOR` refusal, and above-bot calls returning `403` / `50013`.
- [x] Live run through MCP Bridge over stdio and Streamable HTTP: 9/9 reads, message, structure, role and policy paths; results in `docs/LIVE_TESTING.md`.
- [x] All mutation classes remain off in the built-in configuration.

### Outstanding on the branch

- [x] Live smoke for the message tools through MCP Bridge (send, edit, delete, reactions, pins, bulk delete); see `docs/LIVE_TESTING.md`.
- [x] Live smoke for the structure tools (channel create/edit, overwrites, threads, invites); channel create/edit were refused by Discord for missing Manage Channels.
- [x] Live smoke for the HTTP transport against a branch build, including the `401`/`403` refusal matrix and a bridge Streamable HTTP connection.
- [x] Live budget exhaustion, a `require_dry_run` refusal and a `--read-only` session.
- [ ] A second bot/integration role to observe Discord's managed-role delete and permission-edit refusals.
- [ ] Positive `PIN_MESSAGES`, Manage Messages and Manage Threads paths (not granted to the test bot).
- [ ] Open the PR, run CI, review and merge.

**Release decision:** Do not merge while any security or policy acceptance criterion is unmet.

## Phase 12 — Read surface (`feat/user-message-search`)

- [x] `discord_get_user_messages`: scans the newest messages in every visible text and announcement channel for one author, bounded by `scan_limit` per channel and `limit` overall, filtered by `policy.channel_allowlist`, with per-channel errors and `has_more`/`truncated` reporting.
- [x] Classified as a read tool, so `read_enabled`, guild checks and channel checks apply, and `require_dry_run` accepts its name.
- [x] Documentation updated: `README.md`, `docs/TOOLS.md`, `SPEC.md`, `llm_instruction.md`, `CHANGELOG.md`.
- [x] All 231 tests pass (17 lib, 50 integration, 35 protocol, 129 unit).
- [ ] Live scan against the test guild, including a channel without Read Message History.

**Release decision:** Merge only after the live scan confirms the per-channel error path.
