# discord-mcp

A secure Model Context Protocol (MCP) server for Discord, using an **official
Discord bot identity**. Windows 11 x64, Rust, STDIO transport now and MCP
Streamable HTTP next.

It is not a self-bot, not a user-account automation tool, and not a Discord
client reimplementation. See `SPEC.md` for the full specification, `TASKS.md`
for the phased plan, and `docs/SECURITY.md` for the threat model.

## Status

Through Phase 4 — read-only Discord access over stdio.

Implemented: typed configuration with fail-closed validation, stable error
categories, secret wrapper, structural secret redaction, stderr-only logging,
stdio MCP handshake, CLI (`serve`, `--validate-config`), CI; a twilight-backed
read adapter with timeouts, bounded retries and error mapping; a policy engine
(flags, guild/channel allowlists, page limits, concurrency cap, mention
neutralisation, fail-closed approval, content-free audit records) enforced in
front of the client; and eight read tools plus the `discord://guilds` resource.

Not implemented yet: the Streamable HTTP transport, write operations and
destructive tools. `server.transport` values `http` and `both` are rejected at
startup until that phase lands.

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

MCP client configuration over stdio (once tools are available):

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

Operator documentation for bot creation, permissions, tools and troubleshooting
is completed in the release phase; `docs/DEPENDENCIES.md` records the verified
dependency and API decisions.
