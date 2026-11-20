<p align="center">
  <img src="logo.png" alt="discord-mcp logo" width="200">
</p>

# discord-mcp

A Model Context Protocol (MCP) server for Discord, using an **official bot
identity**. Windows 11 x64 (Rust), over STDIO or Streamable HTTP.

It is not a self-bot, not user-account automation, and not a Discord client. It
reads through the bot API and, when each policy class is enabled, sends and
manages messages, channels, threads, invites and roles — every mutation gated,
limited, auditable and dry-runnable.

## Install

Download `discord-mcp.exe` from the
[v0.1.0-access.1 release](https://github.com/Rakeshcool/discord_mcp/releases/tag/v0.1.0-access.1).
The release includes `discord-mcp-0.1.0-access.1-x86_64-pc-windows-msvc.sha256`;
verify the download against that same release:

```powershell
(Get-FileHash .\discord-mcp.exe -Algorithm SHA256).Hash.ToLower()
```

The published binary's SHA-256 is
`55ec0c07da573a3ded997ea78c2a0369a33917273e4cbc3eb6eeb875d073c66f`.
Do not compare a download with a checksum from a different release.

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
empty `content`, `embeds` and `attachments` in REST responses too, so
`discord_get_messages` reports `content: null` and empty rich fields for every
message the bot did not send itself — the tool is working, the data is withheld.
The **Server Members** intent is the same story for the member tools.

## Configuration

Defaults are compiled in from `config/default.toml`. Point the server at your own
copy with `--config <PATH>` or `DISCORD_MCP_CONFIG`; a file that fails to parse
or validate stops startup rather than falling back silently.

For a one-session class override, pass `--access` with `read`, `write`,
`control`, `moderation`, `administrative`, or `all`. It accepts a comma-separated
list and can be repeated:

```powershell
.\discord-mcp.exe serve --access write,control --config config\local.toml
```

Access flags only enable classes. The configured allowlists still apply:
`channel_allowlist` for write and control, `guild_allowlist` for moderation and
administration. `--read-only` (or `DISCORD_MCP_READ_ONLY=1`) forces every mutation class off,
regardless of `--access`.

| Variable | Purpose |
|---|---|
| `DISCORD_BOT_TOKEN` | Bot token, required by `serve`; the variable name comes from `discord.token_env` |
| `DISCORD_MCP_CONFIG` | Path to a TOML configuration file |
| `DISCORD_MCP_HTTP_TOKEN` | Bearer token for the HTTP transport; name comes from `http.auth_token_env` |
| `RUST_LOG` | Log filter, e.g. `info` or `discord_mcp=debug` |

The server reads the environment and never reads `.env` itself; `.env.example`
lists the variable names.

## What it can do

40 tools, grouped by the policy flag that enables them.

| Area | Tools | Enabled by |
|---|---|---|
| Capabilities | `discord_get_capabilities` | `read_enabled` (default on) |
| Read | `discord_list_guilds`, `discord_get_guild`, `discord_list_channels`, `discord_get_channel`, `discord_get_messages`, `discord_get_user_messages`, `discord_find_abusive_messages`, `discord_get_message`, `discord_list_members`, `discord_get_member` | `read_enabled` (default on) |
| Messages | `discord_send_message`, `discord_edit_message`, `discord_add_reaction`, `discord_remove_reaction`, `discord_clear_reactions`, `discord_pin_message`, `discord_unpin_message` | `write_enabled` |
| Message moderation | `discord_delete_message` | `control_enabled` |
| Member moderation | `discord_set_member_nickname`, `discord_kick_member`, `discord_ban_member`, `discord_unban_member`, `discord_timeout_member`, `discord_clear_member_timeout` | `moderation_enabled` |
| Guild moderation | `discord_bulk_delete_messages` | `administrative_enabled` |
| Structure | `discord_create_channel`, `discord_edit_channel`, `discord_delete_channel`, `discord_set_permission_overwrite`, `discord_delete_permission_overwrite`, `discord_create_thread`, `discord_edit_thread`, `discord_create_invite`, `discord_delete_invite` | `control_enabled` |
| Roles | `discord_create_role`, `discord_edit_role`, `discord_delete_role`, `discord_assign_role`, `discord_remove_role` | `administrative_enabled` |

`discord_get_user_messages` accepts `has_attachments` to return only messages
with attachments, only messages without them, or both when omitted.
`discord_find_abusive_messages` scans the same bounded channel window for
case-insensitive abuse keywords and groups matches by author. Its built-in list
is conservative and can be replaced with `keywords`; matches are review signals,
not a moderation verdict, and the tool never takes action automatically.

`discord_get_capabilities` reports the enabled classes, their limits and the
authenticated bot, so a client can plan within its rights before calling anything
else.

### Enabling mutations

Every mutation class is **off by default** and needs its config flag (or the
matching `--access` class) plus a non-empty allowlist:

| Flag | What it opens | Allowlist |
|---|---|---|
| `write_enabled` | send/edit messages, reactions, pins | `channel_allowlist` |
| `control_enabled` | delete a message, channels, permission overwrites, threads, invites | `channel_allowlist` |
| `moderation_enabled` | nicknames, kicks, bans, unbans and timeouts | `guild_allowlist` |
| `administrative_enabled` | bulk delete, all role tools | `guild_allowlist` |

```toml
[policy]
write_enabled = true
control_enabled = true
moderation_enabled = true
administrative_enabled = true
channel_allowlist = ["1572365820133118035"]
guild_allowlist = ["1572365819281932408"]
```

Every mutation except `discord_send_message` accepts `dry_run`, which returns the
planned action without contacting Discord and without spending budget. Every real
mutation, including `discord_send_message`, draws on
`policy.autonomy.max_actions_per_minute`; sends also have their own
`policy.autonomy.max_sends_per_channel_per_minute` (default 5) so one busy channel
cannot consume the whole allowance. No mutation is retried after an ambiguous
outcome. `policy.autonomy.require_dry_run` can require the dry-run step for named
tools, and `--read-only` (or `DISCORD_MCP_READ_ONLY=1`) turns every mutation class
off for a session.

The rolling window is written to `policy.autonomy.budget_state_file` after every
consumed action, so restarting the process does not reset it. Omit the key to use
`%LOCALAPPDATA%\discord-mcp\action-budget.json`; an unreadable or malformed file
stops startup instead of starting with an empty window.

Exactly one running process may own that file. A process with any mutation class
enabled takes an exclusive lock and keeps it for its lifetime; a second process
on the same path refuses to start and names `budget_state_file`, instead of
racing the first and undercounting. Read-only processes never open or lock it. To
run instances concurrently, give each its own `budget_state_file` — those windows
are separate limits, not one shared cap.

There is no confirmation prompt. The approval model is the class flag, the
allowlist, the rolling budget, `dry_run` as the plan step and the audit record of
every attempt; a client that needs human sign-off asks for it before the real
call.

`discord_send_message` accepts text, rich embeds, local files and a reply target.
Text, embed fields, file count and file sizes are all bounded locally before the
request. File uploads are disabled unless `policy.attachment_root` is set, and a
path may not escape that directory.

Bulk delete additionally enforces Discord's 2–100 message range, the 14-day
window and `policy.autonomy.max_bulk_delete`. Mass mentions stay off: parsed
`@everyone`, `@here`, role and user mentions are not sent as live pings.

Member moderation is a separate opt-in class. Nicknames are 1–32 characters
(an empty string clears one), reasons are capped at 512 characters, a ban may
delete at most seven days of messages, and timeouts run from one second to 28
days. The bot cannot target itself, and every action needs the matching Discord
permission (`MANAGE_NICKNAMES`, `KICK_MEMBERS`, `BAN_MEMBERS` or
`MODERATE_MEMBERS`).

Structural tools stay inside the allowlist: a thread is checked through its
parent channel, an invite through the channel it points at, and a channel these
tools create must be added to `channel_allowlist` before anything may edit or
delete it.

`policy.protected_channels` and `policy.protected_roles` are a second, harder
boundary: a listed channel cannot be edited, deleted or given a new permission
overwrite, and a listed thread cannot be edited; a listed role cannot be edited
or deleted. Protection is independent of the allowlists and is checked before
the action budget, so it applies even to an allowlisted ID and a refusal spends
nothing.

### Roles

Role tools are guild-scoped, so `guild_allowlist` is their boundary. Permission
names resolve to Discord bits, `ADMINISTRATOR` can never be granted, and editing
`permissions` replaces the whole set rather than merging into it. Names are
trimmed and limited to 1–100 characters locally.

They also need the bot's Discord **Manage Roles** permission, and Discord only
lets the bot touch roles below the bot's highest role; a missing permission or
hierarchy refusal surfaces as `PERMISSION_DENIED` (`403`, code `50013`). The
bot's own highest role cannot be edited, deleted, or removed from the bot,
unconditionally. `@everyone` can be edited but not deleted, assigned or removed.

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

Windows MSVC only (`x86_64-pc-windows-msvc`); no WSL or Docker. For a
byte-reproducible local build, use `scripts\build-release.ps1`, which prints the
artifact's SHA-256. Compare that hash with the build you are verifying, not with
a different release or checkout.

## Documentation

- [`docs/SETUP.md`](docs/SETUP.md) — Discord application, token, intents,
  permissions, both transports, verification, troubleshooting.
- [`docs/TOOLS.md`](docs/TOOLS.md) — every tool: arguments, result JSON, error
  codes, retry behaviour.
- [`llm_instruction.md`](llm_instruction.md) — how an LLM works through the
  server: handshake, capability discovery, read and mutation workflows, result
  codes and safety rules.
- [`docs/LIVE_TESTING.md`](docs/LIVE_TESTING.md) — live results, findings,
  cleanup and current gaps.
- [`conduct_test.md`](conduct_test.md) — short manual QA checklist.
- [`docs/SECURITY.md`](docs/SECURITY.md) — threat model and implemented controls.
- [`docs/DEPENDENCIES.md`](docs/DEPENDENCIES.md) — dependency choices and the
  advisory scan.
