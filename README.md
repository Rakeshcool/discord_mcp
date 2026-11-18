# discord-mcp

A secure Model Context Protocol (MCP) server for Discord, using an **official
Discord bot identity**. Windows 11 x64, Rust, MCP STDIO and Streamable HTTP
transports.

It is not a self-bot, not a user-account automation tool, and not a Discord
client reimplementation. **Start with [`docs/SETUP.md`](docs/SETUP.md)** for the
full install-to-first-call guide (Discord app, intents, permissions, both
transports, client configuration, verification, troubleshooting). See `SPEC.md`
for the full specification, `TASKS.md` for the phased plan, and
`docs/SECURITY.md` for the threat model.

## Status

Through Phase 6 — read-only Discord access over STDIO and Streamable HTTP.
Phases 7–10 (writes, operational hardening, packaging, release gate) are planned
and not started.

Implemented: typed configuration with fail-closed validation, stable error
categories, secret wrapper, structural secret redaction, stderr-only logging,
stdio MCP handshake, CLI (`serve`, `--validate-config`), CI; a twilight-backed
read adapter with timeouts, bounded retries and error mapping; a policy engine
(flags, guild/channel allowlists, page limits, concurrency cap, mention
neutralisation, fail-closed approval, content-free audit records) enforced in
front of the client; eight read tools plus the `discord://guilds` resource; and a
Streamable HTTP transport with loopback defaults, a bearer-token gate, `Host` and
`Origin` validation, request-body caps, a request deadline that also covers the
response body, idle-session expiry and per-session isolation.

Not implemented yet:

- **Writes (Phase 7).** No tool writes to Discord: no send, edit, delete or
  reaction, and no destructive operation.
- **HTTP gaps (Phase 6).** No health endpoint and no HTTP-level concurrency cap
  (Discord calls are already capped by `policy.max_concurrent_requests`). Session
  resumption, notification delivery and disconnect cleanup are unverified.
- **Deployment (Phase 6).** TLS termination is not implemented and no
  reverse-proxy guide exists; put a proxy that terminates TLS in front of a
  non-loopback bind.
- **Hardening (Phase 8).** No correlation IDs on tool executions, no graceful
  drain of the HTTP listener (STDIO stops when the client closes stdin), and no
  dependency-audit policy.

## Build and test

```powershell
cargo build
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Windows MSVC is the only supported target (`x86_64-pc-windows-msvc`). No WSL or
Docker is needed.

## Configuration

Built-in defaults live in `config/default.toml` and are compiled into the
binary. Point the server at your own copy with `--config <PATH>` or the
`DISCORD_MCP_CONFIG` environment variable; a file that fails to parse or validate
is a startup failure, never a silent fallback.

Validate without starting anything:

```powershell
$env:DISCORD_BOT_TOKEN = "..."   # optional for --validate-config
cargo run -- --validate-config
```

## Environment

| Variable | Purpose |
|---|---|
| `DISCORD_BOT_TOKEN` | Bot token, required by `serve` |
| `DISCORD_MCP_CONFIG` | Path to a TOML configuration file |
| `DISCORD_MCP_HTTP_TOKEN` | Bearer token for the HTTP transport, when `http.require_auth` is set (variable name comes from `http.auth_token_env`) |
| `RUST_LOG` | Log filter override, e.g. `info` or `discord_mcp=debug` |

Secrets are never written to configuration files. `.env.example` lists variable
names only; the server does not read `.env` files by itself.

## Running

```powershell
$env:DISCORD_BOT_TOKEN = "..."
cargo run -- serve
```

stdout carries MCP protocol frames only; every diagnostic goes to stderr. Exit
codes: `0` success, `1` internal failure, `2` usage/configuration error, `3`
authentication failure, `4` policy refusal.

MCP client configuration over stdio:

```json
{
  "mcpServers": {
    "discord": {
      "command": "C:\\path\\to\\discord-mcp.exe",
      "args": ["serve"],
      "env": { "DISCORD_BOT_TOKEN": "<set this outside version control>" }
    }
  }
}
```

Reading message **text** needs the **Message Content** privileged intent on the
bot (Developer Portal → Bot → Privileged Gateway Intents). Without it Discord
returns empty `content`, `embeds` and `attachments` in REST responses as well as
gateway events, so `discord_get_messages` reports `content: null` for every
message the bot did not send itself — the tool is working, the data is withheld.
`GUILD_MEMBERS` is the equivalent toggle for the member tools.

### Streamable HTTP

```toml
# config/http.toml
[server]
transport = "http"

[http]
host = "127.0.0.1"
port = 8787
require_auth = true
```

```powershell
$env:DISCORD_BOT_TOKEN = "..."
$env:DISCORD_MCP_HTTP_TOKEN = "<at least 16 random characters>"
cargo run -- serve --config config/http.toml
```

The endpoint is `POST http://127.0.0.1:8787/mcp`, called with
`Authorization: Bearer $env:DISCORD_MCP_HTTP_TOKEN`. Responses are `200` with an
SSE or JSON body; a missing or wrong token is `401` with `WWW-Authenticate:
Bearer`, a refused `Host` or `Origin` is `403`.

The listener is loopback unless `http.allow_non_loopback = true`, and
configuration validation refuses a non-loopback bind unless `require_auth = true`
and `allowed_origins` is non-empty. `Host` must be the bind address (or
`localhost` when bound to loopback), so a DNS name that resolves to `127.0.0.1`
cannot reach the endpoint. A request carrying `Origin` must match
`allowed_origins`; with the default empty list every `Origin`-bearing request is
refused, which is what stops a browser page from driving the server. `both` runs
STDIO and HTTP from the same process over the same `AppState`.

`http.request_timeout_seconds` bounds how long one request may take, headers and
response body together; a request that outlives it is answered `504` or has its
response body cut off, and the long-lived `GET` event stream is exempt because
that is a session stream rather than a request. `http.session_expiry_seconds` is
the idle timeout after which a session is closed and its resources released.

Bot creation, intents, permissions, client configuration, verification steps and
troubleshooting are in [`docs/SETUP.md`](docs/SETUP.md);
`docs/DEPENDENCIES.md` records the verified dependency and API decisions.
