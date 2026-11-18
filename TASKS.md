# TASKS.md — Discord MCP Server Development Plan

Legend: `TODO` = not started · `IN PROGRESS` = active · `DONE` = verified complete

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
- [ ] Add optional draft/review prompts. *(deliberately not implemented: prompts are optional in the
      specification and this build has no draft/review content to offer)*
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
- [ ] Test launch through a Windows MCP desktop client. *(not verified: no desktop
      client is installed here. The protocol is exercised as a child process over
      real pipes, which is the same contract a desktop client uses)*
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

- [ ] Implement `discord_send_message`.
- [ ] Keep write operations disabled by default.
- [ ] Enforce channel allowlist.
- [ ] Enforce message length.
- [ ] Default allowed mentions to none.
- [ ] Verify bot permissions before sending where practical.
- [ ] Add audit event for every attempted write.
- [ ] Test policy-denied, permission-denied, rate-limited, and success cases.
- [ ] Implement edit-own-message only after ownership checks.
- [ ] Implement delete-own-message only with trustworthy confirmation.
- [ ] Implement reactions only after separate policy tests.
- [ ] Do not add bulk deletion or mass moderation to v1.

**Exit criteria:** Write operations are opt-in, bounded, audited, and safe against accidental mentions.

## Phase 8 — Observability and operational hardening

- [ ] Add correlation IDs and tracing spans. *(no request or trace id is
      generated or attached to a tool execution; `docs/SECURITY.md` §7 tracks it
      as outstanding)*
- [x] Redact authorization headers and secrets unconditionally. `redact()`
      strips `Bot <token>` and `Bearer <token>` shapes and the values of
      `authorization`, `x-api-key` and `token` keys whatever the secret is, and
      both `RedactedStderr` and the panic hook run through it.
- [x] Ensure message content is not logged by default. Audit records carry
      action, target IDs, outcome and category only;
      `logging.redact_message_content` defaults to `true` and
      `logging.log_tool_arguments` to `false`.
- [ ] Add log rotation if file logging is enabled. *(no file logging exists —
      every record goes to stderr — so there is nothing to rotate; this becomes
      real work only if a file sink is added)*
- [x] Add actionable startup diagnostics. `--validate-config` reports the
      configuration source, transport, token presence and effective exposure
      (bind, authentication, write state), and every startup failure carries a
      stable error category and exit code. `docs/SETUP.md` §9 maps symptoms to
      fixes.
- [ ] Add graceful shutdown and cancellation. *(STDIO stops when the client
      closes stdin, which is the protocol's own shutdown; the HTTP listener has
      no graceful drain and Ctrl+C is ordinary process termination)*
- [ ] Test malformed API responses and network interruptions. *(malformed bodies,
      slow responses and the retry budget are covered by
      `an_unparsable_body_maps_to_network_error`, `a_slow_response_times_out` and
      `retries_are_bounded_and_report_the_discord_category`; a connection reset
      or drop mid-body is not)*
- [ ] Test ambiguous write outcomes and no blind retries. *(no write path exists
      yet, so there is no ambiguous write outcome to test; reads retry only
      transient failures, bounded by `discord.max_retries`, and that budget is
      tested)*
- [ ] Review dependency vulnerabilities and update policy. *(every dependency is
      pinned exactly and `Cargo.lock` is committed, but no `cargo audit` run or
      written vulnerability-response policy exists)*

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
- [ ] Document Credential Manager options. *(not implemented: the bot token comes
      from the environment variable named by `discord.token_env`, there is no
      `keyring`-backed store, so there is nothing to document)*
- [x] Document STDIO configuration for Windows MCP clients. `docs/SETUP.md` §6
      gives the Claude Desktop `claude_desktop_config.json` form; launching from
      an installed desktop client is still unverified (see Phase 5).
- [x] Document Streamable HTTP configuration and secure exposure.
      `docs/SETUP.md` §7: configuration, the full request sequence, every status
      code, the loopback/`Host`/`Origin` rules and the TLS caveat.
- [x] Complete `docs/SECURITY.md`. Identity model, assets, adversaries,
      implemented controls, data handling, transport exposure and an explicit
      not-yet-implemented table.
- [ ] Complete `docs/TOOLS.md`. *(not written: the tool table lives in
      `docs/SETUP.md` §6 and the README, with no per-tool argument and response
      reference of its own)*
- [x] Complete `docs/DEPENDENCIES.md`. Pinned crate set with the reason for each
      choice, the rmcp and twilight facts verified from vendored source, and the
      v1 scopes, permissions, intents and endpoint list.
- [ ] Add CHANGELOG. *(no `CHANGELOG.md` exists)*
- [x] Build release executable for Windows x64. `cargo build --release` produced
      `target\release\discord-mcp.exe` for `x86_64-pc-windows-msvc`.
- [ ] Generate SHA-256 checksum. *(nothing produces or commits one)*
- [ ] Test clean installation on a Windows machine. *(the binary is built in
      place here; it has not been installed from a clean checkout following only
      the guide)*
- [ ] Tag first release. *(the repository has no tags)*

**Exit criteria:** A new operator can configure a bot, install the server, connect an MCP client, and use the documented tools safely.

## Phase 10 — Release gate

These are signed off against the release artifact, not the working tree, so they
stay open until Phases 7–9 land. Several already hold at the current commit and
will be re-run then: 132 tests, `fmt`, Clippy, the release build, token-redaction
coverage, no self-bot path, no write tools, mass mentions rejected by validation.

- [ ] All tests pass.
- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] Windows MSVC release build succeeds.
- [ ] STDIO protocol output is clean.
- [ ] HTTP loopback/authentication protections pass.
- [ ] Token redaction tests pass.
- [ ] No self-bot or user-token functionality exists.
- [ ] Write tools are disabled by default.
- [ ] Mass mentions are disabled by default.
- [ ] Discord rate limits are respected.
- [ ] Documentation matches shipped behavior.

**Release decision:** Do not ship if any security or protocol acceptance criterion is unmet.
