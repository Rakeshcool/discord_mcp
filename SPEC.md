# Discord MCP Server — Production Specification

**Status:** Implementation specification  
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
- Moderation actions that require privileged permissions unless explicitly enabled in configuration.
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
   - Tool allowlist.
   - Guild/channel allowlists.
   - Read/write separation.
   - Confirmation policy for destructive actions.
   - Message and attachment size limits.

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
| `discord_get_messages` | Read recent messages from a channel | Bounded page size; only accessible channels |
| `discord_get_message` | Fetch one message by channel and message ID | Verify channel access |
| `discord_list_members` | List members where supported | Require necessary intent/API permissions; bounded pagination |
| `discord_get_member` | Fetch member metadata | Respect visibility and permissions |

### 6.2 Write tools

| Tool | Purpose | Default policy |
|---|---|---|
| `discord_send_message` | Send a message to a configured text-capable channel | Disabled until explicitly enabled; require channel allowlist |
| `discord_edit_own_message` | Edit a message authored by the bot | Disabled by default; own-message check |
| `discord_delete_own_message` | Delete a message authored by the bot | Disabled by default; explicit confirmation |
| `discord_add_reaction` | Add a reaction to a message | Disabled by default; channel allowlist |

Do not include bulk deletion, mass mentions, mass role changes, bans, kicks, or arbitrary permission overwrites in v1. They substantially increase abuse and accidental-impact risk. If later added, they require dedicated policy controls, explicit confirmation, and audit records.

### 6.3 Tool schema rules

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
require_confirmation_for_destructive = true
guild_allowlist = []
channel_allowlist = []
max_messages_per_read = 100
max_message_characters = 2000
allow_mass_mentions = false

[logging]
redact_message_content = true
log_tool_arguments = false
```

The API version must be reviewed against the current official Discord API documentation at release time. Do not silently change the API version without tests.

### 9.1 Secret handling

- Primary development option: `DISCORD_BOT_TOKEN` environment variable.
- Production Windows option: Windows Credential Manager via an OS credential store library.
- Config files must contain only the name of the environment variable or credential key, never the token.
- Redact secrets from Debug output, errors, telemetry, crash reports, and logs.
- Never write tokens to logs or MCP tool results.

## 10. Permission and approval model

Implement defense in depth:

1. **Discord permissions:** Discord remains the final authority. The bot must have the permissions required for each action.
2. **Server policy:** Local allowlists and read/write toggles.
3. **Tool policy:** Each tool declares read-only, write, or destructive classification.
4. **Approval policy:** Destructive tools require an explicit confirmation mechanism supported by the MCP host or are disabled.
5. **Input policy:** Validate IDs, lengths, ranges, mention behavior, and target guild/channel.
6. **Audit policy:** Record action name, target IDs, outcome, timestamp, and correlation ID without recording secrets or unnecessary content.

Do not pretend that an MCP server-side boolean is equivalent to user confirmation. The approval UX must be provided by the host or an explicit operator-controlled policy. If the host cannot provide a trustworthy approval flow, keep the relevant tool disabled.

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
- Default to not logging message content or tool arguments.
- Offer an opt-in diagnostic mode that still redacts secrets and warns before logging message content.
- Include health/readiness information for HTTP deployment without exposing secrets.
- Never send diagnostics to STDIO stdout.

## 14. Key design rule

MCP is the interface, not the permission system. Discord permissions, local
policy, and explicit approval must all be enforced before a write operation is
sent. The server must fail closed when policy or authorization state is unclear.
