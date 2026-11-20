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
| Bot token | Full control of the bot: sends, edits and deletes messages, restructures channels and roles, reads allowlisted channels |
| Discord message content | Private user-generated content, potentially sensitive |
| Guild/channel structure and member lists | Membership and social-graph data |
| Invite codes | A live invite is a credential for joining the guild |
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
- **Every mutation class is off by default.** `write_enabled` (send, edit,
  reactions, pins) and `control_enabled` (message deletion, channels, overwrites,
  threads, invites) each require a non-empty channel allowlist;
  `moderation_enabled` (nicknames, kicks, bans, unbans, timeouts) and
  `administrative_enabled` (bulk delete, roles) require a non-empty guild
  allowlist. Validation refuses a class flag without its allowlist, so a
  half-enabled deployment stops at startup. Parsed `@everyone`/`@here`, role and
  user mentions cannot be enabled at all (`allow_mass_mentions = true` is
  rejected).
- **Destructive tools are class-gated, not absent.** Delete, channel, thread,
  invite and overwrite tools are registered and run only when their class flag,
  allowlist, budget and audit path all pass. `policy.protected_channels` and
  `policy.protected_roles` add a second stop that the allowlist cannot widen:
  protected channels refuse edit, deletion, permission overwrites and thread
  edits, protected roles refuse edit and deletion, and both are checked before
  the budget so a refusal costs no action. The set stays bounded: moderation
  tools (nicknames, kicks, bans, unbans, timeouts) are class-gated under
  `moderation_enabled`; webhooks, emoji management, guild settings, automod and
  channel/role reordering are not exposed.
- **There is no approval prompt, and the server does not claim one.** MCP
  elicitation is optional client UI: many clients do not declare it, and newer
  protocol revisions forbid server-to-client requests, so a fail-closed prompt
  would silently disable every destructive tool. What bounds a destructive call
  is the class flag, the allowlist, the rolling budget, `dry_run` as the plan
  step and the audit trail — not consent it did not obtain. The configuration
  has no setting that promises otherwise.
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
  read, write, destructive, moderation or administrative; an unclassified name
  is a `POLICY_BLOCKED` denial, never an implicit permission.
- **Limits are authority limits.** A page size above
  `policy.max_messages_per_read` is an `INVALID_ARGUMENT` error rather than a
  silent truncation, and message bodies are measured in characters against
  `policy.max_message_characters`. Embed count, total embed text and every
  embed field are bounded before a send; uploaded files are bounded per file and
  in total.
- **File uploads are opt-in and path-scoped.** `policy.attachment_root` is unset
  by default, which refuses every file upload. When set, tool paths must be
  relative, are canonicalised against the root and may not resolve outside it;
  directories and non-regular files are refused. File bytes are read only for the
  request and are never logged or returned.
- **Mentions are neutralised, not trusted.** `sanitize_mentions` breaks
  `@everyone`, `@here`, user and role mentions with a zero-width space; the
  write path composes this through `Policy::prepare_outgoing_message`.
- **Concurrency is capped by backpressure.** `policy.max_concurrent_requests`
  bounds in-flight Discord requests with a semaphore. Waiting cannot fail open:
  a caller only proceeds when a slot frees.
- **Every tool call is correlated.** Each call gets an id in an `mcp.tool`
  span and in a task-local; each Discord request inside it opens a `discord.call`
  span, and every audit record carries the calling id. Two requests from one call
  can be joined without recording arguments or content. HTTP requests carry their
  own `http.request` id.
- **Audit records are content-free.** Each policy decision produces an
  `AuditEvent` (action, target IDs, outcome, category) through an injectable
  `AuditSink`; the shipping sink logs to stderr, denials at `info`. There is no
  field in which a message body or token could be recorded.
- **Twenty-nine mutations, all off by default.** The tools are registered
  unconditionally so a client can see the capability and why it is refused; the
  gate rejects each while its class flag is false. Argument schemas use
  `deny_unknown_fields`, so an unknown argument is refused rather than ignored,
  and the read-only hint is truthful per tool: `true` for the eleven reads,
  `false` for every mutation.
- **Member moderation is bounded and local-first.** Nicknames are 1–32
  characters, reasons at most 512 characters and free of header-breaking
  newlines/NUL, ban message deletion is at most 7 days and timeouts at most 28
  days. The bot refuses to target itself. These checks run before the budget and
  before any request; Discord's permission and hierarchy checks remain
  authoritative. Live moderation checks exercised nickname set/clear, timeout
  set/clear, kick, ban and unban against a consenting non-owner.
- **Role hierarchy is authoritative for role tools.** The bot can create, edit,
  assign or delete only roles below its highest role. It refuses to edit or
  delete that highest role or remove it from itself. It cannot move its own
  managed role above a higher role, so an owner must reorder the roles in
  Discord before a positive role test against that higher role can succeed;
  otherwise Discord returns `403` / `50013`. Positive role edit, assign, remove
  and delete remain unverified for this reason.
- **An outgoing message is checked locally before any request.** Write flag,
  channel allowlist, optional text, embed bounds, file scope/count/bytes and
  mention neutralisation are decided in policy from local configuration; the
  content the adapter receives is the neutralised text, and the request also
  carries an empty mention parse list. The same content rules run for an edit,
  so an edit cannot smuggle a mention or an oversized body past policy. A policy
  refusal means the adapter was never called, which the tests assert directly.
- **Every real mutation attempt leaves exactly one audit record.** `allowed`,
  `denied` (policy refusal) or `failed` (Discord rejected an allowed attempt,
  with its category) — action, target IDs, outcome and correlation id, never the
  content. Member moderation records the target user ID alongside the guild.
  Reads record allowances and policy denials; a read that Discord rejects is an
  error result without a separate audit record. A dry run records nothing,
  because nothing was attempted.
- **Mutations are budgeted, and a restart does not reset the budget.**
  `policy.autonomy.max_actions_per_minute` (default 30) is a rolling 60-second
  window shared by every mutation, `discord_send_message` included; the action
  past it is `POLICY_BLOCKED` and no request is built. Sends also draw on a
  per-channel window, `policy.autonomy.max_sends_per_channel_per_minute`
  (default 5), so one channel cannot absorb the shared allowance. The windows are
  written to `policy.autonomy.budget_state_file` after each consumed action; an
  unreadable or malformed file stops startup rather than silently starting an
  empty window, and a write failure refuses the action. A process with mutations
  enabled holds an exclusive ownership lock on that file for its lifetime, so a
  second process cannot read and overwrite the same window: it fails startup and
  must be given a distinct `budget_state_file`. Read-only processes do not open
  the file. `policy.autonomy.require_dry_run` can additionally force named tools
  to be planned before a real call.
- **A dry run cannot touch Discord.** It runs every argument and local policy
  check, then returns `{"dry_run":true,"action":"…"}` before the budget and
  before any request. That is also its limit: a dry run cannot detect a missing
  Discord permission or a role-hierarchy problem; the first real call is where
  that `403` appears.
- **The bot cannot lock itself out.** Editing or deleting the bot's own highest
  role, or removing that role from the bot, is refused locally and
  unconditionally; there is no setting that disables the guard. Member
  moderation also refuses to target the bot itself. `@everyone` is refused for
  delete, assign and remove. The guard costs no budget but does use
  the concurrency cap, and it runs only for a real attempt because it needs
  Discord lookups.
- **Structure is scoped by resolution.** A new channel's parent must be an
  allowed channel; a thread is checked through its parent channel; an invite is
  resolved to its channel (and guild, under a guild allowlist) before the budget
  is spent. A channel or thread created by a tool is not added to the allowlist,
  so nothing may then modify it until an operator widens the list.
- **Bulk delete is bounded before the request.** 2–
  `policy.autonomy.max_bulk_delete` distinct IDs, none older than 14 days, all
  computed from the snowflakes locally. Discord's own limits still apply.
- **A mutation is never retried after an ambiguous outcome.** A `POST` retried
  after a timeout, a reset or a `5xx` may already have applied, so the adapter
  makes one attempt; a `429` is safe to resend because Discord rejected that
  attempt, and twilight's limiter handles it.
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
- Audit records contain the action, target IDs, outcome, error category and the
  correlation id of the tool call that produced them — never content, never
  credentials. Allowances log at `debug`, policy denials at `info`, failures of
  allowed attempts at `warn`; the timestamp comes from the log line. Nothing is
  persisted: there is no durable audit store in this build.
- An invite code returned by `discord_create_invite`, and message text returned
  by the read tools, are credentials/content leaving through a tool result. They
  are not logged, but the client receives them and must treat them accordingly.
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

## 7. Out of scope

| Control | Why |
|---|---|
| Draft/review prompts | optional in the specification; no draft or review workflow to attach them to |
| Host-level approval prompt | MCP elicitation is optional client UI and is not a gate this build relies on; the shipped approval model is class flag + allowlist + budget + `dry_run` + audit |
| Guild settings, automod, webhooks, emoji management | no tool exists for them in this build |
| Channel or role position/reordering | no tool; Discord's role hierarchy still governs role changes |

Everything else in this document describes shipped behaviour.

## 8. Release integrity

- Use the GitHub release page for the tag, commit and any published SHA-256.
  Verify the downloaded executable against the checksum for that same release.
- The crate/package version remains `0.1.0`; release tags may be feature-qualified
  rather than version bumps.
- CI performs a release build but does not upload an artifact; publication is
  manual.

## 9. Reporting

A security issue in this server should be handled as a private report to the
maintainer of the deployment. Do not open a public issue containing a token, a
message body or a reproducible exploit against a live guild.
