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

- [ ] Add read-enabled and write-enabled flags.
- [ ] Add guild allowlist.
- [ ] Add channel allowlist.
- [ ] Add tool risk classification.
- [ ] Add message length and page-size limits.
- [ ] Add mention sanitization/default-deny behavior.
- [ ] Add concurrency limits.
- [ ] Add destructive-action approval abstraction.
- [ ] Fail closed if approval is unavailable.
- [ ] Add policy unit tests and audit event model.

**Exit criteria:** All policy rules are tested independently of MCP and Discord transport.

## Phase 4 — MCP tools and resources

- [ ] Implement MCP initialization and capability negotiation.
- [ ] Implement central tool registry.
- [ ] Implement strict JSON schemas.
- [ ] Implement `discord_list_guilds`.
- [ ] Implement `discord_get_guild`.
- [ ] Implement `discord_list_channels`.
- [ ] Implement `discord_get_channel`.
- [ ] Implement `discord_get_messages`.
- [ ] Implement `discord_get_message`.
- [ ] Implement member tools if supported by selected permissions/intents.
- [ ] Implement read-only resource URIs.
- [ ] Add optional draft/review prompts.
- [ ] Add schema and tool contract tests.

**Exit criteria:** Read-only tools and resources are discoverable and execute correctly.

## Phase 5 — STDIO transport

- [ ] Integrate MCP SDK STDIO transport.
- [ ] Ensure stdout contains protocol messages only.
- [ ] Route all logs to stderr.
- [ ] Handle stdin closure and Ctrl+C.
- [ ] Test initialize, list tools, call tool, and shutdown.
- [ ] Test launch through a Windows MCP desktop client.
- [ ] Add sample STDIO client configuration to README.

**Exit criteria:** A Windows MCP client can launch and use the server over STDIO without protocol contamination.

## Phase 6 — Streamable HTTP transport

- [ ] Integrate MCP SDK Streamable HTTP transport.
- [ ] Bind to `127.0.0.1` by default.
- [ ] Implement authentication for non-loopback deployment.
- [ ] Validate Host and Origin.
- [ ] Configure body-size and request-time limits.
- [ ] Configure concurrency and session expiry.
- [ ] Add health/readiness endpoint if compatible with deployment design.
- [ ] Test initialization, sessions, notifications, and disconnects.
- [ ] Document TLS/reverse-proxy deployment.
- [ ] Verify unauthenticated public binding is rejected.

**Exit criteria:** Streamable HTTP passes protocol tests and unsafe exposure is blocked.

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
