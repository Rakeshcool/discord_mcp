# discord-mcp

A Model Context Protocol (MCP) server for Discord, using an **official bot
identity**. Windows 11 x64 (Rust), over STDIO or Streamable HTTP.

It is not a self-bot, not user-account automation, and not a Discord client. It
reads through the bot API and can post one message to allowlisted channels when
you enable writes.

## Install

Download `discord-mcp.exe` from the
[latest release](https://github.com/Rakeshcool/discord_mcp/releases/tag/v0.1.0)
and verify it against the attached checksum:

```powershell
(Get-FileHash .\discord-mcp.exe -Algorithm SHA256).Hash.ToLower()
```

The result must be `7d542065e3b178f30279dc79635694e731346c7cf848cc537cd0779cef57add4`.

## Quick start

You need a Discord application with a bot token, and the bot invited to your
server. [`docs/SETUP.md`](docs/SETUP.md) walks through the portal, intents,
permissions and verification.

```powershell
$env:DISCORD_BOT_TOKEN = "<bot token>"
.\discord-mcp.exe --validate-config    # checks config, prints nothing secret
.\discord-mcp.exe serve                # STDIO, the default transport
```

stdout carries MCP frames only; all logs go to stderr. Exit codes: `0` success,
`1` internal error, `2` usage/configuration, `3` authentication, `4` policy
refusal.

## Connect an MCP client

For a client that launches stdio servers (Claude Desktop uses
`%APPDATA%\Claude\claude_desktop_config.json`):

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

**Message text needs the Message Content intent.** Without it Discord returns
empty `content` in REST responses too, so `discord_get_messages` reports
`content: null` for every message the bot did not send itself — the tool is
working, the data is withheld. The **Server Members** intent is the same story
for the member tools.

## Configuration

Defaults are compiled in from `config/default.toml`. Point the server at your own
copy with `--config <PATH>` or `DISCORD_MCP_CONFIG`; a file that fails to parse
or validate stops startup rather than falling back silently.

| Variable | Purpose |
|---|---|
| `DISCORD_BOT_TOKEN` | Bot token, required by `serve`; the variable name comes from `discord.token_env` |
| `DISCORD_MCP_CONFIG` | Path to a TOML configuration file |
| `DISCORD_MCP_HTTP_TOKEN` | Bearer token for the HTTP transport; name comes from `http.auth_token_env` |
| `RUST_LOG` | Log filter, e.g. `info` or `discord_mcp=debug` |

The server reads the environment and never reads `.env` itself; `.env.example`
lists the variable names.

## Writes

Sending is **off by default**. Turning it on needs both settings, and validation
refuses the first without the second:

```toml
[policy]
write_enabled = true
channel_allowlist = ["1572365820133118035"]
```

Message editing, reactions and pins follow `write_enabled`; deleting a message
and every channel, permission, thread and invite tool need `control_enabled`;
bulk delete and every role tool need `administrative_enabled`; each class has its
own allowlist requirement. Every mutation is audited, limited by a shared per-minute budget,
and never retried. `--read-only` (or `DISCORD_MCP_READ_ONLY=1`) overrides the
file and disables all of them for a session.

Structural tools stay inside the allowlist: a thread is checked through its
parent channel, an invite through the channel it points at, and a channel these
tools create must be added to `channel_allowlist` before anything may edit or
delete it. Role tools are guild-scoped, so `guild_allowlist` is their boundary,
and editing a role's permissions replaces the whole set rather than merging into
it. The bot's own highest role cannot be edited or deleted, unconditionally. A
`dry_run` names the action and sends no request.

## Streamable HTTP

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
.\discord-mcp.exe serve --config config\http.toml
```

`POST http://127.0.0.1:8787/mcp` with `Authorization: Bearer <token>`. A missing
or wrong token is `401`; a refused `Host` or `Origin` is `403`. `GET /health`
reports status behind the same token. The listener is loopback-only unless
`http.allow_non_loopback = true`, which validation allows only with
`require_auth = true` and an explicit `allowed_origins` list.

**This server does not terminate TLS.** For anything beyond loopback, run it
behind a proxy that does, with the settings in
[`docs/SETUP.md`](docs/SETUP.md) §7.

## Build from source

```powershell
cargo build --release
```

Windows MSVC only (`x86_64-pc-windows-msvc`); no WSL or Docker. For a build whose
hash matches `dist/*.sha256`, use `scripts\build-release.ps1`, which makes the
build byte-reproducible and prints the artifact's SHA-256.

## Documentation

- [`docs/SETUP.md`](docs/SETUP.md) — Discord application, token, intents,
  permissions, both transports, verification, troubleshooting.
- [`docs/TOOLS.md`](docs/TOOLS.md) — every tool: arguments, result JSON, error
  codes, retry behaviour.
- [`docs/SECURITY.md`](docs/SECURITY.md) — threat model and implemented controls.
- [`docs/DEPENDENCIES.md`](docs/DEPENDENCIES.md) — dependency choices and the
  advisory scan.
