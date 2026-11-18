# TASKS.md — Discord MCP Server Development Plan

Legend: `TODO` = not started · `IN PROGRESS` = active · `DONE` = verified
complete · `CLOSED` = deliberately out of v1 scope or not verifiable here, with
the note saying which. A closed item changes no shipped behaviour.

## Phase 0 — Discovery and technical decisions

- [x] Verify current MCP protocol requirements and Rust SDK capabilities.
- [x] Verify `rmcp` STDIO and Streamable HTTP examples.
- [x] Evaluate Rust Discord libraries for current bot REST API support.
- [x] Choose one Discord REST/Gateway strategy; document rationale.
- [x] Confirm current Discord API version and endpoint requirements.
- [x] Define minimum bot scopes and permissions for v1.
- [x] Record dependency versions in `docs/DEPENDENCIES.md`.
- [x] Create threat model and data-handling notes.

**Exit criteria:** Dependency and protocol decisions are documented; no unsupported API assumptions remain.

## Phase 1 — Project foundation

- [x] Create Cargo project and library/binary targets.
- [x] Configure Windows MSVC build target in CI.
- [x] Add module skeleton and architectural boundaries.
- [x] Add configuration loading and validation.
- [x] Add `.env.example` with variable names only.
- [x] Add typed error model and stable error categories.
- [x] Add tracing with stderr output in STDIO mode.
- [x] Add secret wrapper and redaction tests.
- [x] Commit `Cargo.lock`.
- [x] Add `cargo fmt`, Clippy, and test CI jobs.

**Exit criteria:** Project builds on Windows; configuration validation and baseline CI pass.

## Phase 2 — Discord client adapter

- [x] Implement bot-token authentication from environment/credential store.
- [x] Implement typed Discord client abstraction.
- [x] Implement snowflake ID newtypes or validated string types.
- [x] Implement guild listing and guild lookup.
- [x] Implement channel listing and channel lookup.
- [x] Implement bounded message history retrieval.
- [x] Implement message lookup.
- [x] Implement member lookup/listing only where supported and permitted.
- [x] Implement pagination and stable ordering.
- [x] Implement request timeouts.
- [x] Implement rate-limit handling and bounded retry policy.
- [x] Map Discord HTTP errors to internal error categories.
- [x] Add mock-based adapter tests.

**Exit criteria:** Read-only Discord operations work against mocks and an optional test bot.

Met against mocks first: 22 adapter tests run the real twilight client over a
local socket (`tests/integration`). Live verification followed: the eight read
tools were exercised against a real guild (`test_mcp_server`) over both STDIO and
HTTP, including non-empty message text once the Message Content intent was
enabled.

## Phase 3 — Security and policy engine

- [x] Add read-enabled and write-enabled flags.
- [x] Add guild allowlist.
- [x] Add channel allowlist.
- [x] Add tool risk classification.
- [x] Add message length and page-size limits.
- [x] Add mention sanitization/default-deny behavior.
- [x] Add concurrency limits.
- [x] Add destructive-action approval abstraction.
- [x] Fail closed if approval is unavailable.
- [x] Add policy unit tests and audit event model.

**Exit criteria:** All policy rules are tested independently of MCP and Discord transport.

Met at the gate boundary: 35 unit tests drive the policy rules and the enforcing
decorator against a recording fake `DiscordApi`, where "denied" means the fake was
never called. Policy is enforced in `AppService::bootstrap`, so the handle stored
in `AppState` is the gate, never the raw client. No destructive tool exists, so the
approval path is proven against fake approval sources rather than a real one.

## Phase 4 — MCP tools and resources

- [x] Implement MCP initialization and capability negotiation.
- [x] Implement central tool registry.
- [x] Implement strict JSON schemas.
- [x] Implement `discord_list_guilds`.
- [x] Implement `discord_get_guild`.
- [x] Implement `discord_list_channels`.
- [x] Implement `discord_get_channel`.
- [x] Implement `discord_get_messages`.
- [x] Implement `discord_get_message`.
- [x] Implement member tools if supported by selected permissions/intents.
- [x] Implement read-only resource URIs.
- [x] Add optional draft/review prompts. *(CLOSED — not implemented: prompts are
      optional in the specification and this build has no draft or review
      workflow to attach them to. Nothing else depends on them)*
- [x] Add schema and tool contract tests.

**Exit criteria:** Read-only tools and resources are discoverable and execute correctly.

Met: eight read tools and the `discord://guilds` resource are registered over the
policy gate, with `deny_unknown_fields` argument schemas and `readOnlyHint`
annotations. Denials are proven over stdio (`isError` results carrying
`POLICY_BLOCKED`), and successful reads were confirmed end to end against a real
guild over both transports.

## Phase 5 — STDIO transport

- [x] Integrate MCP SDK STDIO transport.
- [x] Ensure stdout contains protocol messages only.
- [x] Route all logs to stderr.
- [x] Handle stdin closure and Ctrl+C.
- [x] Test initialize, list tools, call tool, and shutdown.
- [x] Test launch through a Windows MCP desktop client. *(CLOSED — not verified:
      no desktop client is installed here. The protocol is exercised as a child
      process over real pipes, which is the same contract a desktop client uses,
      so this is a verification gap rather than a functional one)*
- [x] Add sample STDIO client configuration to README.

**Exit criteria:** A Windows MCP client can launch and use the server over STDIO without protocol contamination.

## Phase 6 — Streamable HTTP transport

- [x] Integrate MCP SDK Streamable HTTP transport.
- [x] Bind to `127.0.0.1` by default.
- [x] Implement authentication for non-loopback deployment.
- [x] Validate Host and Origin.
- [x] Configure body-size limits (`http.max_request_bytes`).
- [x] Configure request-time limits. `http.request_timeout_seconds` bounds each
      non-`GET` request from arrival to the end of its response body; the
      long-lived standalone `GET` event stream is exempt because expiring it
      would disconnect every subscribed session.
- [x] Configure session expiry. `http.session_expiry_seconds` is the SDK session
      manager's idle timeout (`SessionConfig::keep_alive`), replacing its 5-minute
      default.
- [x] Configure HTTP-level concurrency. `http.max_in_flight_requests` (default
      64) caps non-`GET` requests at the listener; one beyond the cap is refused
      with `503` and `Retry-After` instead of being queued, and the permit rides
      with the response body so a streaming answer holds its slot to the last
      byte. `GET` and `/health` are exempt.
- [x] Add health/readiness endpoint if compatible with deployment design.
      `GET /health` answers status, server, version and transport behind the same
      bearer gate, and never contacts Discord.
- [x] Test initialization over HTTP, including the refused cases.
- [x] Test sessions, notifications, and disconnects over HTTP. Process tests
      cover session creation and its announced id, a notification accepted with
      `202`, a terminated session refused with `404`, an unknown session refused,
      and an aborted `GET` event stream leaving the listener usable. Resumption
      (event replay after a reconnect) is still unverified.
- [x] Document TLS/reverse-proxy deployment. `docs/SETUP.md` §7 now has a
      "Behind a reverse proxy (TLS)" section with nginx and Caddy configurations,
      the mandatory `Host` rewrite, SSE buffering, the ignored `X-Forwarded-*`
      headers and the proxied health check. TLS termination itself is still not
      implemented in the server.
- [x] Verify unauthenticated public binding is rejected.

**Exit criteria:** Streamable HTTP passes protocol tests and unsafe exposure is blocked.

Met for exposure: process tests prove that a request with no token, a wrong token,
a foreign `Host` and a foreign `Origin` are all refused, that a valid token is
served, and that a non-loopback bind without its bearer token exits `2` before it
binds. Session lifecycle is covered for creation, acknowledgement, termination
and disconnect recovery; event replay after a reconnect (resumption) is not.

## Phase 7 — Controlled write operations

- [x] Implement `discord_send_message`. Registered unconditionally and refused
      by the gate while writes are off, so a client can see the capability and
      the reason it is unavailable. The adapter posts with
      `allowed_mentions: {parse: []}` and no retries.
- [x] Keep write operations disabled by default. `policy.write_enabled` is
      `false` in the compiled defaults, and validation refuses enabling it with
      an empty `policy.channel_allowlist`.
- [x] Enforce channel allowlist. `Policy::check_channel` runs before a permit is
      taken or any request is built.
- [x] Enforce message length. `policy.max_message_characters`, measured in
      characters, refused as `INVALID_ARGUMENT` rather than truncated.
- [x] Default allowed mentions to none. The content is neutralised by policy and
      the request itself sends an empty mention parse list.
- [x] Verify bot permissions before sending where practical. *(CLOSED — not
      implemented, and not needed for correctness: there is no pre-flight
      permission read. Discord's `403` becomes `PERMISSION_DENIED`, is audited as
      `failed`, and nothing is retried, so the authoritative check already
      happens at Discord; a pre-flight call would add a request without changing
      the outcome)*
- [x] Add audit event for every attempted write. `run_write` records exactly one
      event per attempt: `allowed`, `denied` for a policy refusal, or `failed`
      with the category when Discord rejected an allowed attempt.
- [x] Test policy-denied, permission-denied, rate-limited, and success cases.
      Gate tests cover the first three plus empty/oversized content and mention
      neutralisation; mock-Discord tests cover success, `403`, and a `429` that
      is waited out; protocol tests cover the disabled and unlisted-channel
      refusals end to end.
- [x] Implement edit-own-message only after ownership checks. *(CLOSED — out of
      v1 scope. Editing requires ownership checks against the bot's own messages,
      which do not exist here; no tool edits anything)*
- [x] Implement delete-own-message only with trustworthy confirmation. *(CLOSED —
      out of v1 scope. MCP has no trustworthy approval channel, which is why
      `ToolRisk::Destructive` fails closed and no delete tool is registered)*
- [x] Implement reactions only after separate policy tests. *(CLOSED — out of v1
      scope. Reactions would need their own policy and audit tests; the write
      surface is one send tool by design)*
- [x] Do not add bulk deletion or mass moderation to v1. Nothing in this phase
      can edit, delete or bulk-delete.

**Exit criteria:** Write operations are opt-in, bounded, audited, and safe against accidental mentions.

## Phase 8 — Observability and operational hardening

- [x] Add correlation IDs and tracing spans. Each tool call allocates an id
      carried in an `mcp.tool` span and in a task-local; the gate opens a nested
      `discord.call` span per Discord request and stamps the id on every audit
      record, so two requests from one call can be joined without recording
      arguments. HTTP requests carry their own id in an `http.request` span.
- [x] Redact authorization headers and secrets unconditionally. `redact()`
      strips `Bot <token>` and `Bearer <token>` shapes and the values of
      `authorization`, `x-api-key` and `token` keys whatever the secret is, and
      both `RedactedStderr` and the panic hook run through it.
- [x] Ensure message content is not logged by default. Audit records carry
      action, target IDs, outcome and category only;
      `logging.redact_message_content` defaults to `true` and
      `logging.log_tool_arguments` to `false`.
- [x] Add log rotation if file logging is enabled. *(CLOSED — not applicable:
      every record goes to stderr and there is no file sink, so there is nothing
      to rotate. This becomes real work only if one is added)*
- [x] Add actionable startup diagnostics. `--validate-config` reports the
      configuration source, transport, token presence and effective exposure
      (bind, authentication, write state), and every startup failure carries a
      stable error category and exit code. `docs/SETUP.md` §9 maps symptoms to
      fixes.
- [x] Add graceful shutdown and cancellation. The HTTP listener uses axum's
      graceful shutdown on Ctrl+C: it stops accepting, finishes requests already
      being served (each is bounded by `http.request_timeout_seconds`) and logs
      the stop. STDIO keeps its protocol shutdown on stdin close. No automated
      test drives a real signal; that remains a manual check.
- [x] Test malformed API responses and network interruptions. Malformed bodies,
      slow responses, the retry budget and a connection dropped mid-body are
      covered (`an_unparsable_body_maps_to_network_error`,
      `a_truncated_response_fails_without_a_retry`, `a_slow_response_times_out`,
      `retries_are_bounded_and_report_the_discord_category`).
- [x] Test ambiguous write outcomes and no blind retries. A timed-out write and
      a `500` on a write are each sent exactly once and surface the error
      (`a_timed_out_write_is_never_resent`, `a_server_error_on_a_write_is_not_retried`),
      and a write rejected with `403` is not retried either.
- [x] Review dependency vulnerabilities and update policy. `cargo-audit 0.23.0`
      was installed and run against the committed lock file: 204 crates, 0
      vulnerabilities, 1 warning (`chrono` unmaintained via `rmcp`/`schemars`, no
      fixed version). Recorded with its reachability and review trigger in
      `docs/DEPENDENCIES.md`, not suppressed.

**Exit criteria:** Operational diagnostics are useful without leaking credentials or message content.

## Phase 9 — Documentation and release packaging

- [x] Complete README setup guide. The README carries build, configuration,
      environment, STDIO client config and HTTP sections, and links the full
      walkthrough.
- [x] Document Discord Developer Portal bot creation. `docs/SETUP.md` §2.1–2.2
      (application, token) and §2.3 (privileged intents, with the `flags` check
      that proves the toggle took effect).
- [x] Document OAuth2 installation and least-privilege permissions.
      `docs/SETUP.md` §2.4: `bot` scope only, permissions `66560`, and the call
      that shows the bits the bot actually holds.
- [x] Document environment-variable options. `docs/SETUP.md` §4 and the README
      environment table cover both tokens, `DISCORD_MCP_CONFIG` and `RUST_LOG`.
- [x] Document Credential Manager options. `docs/SETUP.md` §4 compares every
      place the bot token can live — client `env` block, per-session variable,
      `.env`, `setx`, and Windows Credential Manager read by a launcher — with
      each option's trade-off and the removal step. Native `keyring` lookup is not
      implemented (optional in `SPEC.md`) and the section says so instead of
      implying it exists.
- [x] Document STDIO configuration for Windows MCP clients. `docs/SETUP.md` §6
      gives the Claude Desktop `claude_desktop_config.json` form; launching from
      an installed desktop client is still unverified (see Phase 5).
- [x] Document Streamable HTTP configuration and secure exposure.
      `docs/SETUP.md` §7: configuration, the full request sequence, every status
      code, the loopback/`Host`/`Origin` rules and the TLS caveat.
- [x] Complete `docs/SECURITY.md`. Identity model, assets, adversaries,
      implemented controls, data handling, transport exposure and an explicit
      not-yet-implemented table.
- [x] Complete `docs/TOOLS.md`. One section per tool with arguments, result
      JSON, annotations, the policy preconditions that apply, error codes, retry
      and timeout rules, the `discord://guilds` resource, and what the surface
      deliberately does not do. Linked from the README and `docs/SETUP.md` §6.
- [x] Complete `docs/DEPENDENCIES.md`. Pinned crate set with the reason for each
      choice, the rmcp and twilight facts verified from vendored source, and the
      v1 scopes, permissions, intents and endpoint list.
- [x] Add CHANGELOG. `CHANGELOG.md` groups changes by capability, states what is
      and is not released, and now carries the `v0.1.0` packaging facts.
- [x] Build release executable for Windows x64. `cargo build --release` produced
      `target\release\discord-mcp.exe` for `x86_64-pc-windows-msvc`.
- [x] Generate SHA-256 checksum.
      `dist/discord-mcp-0.1.0-x86_64-pc-windows-msvc.sha256` records the hash of
      the release build from the tagged commit, with the command that regenerates
      it; `docs/SETUP.md` §8 verifies it. The executable is not committed. The build
      is byte-reproducible — `build.rs` passes `/Brepro` and
      `scripts/build-release.ps1` remaps absolute dependency paths — so the
      checksum verifies a rebuild in the same checkout. A build from another
      directory differs, because Cargo hashes the package's absolute path into
      crate metadata; `docs/SETUP.md` §8 says so.
- [x] Test clean installation on a Windows machine. A fresh `git worktree` of
      the tag was built with `cargo build --release --locked`, then
      `--validate-config` and a STDIO `initialize`/`tools/call` handshake were run
      from that tree, so the guide's §3 path needs no uncommitted file. Still one
      machine: the Discord-portal steps and a desktop-client launch were not
      repeated from scratch.
- [x] Tag first release. Annotated tag `v0.1.0` on the release commit. It is a
      local tag: the repository has no remote, so nothing was pushed or
      published.

**Exit criteria:** A new operator can configure a bot, install the server, connect an MCP client, and use the documented tools safely.

## Phase 10 — Release gate

Signed off on 2026-11-18 against the release artifact built from commit `d64ba14`
with `cargo build --release --locked --target x86_64-pc-windows-msvc` from an
empty `target/` — not against the working tree.

- [x] All tests pass. 151 tests: 13 library, 30 integration (real twilight client
      against a local mock Discord), 24 protocol (the real binary as a child
      process, plus HTTP refusals and session lifecycle), 84 unit.
- [x] `cargo fmt --check` passes. `cargo fmt --all --check` exits 0.
- [x] `cargo clippy --all-targets --all-features -- -D warnings` passes. Exit 0
      with no warnings.
- [x] Windows MSVC release build succeeds. From a cleaned tree,
      `x86_64-pc-windows-msvc` release build finished in 2m57s producing an
      11.6 MB executable. The build is byte-reproducible within a checkout:
      `build.rs` passes `/Brepro` to the MSVC linker, and the release script
      normalises the absolute paths that dependency panic messages would embed.
- [x] STDIO protocol output is clean. `initialize` plus `tools/list` produced two
      stdout frames, both parsing as JSON-RPC 2.0, with zero non-JSON lines; all
      eight log lines went to stderr.
- [x] HTTP loopback/authentication protections pass. Against the artifact: no
      token `401`, wrong token `401`, foreign `Host` `403`, foreign `Origin` `403`,
      correct token `200` with a session id, `GET /health` `401` then `200` with
      the status JSON.
- [x] Token redaction tests pass. Seven redaction tests cover `Bot`/`Bearer`
      shapes, header and JSON key values, and the panic path.
- [x] No self-bot or user-token functionality exists. The client is built from
      the bot token only; there is no user-authorization path and no `Authorization`
      header built outside the inbound HTTP gate.
- [x] Write tools are disabled by default. `--validate-config` on the shipped
      defaults prints `write tools are disabled`, `policy.write_enabled = false`,
      and validation refuses enabling it with an empty channel allowlist.
- [x] Mass mentions are disabled by default. `policy.allow_mass_mentions = false`,
      validation rejects `true`, and the write path neutralises mentions and sends
      an empty mention parse list.
- [x] Discord rate limits are respected. twilight's limiter waits out `429` and
      resends within the per-call deadline; three tests cover the read retry, the
      write wait, and the error mapping when a limit cannot be waited out.
- [x] Documentation matches shipped behavior. `docs/TOOLS.md` was re-checked
      claim by claim against the code, and the version, nine-tool surface and
      status codes match what this artifact reports.

**Release decision:** Do not ship if any security or protocol acceptance criterion is unmet.
