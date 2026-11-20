# Discord MCP Server — Production Specification

**Status:** Implementation specification — reflects `feat/user-message-search`
(40 tools, 29 mutations in four policy classes)

**Target:** Windows 11 x64  
**Language:** Rust  
**Transports:** STDIO and MCP Streamable HTTP  
**Application:** Discord  
**Architecture:** Dedicated Discord integration  
**Initial release:** Single-tenant local server, one configured Discord bot identity

## 1. Purpose

Build a secure, production-quality Model Context Protocol (MCP) server that allows an MCP-compatible AI client to interact with Discord through an explicitly configured Discord bot application.

The server is an adapter between MCP clients and the official Discord API. It must use supported Discord bot authentication and APIs. It must not automate a normal user account, use self-bots, scrape private client state, reverse-engineer Discord's desktop application, or bypass Discord permissions, rate limits, or platform restrictions.

## 2. Goals and non-goals

### Goals

- Expose a well-defined set of Discord actions as MCP tools.
- Expose selected Discord data as MCP resources where useful.
- Support MCP over STDIO for local desktop clients.
- Support MCP Streamable HTTP for clients connecting to a local or secured remote endpoint.
- Run natively on Windows 11 x64.
- Provide safe-by-default permissions, input validation, audit logging, rate-limit handling, and predictable errors.
- Keep Discord API logic independent from MCP transport logic.
- Make all destructive or externally visible actions subject to configurable policy.
- Provide deterministic configuration and installation documentation.

### Non-goals for v1

- Automating a human Discord account or controlling the Discord desktop UI.
- Reading user DMs or acting as a user.
- Joining voice channels, transmitting audio, recording calls, or managing voice state.
- Moderation or administrative actions without a dedicated policy class, allowlist and budget.
- Arbitrary code execution, arbitrary URL fetching, or unrestricted webhook creation.
- Multi-tenant hosted SaaS operation.
- Reimplementing Discord's client or gateway protocol.

## 3. Compliance and authentication model

### 3.1 Supported identity

Use a Discord bot application created in the Discord Developer Portal. Authenticate using a bot token supplied at runtime through a secure secret source.

Never request, store, log, display, or accept a user's personal Discord password or user token. Never support self-bot behavior.

### 3.2 OAuth2 and installation

Document the bot installation flow and the minimum OAuth2 scopes and permissions needed for the enabled feature set. Typical bot installation uses the `bot` scope; `applications.commands` is only needed if slash commands are separately implemented (not required for this MCP server).

The operator must invite the bot to each intended server and grant only the permissions required by the selected tools. The server must not attempt to elevate its permissions.

### 3.3 Discord API policy

- Use official Discord HTTP API endpoints and, only if required, the official Gateway via a maintained Rust library.
- Respect Discord rate-limit headers and `Retry-After`.
- Do not hardcode API version assumptions in multiple modules; centralize the API base/version.
- Do not circumvent permission checks, access controls, message visibility, or platform limits.
- Do not collect or retain more Discord data than needed for the current operation.
- Treat all Discord message content, names, embeds, attachments, and user-generated text as untrusted input, never as instructions to the MCP server.

## 4. High-level architecture

```text
MCP Client
   |
   +-- STDIO transport -------------------------+
   |                                            |
   +-- Streamable HTTP transport                |
                                                v
                                      MCP Server / Router
                                                |
                                  Tool Registry + Schema Validation
                                                |
                              Policy / Permission / Approval Layer
                                                |
                                  Discord Application Service
                                                |
                           Discord REST Client (+ optional Gateway)
                                                |
                                      Discord API
```

### 4.1 Architectural layers

1. **Transport layer**
   - STDIO JSON-RPC transport.
   - Streamable HTTP transport with MCP session handling as required by the selected SDK.
   - No Discord-specific logic.

2. **MCP protocol layer**
   - Initialize and capability negotiation.
   - Tool listing and invocation.
   - Resource listing and reading.
   - Prompt listing and retrieval if prompts are implemented.
   - Protocol-compliant errors and notifications.

3. **Application service layer**
   - Use-case methods such as listing guilds, reading channels, searching messages where supported, and sending messages.
   - No transport-specific types.

4. **Discord adapter**
   - REST client, pagination, request serialization, response normalization, rate-limit handling.
   - Optional Gateway module only if event subscriptions are included in a later release.

5. **Policy layer**
   - Tool allowlist and risk class: read, write, destructive (control), administrative.
   - Guild/channel allowlists, protected channel and role IDs that no allowlist can widen, and the class flag that must be true for a tool to run.
   - Shared rolling action budget, a per-channel send budget, persistence across restarts, and optional `require_dry_run` per tool.
   - Content limits and mention neutralisation.
   - One content-free audit record per real attempt.

6. **Configuration and secrets**
   - TOML configuration for non-secret settings.
   - Bot token from environment variable or Windows Credential Manager integration.
   - Never put secrets in command-line arguments or committed files.

7. **Observability**
   - Structured logs to stderr for STDIO mode.
   - Rotating file logs only when configured.
   - Redaction of tokens, authorization headers, and sensitive message content.

## 5. Technology decisions

Use stable Rust and pin compatible dependency versions in `Cargo.lock`.

Suggested crates (verify current compatibility before implementation):
- `rmcp`: Rust MCP SDK, using its supported STDIO and Streamable HTTP transports.
- `tokio`: asynchronous runtime.
- `serenity` or a focused Discord REST client: choose one primary Discord integration library after evaluating current Discord API support and maintenance. Avoid maintaining two competing Discord clients.
- `reqwest`: only if a direct REST implementation is deliberately chosen.
- `axum`: HTTP serving if not supplied by the MCP SDK transport.
- `serde`, `serde_json`: serialization.
- `config` or `toml`: configuration parsing.
- `thiserror`, `anyhow`: typed and contextual errors.
- `tracing`, `tracing-subscriber`: structured diagnostics.
- `secrecy`: secret wrappers and accidental debug redaction.
- `windows` or `windows-sys`: Windows-specific integration only where needed.
- `keyring`: optional Windows Credential Manager-backed token storage.
- `uuid`: correlation/request identifiers where useful.

Do not introduce a dependency merely because it is listed here. Confirm the selected SDK's current API and transport implementation before writing integration code.

## 6. Initial feature scope

The v1 feature set is intentionally narrow and permission-aware.

### 6.1 Read tools

| Tool | Purpose | Important constraints |
|---|---|---|
| `discord_list_guilds` | List guilds visible to the bot | Return only bot-accessible guilds |
| `discord_get_guild` | Get guild metadata | Validate guild ID |
| `discord_list_channels` | List guild channels | Respect bot visibility and permissions |
| `discord_get_channel` | Get channel metadata | Validate channel access |
| `discord_get_messages` | Read recent messages from a channel | Bounded page size; bounded embeds, reactions and attachment metadata; only accessible channels |
| `discord_get_user_messages` | Collect recent messages by one author across visible text channels | Bounded per-channel scan, 50-request global cap; `has_attachments` filter; policy-filtered; no Discord search endpoint |
| `discord_find_abusive_messages` | Scan visible text channels for abuse keywords and group matches by author | Bounded scan, 50-request global cap; conservative defaults or caller-supplied keywords; signals only, never a verdict or automatic action |
| `discord_get_message` | Fetch one message by channel and message ID | Verify channel access |
| `discord_list_members` | List members where supported | Require necessary intent/API permissions; bounded pagination |
| `discord_get_member` | Fetch member metadata | Respect visibility and permissions |
| `discord_get_capabilities` | Report enabled classes, autonomy limits and the authenticated bot | Read-gated like every other read |

### 6.2 Mutation tools

Every mutation is off by default. Its class flag must be true, its scope must be
allowlisted, and every call supports `dry_run` except `discord_send_message`.

| Tools | Class flag | Scope |
|---|---|---|
| `discord_send_message`, `discord_edit_message`, `discord_add_reaction`, `discord_remove_reaction`, `discord_clear_reactions`, `discord_pin_message`, `discord_unpin_message` | `write_enabled` | channel allowlist; sends take text, embeds, files beneath `policy.attachment_root` and a reply target, all locally bounded |
| `discord_delete_message`, `discord_create_channel`, `discord_edit_channel`, `discord_delete_channel`, `discord_set_permission_overwrite`, `discord_delete_permission_overwrite`, `discord_create_thread`, `discord_edit_thread`, `discord_create_invite`, `discord_delete_invite` | `control_enabled` | channel allowlist; a created channel's parent must be allowed, a thread is resolved through its parent, an invite through its channel |
| `discord_set_member_nickname`, `discord_kick_member`, `discord_ban_member`, `discord_unban_member`, `discord_timeout_member`, `discord_clear_member_timeout` | `moderation_enabled` | guild allowlist; nicknames are 1–32 characters, reasons at most 512 characters, bans delete at most 7 days of messages and timeouts last at most 28 days; the bot cannot target itself |
| `discord_bulk_delete_messages`, `discord_create_role`, `discord_edit_role`, `discord_delete_role`, `discord_assign_role`, `discord_remove_role` | `administrative_enabled` | guild allowlist; bulk delete takes 2–`policy.autonomy.max_bulk_delete` distinct IDs younger than 14 days; role tools also need Manage Roles and a role below the bot's highest |

Not exposed: webhooks, emoji management, guild settings, automod, and channel
position changes. Mass mentions remain impossible: `allow_mass_mentions = true`
is rejected by validation and outgoing content is neutralised.

### 6.3 Mutation controls

- **Dry run.** Every mutation except `discord_send_message` accepts `dry_run`.
  It runs every argument and local policy check, returns
  `{"dry_run":true,"action":"…"}` and sends no request, spends no budget and
  writes no audit record. It cannot see a missing Discord permission or a
  hierarchy problem; the first real call is where that `403` appears.
- **Budget.** `policy.autonomy.max_actions_per_minute` (default 30) is a rolling
  60-second window shared by every mutation, including `discord_send_message`.
  The action past it is `POLICY_BLOCKED` and is never sent. Sends also draw on
  `policy.autonomy.max_sends_per_channel_per_minute` (default 5). The windows are
  persisted to `policy.autonomy.budget_state_file` so a process restart cannot
  reset them; an unreadable state file is a startup error. One running process
  owns the file through an exclusive lock and a second instance with mutations
  enabled fails startup; concurrent instances need distinct state files, which
  are independent limits.
- **Member moderation limits.** Nicknames are 1–32 characters, an empty string
  clears one; audit-log reasons are at most 512 characters and reject newlines
  and NUL; a ban's `delete_message_seconds` is 0–604800; a timeout is
  1–2419200 seconds. The bot refuses to moderate itself.
- **Rich send limits.** `policy.max_message_characters` bounds text;
  `policy.max_embeds_per_message` and `policy.max_embed_characters` bound embed
  count and total text; Discord's per-field limits are checked locally.
  `policy.max_attachments_per_message`, `policy.max_attachment_bytes` and
  `policy.max_attachment_total_bytes` bound uploads. Files are disabled unless
  `policy.attachment_root` is set, and every path is canonicalised beneath that
  root before the budget is spent. Reply targets are message IDs in the same
  channel.
- **Required dry runs.** `policy.autonomy.require_dry_run` lists tools that refuse
  a real call; a name that is not a classified tool is a configuration error.
- **Audit.** Each real attempt produces exactly one content-free record
  (`allowed`, `denied` or `failed`) with action, target IDs, outcome, category and
  the correlation id. A dry run produces none.
- **Role guard.** Editing or deleting the bot's own highest role, or removing it
  from the bot, is refused locally and unconditionally; no setting disables it.
  `@everyone` is refused for delete, assign and remove.
- **No ambiguous retries.** A mutation is attempted once. A `429` is safe to
  resend because Discord rejected that attempt; a timeout, reset or `5xx` is not.
- **Session access overrides.** `--access <CLASSES>` enables one or more of
  `read`, `write`, `control`, `moderation` and `administrative` for a process
  without editing the configuration; `all` enables all five, and the flag may be
  repeated or comma-separated. It only enables classes: the existing allowlist
  requirements are revalidated after the override, so a class cannot be enabled
  without its channel or guild allowlist.
- **Session kill switch.** `--read-only` or `DISCORD_MCP_READ_ONLY=1` turns all
  four mutation classes off without editing the configuration, and overrides
  every `--access` grant.

### 6.4 Tool schema rules

Every tool must:
- Have a stable, descriptive name and concise description.
- Declare a strict JSON input schema.
- Reject unknown fields unless the MCP SDK's schema conventions require otherwise.
- Validate snowflake IDs as strings; never parse them into floating-point numbers.
- Apply maximum lengths and page sizes.
- Return structured content plus a concise human-readable summary where supported.
- Never return bot tokens, authorization headers, or internal stack traces.

## 7. MCP resources and prompts

### 7.1 Resources

Potential resources:
- `discord://guilds` — guilds accessible to the bot.
- `discord://guild/{guild_id}` — guild metadata.
- `discord://guild/{guild_id}/channels` — visible channel metadata.
- `discord://channel/{channel_id}/messages/recent` — bounded recent message snapshot.

Resources must be read-only and must not imply that a resource URI grants access beyond the bot's Discord permissions. Avoid caching sensitive message bodies by default.

### 7.2 Prompts

Optional prompts:
- `summarize_channel` — asks the model to summarize supplied channel context.
- `draft_discord_message` — creates a draft only; it does not send.
- `moderation_review` — reviews supplied content and suggests actions; it does not perform moderation.

Prompts must not instruct the model to bypass Discord rules or access restrictions.

## 8. Transport requirements

### 8.1 STDIO

- Support MCP STDIO for local MCP clients.
- Standard output is reserved exclusively for MCP protocol messages.
- All logs and diagnostics go to standard error.
- Do not print startup banners, progress bars, debug output, or panic messages to stdout.
- Support graceful shutdown on stdin closure and process termination.
- Configuration may be supplied via environment variables and a config file path.

### 8.2 Streamable HTTP

- Implement the MCP Streamable HTTP transport supported by the chosen MCP SDK.
- Bind to `127.0.0.1` by default.
- Require explicit configuration to bind to a non-loopback interface.
- When exposed beyond loopback, require authentication and TLS termination at a trusted reverse proxy or equivalent deployment layer.
- Validate `Origin` and Host headers to mitigate DNS rebinding.
- Apply request body limits, timeouts, concurrency limits, and session expiration.
- Do not assume that CORS is authentication.
- Do not expose an unauthenticated endpoint on `0.0.0.0`.
- Document session and stateless operation behavior according to the selected SDK and MCP specification.

### 8.3 Transport selection

Configuration should allow:
- `stdio`
- `http`
- `both`

For `both`, start independent transport tasks over one shared application service and policy state. Ensure tool calls are concurrency-safe.

## 9. Configuration

Example `config/default.toml`:

```toml
[server]
name = "discord-mcp"
version = "0.1.0"
transport = "stdio"
log_level = "info"

[http]
host = "127.0.0.1"
port = 8787
require_auth = true
request_timeout_seconds = 30
max_request_bytes = 1048576

[discord]
api_base = "https://discord.com/api"
api_version = "v10"
token_env = "DISCORD_BOT_TOKEN"
request_timeout_seconds = 20
max_retries = 3

[policy]
read_enabled = true
write_enabled = false
control_enabled = false
moderation_enabled = false
administrative_enabled = false
guild_allowlist = []
channel_allowlist = []
protected_channels = []
protected_roles = []
max_messages_per_read = 100
max_message_characters = 2000
max_embeds_per_message = 10
max_embed_characters = 6000
max_attachments_per_message = 10
max_attachment_bytes = 8388608
max_attachment_total_bytes = 16777216
# attachment_root = "C:\\path\\to\\uploads"  # unset disables file uploads
allow_mass_mentions = false
max_concurrent_requests = 8

[policy.autonomy]
max_actions_per_minute = 30
max_sends_per_channel_per_minute = 5
max_bulk_delete = 50
require_dry_run = []

[logging]
redact_message_content = true
log_tool_arguments = false
```

The API version must be reviewed against the current official Discord API documentation at release time. Do not silently change the API version without tests.

### 9.1 Secret handling

- The server reads `DISCORD_BOT_TOKEN` from the environment. It does not read `.env` itself.
- Native Windows Credential Manager lookup is not implemented in this build. A launcher can read the OS credential store and set the environment variable before starting the server.
- Config files must contain only the name of the environment variable, never the token.
- Redact secrets from Debug output, errors, telemetry, crash reports, and logs.
- Never write tokens to logs or MCP tool results.

## 10. Permission and approval model

Implement defense in depth:

1. **Discord permissions:** Discord remains the final authority. The bot must have the permissions required for each action.
2. **Server policy:** Local allowlists and read/write toggles.
3. **Tool policy:** Each tool is classified read, write, destructive (control) or administrative, and its class flag must be true.
4. **Approval policy:** The server offers no confirmation prompt. Its gate is the class flag plus allowlist plus rolling budget plus audit, with `dry_run` as the plan step; no tool claims an approval it did not get. MCP elicitation is optional client UI and is unavailable on protocol revisions that forbid server-to-client requests, so a fail-closed prompt would silently disable destructive tools. Configuration has no setting that promises one.
5. **Input policy:** Validate IDs, lengths, ranges, mention behavior, target guild/channel, role hierarchy and bulk-delete age/count before any request.
6. **Audit policy:** Record action name, target IDs, outcome, error category and correlation ID, without secrets or content.
7. **Budget policy:** Cap real mutations per rolling minute, share the cap across tools, and let dry runs cost nothing.

Do not pretend that an MCP server-side boolean is equivalent to user confirmation. If a deployment needs human approval, it must come from the host or an operator-controlled policy outside this server; the server's own controls bound blast radius rather than replace consent.

## 11. Error handling

Use stable error categories:
- `CONFIGURATION_ERROR`
- `AUTHENTICATION_FAILED`
- `PERMISSION_DENIED`
- `RESOURCE_NOT_FOUND`
- `INVALID_ARGUMENT`
- `RATE_LIMITED`
- `DISCORD_API_ERROR`
- `NETWORK_ERROR`
- `TIMEOUT`
- `POLICY_BLOCKED`
- `INTERNAL_ERROR`

Return safe, actionable messages. Keep raw diagnostics in redacted logs. Preserve Discord status codes and retry metadata where safe and useful.

### 11.1 Rate limits and retries

- Respect Discord's rate-limit response headers and `Retry-After`.
- Use bounded retries with jitter for retryable network failures and server errors.
- Do not retry non-idempotent writes blindly.
- Never retry a write if the outcome is ambiguous unless an idempotency strategy is available.
- Surface rate-limit delays to the caller rather than spinning indefinitely.
- Apply global and per-route concurrency limits.

## 12. Windows process and lifecycle requirements

- Build and test for `x86_64-pc-windows-msvc`.
- Support native Windows paths and UTF-8 configuration.
- Handle Ctrl+C and process shutdown cleanly.
- Avoid shell invocation for HTTP calls or normal Discord operations.
- If subprocess support is added later, use explicit executable paths and argument arrays; never interpolate untrusted input into a shell command.
- Provide release binaries and a checksum.
- Provide a sample MCP client configuration for STDIO.
- Document Windows Credential Manager setup if implemented.
- Ensure logs and configuration directories follow Windows conventions.

## 13. Observability and diagnostics

- Use structured tracing spans for each MCP request and Discord API operation.
- Attach a correlation ID to each tool execution.
- Redact bot tokens and authorization headers unconditionally.
- Emit one content-free audit record per real mutation attempt (`allowed`,
  `denied`, `failed`) and per read decision; denials at `info`, failures at
  `warn`, allowances at `debug`. Nothing is persisted.
- Default to not logging message content or tool arguments.
- Offer an opt-in diagnostic mode that still redacts secrets and warns before logging message content.
- Include health/readiness information for HTTP deployment without exposing secrets.
- Never send diagnostics to STDIO stdout.

## 14. Key design rule

MCP is the interface, not the permission system. Discord permissions, local
policy, the class flag and its allowlist must all be enforced before a write
operation is sent, and a configured `dry_run` requirement must be satisfied. The
server must fail closed when policy or authorization state is unclear.
