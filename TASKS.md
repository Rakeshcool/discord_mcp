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

Met against mocks: 22 adapter tests run the real twilight client over a local
socket (`tests/integration`). No live test bot was used, so live-endpoint
behaviour rests on the documented API contract rather than an observed call.

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
`POLICY_BLOCKED`); successful Discord reads are covered by the adapter and gate
tests, not end to end, because no live bot is used.

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
- [ ] Configure HTTP-level concurrency. *(`policy.max_concurrent_requests` caps
      Discord calls, which is where the expensive work is; the HTTP listener
      itself has no connection or in-flight-request cap)*
- [ ] Add health/readiness endpoint if compatible with deployment design.
      *(not added: `/mcp` is the only route, and no deployment design asked for
      one)*
- [x] Test initialization over HTTP, including the refused cases.
- [ ] Test sessions, notifications, and disconnects over HTTP.
- [ ] Document TLS/reverse-proxy deployment. *(one paragraph in the README warns
      that TLS is not implemented; a full proxy deployment guide is outstanding)*
- [x] Verify unauthenticated public binding is rejected.

**Exit criteria:** Streamable HTTP passes protocol tests and unsafe exposure is blocked.

Met for exposure: process tests prove that a request with no token, a wrong token,
a foreign `Host` and a foreign `Origin` are all refused, that a valid token is
served, and that a non-loopback bind without its bearer token exits `2` before it
binds. Session lifecycle (resumption, notifications, disconnect cleanup) is not
yet covered end to end.

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

- [ ] Add correlation IDs and tracing spans.
- [ ] Redact authorization headers and secrets unconditionally.
- [ ] Ensure message content is not logged by default.
- [ ] Add log rotation if file logging is enabled.
- [ ] Add actionable startup diagnostics.
- [ ] Add graceful shutdown and cancellation.
- [ ] Test malformed API responses and network interruptions.
- [ ] Test ambiguous write outcomes and no blind retries.
- [ ] Review dependency vulnerabilities and update policy.

**Exit criteria:** Operational diagnostics are useful without leaking credentials or message content.

## Phase 9 — Documentation and release packaging

- [ ] Complete README setup guide.
- [ ] Document Discord Developer Portal bot creation.
- [ ] Document OAuth2 installation and least-privilege permissions.
- [ ] Document environment-variable and Credential Manager options.
- [ ] Document STDIO configuration for Windows MCP clients.
- [ ] Document Streamable HTTP configuration and secure exposure.
- [ ] Complete `docs/SECURITY.md`.
- [ ] Complete `docs/TOOLS.md`.
- [ ] Complete `docs/DEPENDENCIES.md`.
- [ ] Add CHANGELOG.
- [ ] Build release executable for Windows x64.
- [ ] Generate SHA-256 checksum.
- [ ] Test clean installation on a Windows machine.
- [ ] Tag first release.

**Exit criteria:** A new operator can configure a bot, install the server, connect an MCP client, and use the documented tools safely.

## Phase 10 — Release gate

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
