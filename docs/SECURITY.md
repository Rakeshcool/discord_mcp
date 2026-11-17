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
- **No message-content logging by default.** `redact_message_content = true`;
  turning it off is reported as a startup warning.

## 5. Data handling

- The process does not persist Discord data. There is no cache, database or
  message archive.
- Message bodies are fetched per call and dropped after the response is built.
- Audit records (from the write phase onward) will contain the action, target
  IDs, outcome, timestamp and correlation ID — never content, never credentials.
- Configuration files hold no secrets. Only environment variable **names**
  appear in configuration and in `.env.example`.

## 6. Transport exposure

- STDIO is the default. It has no listening socket at all.
- The Streamable HTTP transport binds `127.0.0.1` by default and refuses to
  start on a non-loopback interface unless `http.allow_non_loopback = true`,
  `http.require_auth = true` and explicit `http.allowed_origins` are configured.
- When HTTP lands it will also enforce `Host`/`Origin` validation, a request body
  cap, per-request timeouts, a concurrency cap and session expiry. CORS is never
  treated as authentication.

## 7. Not yet implemented (tracked, not claimed)

| Control | Phase |
|---|---|
| Discord adapter, rate-limit handling, error mapping | Phase 2 |
| Policy engine (allowlists, classification, limits, audit events) | Phase 3 |
| Read tools, resources, prompts over MCP | Phase 4 |
| Streamable HTTP transport, bearer-token gate for non-loopback | Phase 5 |
| `discord_send_message` with pre-flight policy and audit record | Phase 6 |
| Correlation IDs on every tool execution | Phase 6 |
| Release gate: full test suite, checksums, packaging | Phase 7 |

## 8. Reporting

A security issue in this server should be handled as a private report to the
maintainer of the deployment. Do not open a public issue containing a token, a
message body or a reproducible exploit against a live guild.
