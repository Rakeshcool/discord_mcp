# Security

This document is the operator-facing security model. It states what the server
protects, what it assumes, and which controls exist today versus which are still
to land. Nothing here is aspirational without being labelled.

## 1. Identity model

The server acts only as an **official Discord bot application**:

- it authenticates with a bot token,
- it never accepts, requests or stores a Discord user password or user token,
- it has no self-bot path and no code path that drives the Discord desktop
  client or the user gateway,
- it cannot grant itself permissions; the bot's Discord permissions and the
  channel overwrites remain the final authority.

`BotToken` (`src/security/secrets.rs`) is the only representation of the secret.
It has no `Display`, its `Debug` prints `BotToken(<redacted>)`, it lives in a
`secrecy::SecretString` that zeroizes on drop, and `expose()` is called only to
build an `Authorization` header.

## 2. Assets

| Asset | Why it matters |
|---|---|
| Bot token | Full control of the bot: sends messages, reads allowlisted channels |
| Discord message content | Private user-generated content, potentially sensitive |
| Guild/channel structure and member lists | Membership and social-graph data |
| Local configuration | Allowlists and limits, i.e. the effective authority boundary |
| Host CPU/network | An unauthenticated HTTP endpoint is a local attack surface |

Non-assets: there is no user credential store, no cookie jar, no cached Discord
session, and no database — the process holds nothing beyond the current call.

## 3. Adversaries and threats

1. **A prompt-injected model.** Discord message content is attacker-controlled
   text. It is treated as data, never as instructions, and it cannot widen the
   server's authority: policy is evaluated in the application layer from local
   configuration, not from tool arguments.
2. **A malicious or careless operator.** Config mistakes must not silently
   create a dangerous deployment, so validation fails closed (see §5).
3. **A network attacker.** Anything bound beyond loopback is reachable by
   others on the network; plaintext or unauthenticated HTTP is therefore
   refused rather than warned about.
4. **A local, unprivileged process.** A loopback HTTP port is reachable by any
   local user, plus any web page in a browser via DNS rebinding, which is why
   `Host` and `Origin` are validated independently of authentication.
5. **Log readers and crash reports.** Tokens and message bodies must not reach
   logs, error strings or stack traces.

## 4. Implemented controls (this build)

- **stdout is protocol-only.** Logs go to stderr through a redacting writer, the
  panic hook writes to stderr, and `clippy::print_stdout` is denied
  crate-wide, so a stray `println!` fails the build.
- **Token handling.** Read once from the environment variable named by
  `discord.token_env`; shape-validated; never logged, never returned by a tool,
  never written to a file. Placeholder values, whitespace and `Bot `-prefixed
  values are rejected.
- **Redaction, structurally.** `redact()` removes `Bot <token>` and
  `Bearer <token>` shaped values and the values of `authorization`, `x-api-key`
  and `token` keys, whatever the secret is. Nothing is registered with the
  redactor, so it never holds a second copy of a secret.
- **Fail-closed configuration.** Unknown fields, out-of-range limits, non-loopback
  binding without an explicit opt-in, non-loopback binding without
  authentication, missing `allowed_origins` for non-loopback binding, plaintext
  Discord API base URLs, a non-`v10` API version and non-snowflake allowlist
  entries are all startup errors.
- **Writes off by default.** `policy.write_enabled = false`, and enabling it
  requires a non-empty channel allowlist. Parsed `@everyone`/`@here`, role and
  user mentions cannot be enabled at all (`allow_mass_mentions = true` is
  rejected).
- **No destructive tools exist.** Delete and edit tools are not registered in
  v1. There is no trustworthy MCP approval channel available, so the safe
  implementation is the absent one.
- **Policy is enforced, not advisory.** `PolicyGate` wraps the Discord client and
  `AppState` stores only the wrapped handle, so no tool can reach Discord without
  passing the allowlists, feature flags, page-size ceiling and concurrency cap.
  The gate resolves a channel's guild (one extra request) before a channel-scoped
  read when `policy.guild_allowlist` is non-empty, and refuses a channel that
  belongs to no guild in that configuration.
- **Allowlists narrow visibility too.** `discord_list_guilds` filters its page to
  allowlisted guilds, so a guild allowlist cannot be inferred from the listing.
  The opaque pagination cursor is left untouched so paging still advances.
- **Risk classification is deny-by-default.** Every tool name is classified as
  read, write or destructive; an unclassified name is a `POLICY_BLOCKED` denial,
  never an implicit permission.
- **Limits are authority limits.** A page size above
  `policy.max_messages_per_read` is an `INVALID_ARGUMENT` error rather than a
  silent truncation, and message bodies are measured in characters against
  `policy.max_message_characters`.
- **Mentions are neutralised, not trusted.** `sanitize_mentions` breaks
  `@everyone`, `@here`, user and role mentions with a zero-width space; the
  write path composes this through `Policy::prepare_outgoing_message`.
- **Concurrency is capped by backpressure.** `policy.max_concurrent_requests`
  bounds in-flight Discord requests with a semaphore. Waiting cannot fail open:
  a caller only proceeds when a slot frees.
- **Audit records are content-free.** Each policy decision produces an
  `AuditEvent` (action, target IDs, outcome, category) through an injectable
  `AuditSink`; the shipping sink logs to stderr, denials at `info`. There is no
  field in which a message body or token could be recorded.
- **The MCP surface is read-only and strict.** Eight read tools and one resource
  (`discord://guilds`) are registered; no write or destructive tool exists in
  this build. Argument schemas use `deny_unknown_fields`, so an unknown argument
  is refused rather than ignored, and every tool is annotated `readOnlyHint`.
- **HTTP authenticates before it serves.** The Streamable HTTP transport resolves
  its bearer token from `http.auth_token_env` before binding, so a server that
  cannot authenticate its callers never opens a socket. The token is compared in
  constant time; a missing, malformed or wrong `Authorization` header all produce
  the same `401`, so nothing is learned from the response. The token is a separate
  credential from the bot token and never reaches Discord.
- **`Host` and `Origin` are validated, and are not authentication.** `Host` must
  be the configured bind (plus `localhost`/`127.0.0.1`/`::1` spellings when the
  bind is loopback), which is what stops a DNS name resolving to `127.0.0.1` from
  reaching the endpoint. A request that carries `Origin` must match
  `http.allowed_origins`, and with the default empty list every `Origin`-bearing
  request is refused rather than ignored. Both checks are anti-rebinding and
  anti-CSRF; the bearer token remains the authentication.
- **The listener caps concurrent requests.** `http.max_in_flight_requests`
  bounds non-`GET` requests in flight; one beyond the cap is refused with `503`
  and `Retry-After` rather than queued, because an unbounded queue is the memory
  the cap exists to protect. The permit is attached to the response body, so a
  streaming answer holds its slot until its last byte. `GET` (a session's
  long-lived event stream) and `/health` are exempt: the first would let one
  client starve the rest, the second must answer while the server is saturated.
- **Health is a probe, not a surface.** `GET /health` sits behind the same
  bearer gate and returns only status, server name, version and transport. It
  makes no Discord call and holds no secret.
- **Body size, request duration and session lifetime are bounded.**
  `http.max_request_bytes` caps a request body. `http.request_timeout_seconds`
  bounds each non-`GET` request from arrival to the last byte of its response
  body — a timeout on the handler alone would be a placebo, because the MCP
  service returns an SSE response as soon as it begins streaming and a stalled
  tool call would keep an answered request alive indefinitely. The long-lived
  `GET` event stream is exempt. `http.session_expiry_seconds` is handed to the
  SDK session manager's idle timeout, so an abandoned session is torn down
  instead of retaining a worker. The endpoint exists only at `/mcp`, and `both`
  and `http` share one `AppState` so the two transports cannot enforce different
  policy.
- **Failures are legible, not opaque.** Policy denials and Discord errors are
  returned as `isError` tool results whose text begins with the stable error code
  (`POLICY_BLOCKED`, `INVALID_ARGUMENT`, …), because a protocol error would be
  rendered as an internal failure the model cannot act on. Resources have no such
  flag, so a refused resource read is a request error carrying the same code.

## 5. Data handling

- The process does not persist Discord data. There is no cache, database or
  message archive.
- Message bodies are fetched per call and dropped after the response is built.
- Audit records contain the action, target IDs, outcome and error category —
  never content, never credentials. Timestamps and correlation IDs join them with
  the write phase, which is the first place a durable record is required.
- Configuration files hold no secrets. Only environment variable **names**
  appear in configuration and in `.env.example`.

## 6. Transport exposure

- STDIO is the default. It has no listening socket at all.
- The Streamable HTTP transport binds `127.0.0.1` by default and refuses to
  start on a non-loopback interface unless `http.allow_non_loopback = true`,
  `http.require_auth = true` and explicit `http.allowed_origins` are configured.
  The bind guard is applied twice: configuration validation refuses the
  combination, and the transport re-checks before binding.
- The MCP endpoint is `POST /mcp` (plus the `GET` event stream); `GET /health`
  is the only other route. Every route is behind the same bearer gate.
  Authentication is a token compared in constant time; `Host` and `Origin` are
  validated as described in §4; the body is capped by `http.max_request_bytes`. Process tests prove that requests without a
  token, with a wrong token, with a foreign `Host` and with a foreign `Origin` are
  refused, and that a non-loopback bind missing its token exits with a
  configuration error before listening.
- CORS is never treated as authentication: allowing an origin only means the
  browser preflight is not what blocks a request.
- **TLS is not implemented.** A non-loopback bind is expected to sit behind a
  reverse proxy that terminates TLS; the process serves plain HTTP on its bind
  address. `docs/SETUP.md` §7 documents the proxy settings that work with this
  server's `Host` validation, including the requirement to rewrite `Host` to the
  bind address. `X-Forwarded-*` headers are ignored: there is no trusted-proxy
  mode. In-flight requests are capped by `http.max_in_flight_requests` and
  Discord calls by `policy.max_concurrent_requests`.

## 7. Not yet implemented (tracked, not claimed)

| Control | Phase |
|---|---|
| Draft/review prompts | not scheduled (optional in the specification) |
| `discord_send_message` with pre-flight policy and audit record | Phase 7 |
| Correlation IDs on every tool execution | Phase 8 |
| Packaging: SHA-256 checksum, clean-install test, first tag | Phase 9 |
| Release gate: full test suite, `fmt`, Clippy, acceptance criteria | Phase 10 |

## 8. Reporting

A security issue in this server should be handled as a private report to the
maintainer of the deployment. Do not open a public issue containing a token, a
message body or a reproducible exploit against a live guild.
