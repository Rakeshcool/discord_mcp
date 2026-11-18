# Changelog

Notable changes to `discord-mcp`, grouped by capability rather than by commit —
`TASKS.md` is the unit of work in this project.

## v0.1.0 — 2026-11-18

**Published as GitHub release `v0.1.0`.** The tag names the release commit, and
`dist/discord-mcp-0.1.0-x86_64-pc-windows-msvc.sha256` records the SHA-256 of the
executable built from it. The release attaches `discord-mcp.exe` and that
checksum file; the executable is an attachment, never a file in the repository.
Phase 10 of `TASKS.md` is the release gate, signed off against this artifact, and
GitHub's own asset digest matches the committed hash.

This build reads Discord through an official bot identity and speaks MCP over
STDIO and Streamable HTTP. Its one write tool posts a message, only to allowlisted
channels, and only when writes are deliberately enabled.

### Discord access

- REST client for Discord API v10 built on `twilight-http`, behind an
  application-owned `DiscordApi` trait so twilight types never leave the adapter.
- Guild listing and lookup, channel listing and lookup, bounded message history
  with paging cursors, single-message lookup, and member listing/lookup (subject
  to the Server Members intent).
- Page sizes bounded by policy and by Discord's documented maxima; an
  out-of-range policy limit is an `INVALID_ARGUMENT` rather than a silent
  truncation, and a full page returns `next_cursor` for the next call.
- Per-request timeouts, a bounded retry budget with jittered backoff for
  transient failures only (network errors, timeouts, 5xx), and rate limiting plus
  `429`/`Retry-After` handling from twilight's bucket limiter.
- Discord HTTP failures mapped to stable internal categories, so a tool result
  never surfaces a raw status code as the only explanation.

### Policy and safety

- Read and write feature flags, guild and channel allowlists, and tool risk
  classification that is deny-by-default: an unclassified tool name is a
  `POLICY_BLOCKED` denial.
- Policy enforced in front of the client by `PolicyGate`; the application state
  holds only the gated handle, so no tool path reaches Discord without passing
  allowlists, limits and the concurrency cap.
- `policy.max_concurrent_requests` caps in-flight Discord calls with a semaphore
  that queues rather than fails open.
- Mention neutralisation (`@everyone`, `@here`, user and role mentions) and
  rejection of `policy.allow_mass_mentions = true` in configuration validation.
- Content-free audit records (action, target IDs, outcome, category) through an
  injectable sink, with no field that could hold a message body or a token.
- Fail-closed approval abstraction for destructive actions. No destructive tool
  exists, so nothing can reach it yet.

### MCP surface

- Eight read tools: `discord_list_guilds`, `discord_get_guild`,
  `discord_list_channels`, `discord_get_channel`, `discord_get_messages`,
  `discord_get_message`, `discord_list_members`, `discord_get_member`.
- `discord://guilds` resource for clients that prefer resources over tools.
- Strict argument schemas (`deny_unknown_fields`) and `readOnlyHint` annotations.
- Denials and Discord failures returned as `isError` tool results whose text
  begins with a stable error code, so a model can act on them.
- Guild listings filtered to allowlisted guilds, so a narrow allowlist cannot be
  inferred from the listing.

### Transports

- **STDIO** (default): stdout carries MCP frames only, every diagnostic goes to
  stderr, and the server stops when the client closes stdin.
- **Streamable HTTP** at `/mcp`: loopback bind by default, bearer token compared
  in constant time, `Host` validated against the configured bind, and a request
  carrying `Origin` refused unless it is listed in `http.allowed_origins`.
- `http.max_request_bytes` caps request bodies; `http.request_timeout_seconds`
  bounds a non-`GET` request from arrival to the last byte of its response body
  (the long-lived `GET` event stream is exempt); `http.session_expiry_seconds`
  replaces the SDK's default idle-session timeout.
- `transport = "both"` runs STDIO and HTTP in one process over the same state, so
  the two transports cannot enforce different policy.
- A non-loopback bind is refused unless `http.allow_non_loopback = true`,
  `require_auth = true` and a non-empty `allowed_origins` are configured — and
  the transport re-checks the bind before listening.
- `GET /health` answers status, server, version and transport behind the same
  bearer gate, makes no Discord call, and stays available while the server is
  saturated.
- `http.max_in_flight_requests` caps non-`GET` requests; one beyond the cap is
  refused with `503` and `Retry-After` rather than queued, and a streaming
  response holds its slot until its last byte.

### Writes

- `discord_send_message`: posts one message to a channel on
  `policy.channel_allowlist`. Disabled by default (`policy.write_enabled =
  false`), and validation refuses to enable it without a non-empty allowlist.
- The content passes policy before the request exists: non-empty, within
  `policy.max_message_characters`, and mention-neutralised. The request also
  carries `allowed_mentions: {parse: []}`, so mentions are disabled twice over.
- Each attempt produces exactly one audit record — `allowed`, `denied` (policy)
  or `failed` (Discord rejected it, with the category) — carrying action, channel
  and outcome, never the body.
- Writes are never retried: a `POST` retried after an ambiguous failure could
  duplicate the message. A `429` is still waited out and resent, because Discord
  rejected that attempt.
- The tool is registered even while disabled, so a client sees the capability and
  the `POLICY_BLOCKED` reason instead of a tool that silently appears on some
  deployments.

### Observability

- Every tool call gets a correlation id, carried in an `mcp.tool` span and in a
  task-local; each Discord request inside it opens a nested `discord.call` span,
  and every audit record carries the id of the call that produced it. HTTP
  requests carry their own `http.request` id.
- Ctrl+C now drains the HTTP listener: it stops accepting, finishes in-flight
  requests (each already bounded by the request deadline) and logs the stop.
- A connection dropped mid-body is covered by a test and fails as
  `NETWORK_ERROR` without a retry; a write that times out or gets a `500` is sent
  exactly once, so an unknown outcome can never become a duplicate message.

### Security

- Bot identity only: no self-bot path, no user token, no gateway connection.
- The bot token is read once from the environment variable named by
  `discord.token_env`, shape-validated, wrapped in a zeroizing secret with no
  `Display`, and never logged, returned or written to a file.
- Structural redaction of `Bot <token>` and `Bearer <token>` values and of
  `authorization`, `x-api-key` and `token` keys, applied to the stderr writer and
  the panic hook; `clippy::print_stdout` is denied crate-wide.
- Configuration validation rejects unknown fields, out-of-range limits,
  non-snowflake allowlist entries, plaintext non-loopback API bases and any API
  version other than `v10`.

### Configuration and operations

- Compiled-in defaults in `config/default.toml`, overridden by `--config <PATH>`
  or `DISCORD_MCP_CONFIG`; an unparsable or invalid file is a startup failure,
  never a silent fallback.
- CLI with `serve`, `--validate-config`, `--version` and `--help`; all output on
  stderr, with exit codes `0` success, `1` internal, `2` configuration, `3`
  authentication, `4` policy refusal.
- `--validate-config` reports the configuration source, transport, token presence
  and effective exposure (bind, authentication, write state) without starting
  anything.
- Secrets come from the environment only; `.env.example` lists variable names and
  the server never reads `.env` itself.

### Release packaging

- Annotated tag `v0.1.0` on the release commit, pushed to `origin`, with a GitHub
  release named `discord-mcp v0.1.0` whose assets are `discord-mcp.exe`
  (11,625,984 bytes) and `discord-mcp-0.1.0-x86_64-pc-windows-msvc.sha256`.
- `dist/discord-mcp-0.1.0-x86_64-pc-windows-msvc.sha256` records the SHA-256 of
  the release executable, and the build is byte-reproducible, so the file
  verifies a rebuild instead of identifying one build. `build.rs` passes
  `/Brepro` — without it the MSVC linker stamps the link time and a random PDB
  GUID into every executable — and `scripts/build-release.ps1` remaps the
  absolute Cargo-registry and workspace paths that dependency panic messages
  embed. `docs/SETUP.md` §8 rebuilds and compares. Reproducibility is scoped to
  one checkout: a build from a different directory differs, because Cargo hashes
  the package's absolute path into crate metadata and that reorders symbols.
- The release build was verified from a clean checkout of the tag
  (`git worktree`, `cargo build --release --locked`), then `--validate-config` and
  a STDIO handshake from that tree — the guide's §3 path needs no file that is
  not committed.

### Documentation

- `docs/TOOLS.md`: per-tool reference — arguments, result JSON, annotations,
  policy preconditions, error codes, retry and timeout rules, the
  `discord://guilds` resource and an explicit list of what the surface does not
  do.
- `docs/SETUP.md` §4: where the bot token can live — client `env` block, shell
  variable, `.env`, `setx`, and Windows Credential Manager read into the
  environment by a launcher — with each option's trade-off. Native Credential
  Manager lookup is documented as not implemented (`SPEC.md` lists it as
  optional), including how to remove a stored credential.
- `docs/SETUP.md` §8: artifact-integrity check against the committed checksum.
- `docs/SETUP.md`: install-to-first-call guide — Discord application, bot token,
  privileged intents, least-privilege install URL, installation, secrets,
  configuration, both transports, verification, troubleshooting, removal.
- `docs/SECURITY.md`: identity model, assets, adversaries, implemented controls,
  data handling, transport exposure and an explicit not-yet-implemented table.
- `docs/DEPENDENCIES.md`: pinned crate set with the reason for each choice, the
  rmcp and twilight facts verified from vendored source, and the v1 scopes,
  permissions, intents and endpoints.
- Message text requires Discord's **Message Content** intent, and Discord blanks
  `content`, `embeds` and `attachments` in REST responses without it. This is
  documented rather than worked around, including how to confirm the toggle from
  the application's `flags`.

### Verified

- 151 tests pass: 84 unit, 30 integration (the real twilight client against a
  local mock Discord over a socket), 24 protocol (the real binary as a child
  process over stdio, plus HTTP refusals, session lifecycle and a dropped event
  stream), 13 library unit tests.
- `cargo fmt --all --check`, `cargo clippy --all-targets --all-features --
  -D warnings` and `cargo build --release` (`x86_64-pc-windows-msvc`) pass.
- `cargo-audit 0.23.0` against the committed `Cargo.lock`: 204 crates, 0
  vulnerabilities, 1 unmaintained-crate warning (`chrono`, reached through
  `rmcp`/`schemars`, no fixed version). Recorded in `docs/DEPENDENCIES.md`.
- Release build from a clean checkout of the tag: `cargo build --release
  --locked` on a fresh `git worktree`, then `--validate-config` and the STDIO
  handshake from that tree. Not yet installed on a second machine.
- SHA-256 of the release executable recorded in `dist/`. Repeated builds of the
  tag hash to the committed value, including a build from an empty `target/`; a
  build from a different directory does not, which the docs state.
- Release gate (Phase 10) signed off against the clean release artifact: 151
  tests, `fmt`, Clippy, a from-scratch MSVC release build, protocol-clean STDIO
  output, the HTTP refusals (`401`, `401`, `403`, `403`), writes and mass mentions
  off by default, rate-limit handling, and documentation re-checked against the
  code. Nothing is published.
- Read tools exercised end to end against a real guild over both transports,
  including non-empty message text after enabling the intent, and the HTTP gate
  checked by hand (`401`, `401`, `403`, `403`, `200`, `202`, `400`).

### Not in this build

- More write operations (Phase 7): the send tool exists, but nothing can edit,
  delete, react or bulk-delete, and it stays off unless deliberately enabled.
- No TLS termination; a non-loopback bind belongs behind a proxy that terminates
  TLS, and `docs/SETUP.md` §7 documents the settings it must use.
- Native Windows Credential Manager lookup: the token comes from the
  environment, and a launcher that reads the OS credential store is the supported
  alternative (`SPEC.md` lists a `keyring`-backed store as optional).
- Session resumption (event replay after a reconnect) is unverified; creation,
  acknowledgement, termination and disconnect recovery are covered.
- No installer or MSI: the release attaches a single portable `.exe`, and there
  is no auto-update path.
