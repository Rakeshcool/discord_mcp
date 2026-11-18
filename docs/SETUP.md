# Setup guide

End-to-end instructions for installing and using this MCP server: a Discord bot
application, the build, configuration, both transports, client wiring, and the
checks that tell you it is actually working.

This build exposes 32 tools and one resource: nine read tools and 23 mutations
in three separately gated classes. Every mutation class is off by default.
`policy.write_enabled` opens send, edit, reactions and pins;
`policy.control_enabled` opens message deletion, channels, permission overwrites,
threads and invites; `policy.administrative_enabled` opens bulk delete and the
role tools. Enabling a class also requires a non-empty allowlist (channels for
the first two, guilds for the third). Every real mutation is audited, every
mutation except `discord_send_message` can be planned with `dry_run`, and none is
retried after an ambiguous outcome.

- Platform: Windows 11 x64 (`x86_64-pc-windows-msvc`). No WSL or Docker.
- Transports: STDIO (default) and MCP Streamable HTTP (`/mcp`).
- Identity: an official **bot** token. Never a user token.

---

## 1. Prerequisites

| Requirement | Notes |
|---|---|
| Rust 1.89+ (stable, MSVC host) | `rustc --version`; install from <https://rustup.rs> |
| A Discord account and a server you own | The demo server in these instructions is `test_mcp_server` |
| Permission to create a Discord application | Developer Portal access with your account |

Build and tool checks:

```powershell
cargo build --release
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

The binary lands at `target\release\discord-mcp.exe`. `cargo run --` works the
same way during development.

---

## 2. Discord side

### 2.1 Create the application

1. Open <https://discord.com/developers/applications> and click **New Application**.
2. Name it (for example `discord-mcp`), accept the terms, **Create**.

### 2.2 Get the bot token

1. Left sidebar → **Bot** → **Reset Token** → **Copy**.
2. Store it in `.env` (see §4). Treat it as a password: it grants everything the
   bot can do in every server it has joined.

### 2.3 Enable the privileged intents

Still on the **Bot** page, scroll to **Privileged Gateway Intents**:

| Intent | Why this server needs it | Effect if off |
|---|---|---|
| **Message Content** | Reads `content`, `embeds`, `attachments`, `components` | Every message reads as `content: null`. Discord blanks these fields in **REST responses too**, not just gateway events |
| **Server Members** | `discord_list_members`, `discord_get_member` | Member tools return Discord's permission error |

Toggle the switch, then click **Save Changes** and confirm the dialog. Without
the confirmation the change is not applied. No restart of this server is needed;
Discord applies the flag within seconds.

Verify from a shell (PowerShell):

```powershell
curl.exe -s https://discord.com/api/v10/applications/@me `
  -H "Authorization: Bot $env:DISCORD_BOT_TOKEN"
```

In the JSON, `flags` should include `1 << 18` (262144) or `1 << 19` (524288) —
either means Message Content is configured. For example `flags: 557056` is
`524288 + 32768` = Message Content *limited* + Server Members *limited*.

This server is REST-only: it never opens a gateway connection, so there is
nothing to declare in code. The portal toggle is the whole switch.

### 2.4 Invite the bot with the right guild permissions

1. **OAuth2 → URL Generator**.
2. **Scopes**: tick **`bot`** only. Do not tick `applications.commands` — this
   server registers no slash commands.
3. **Bot Permissions**: tick **View Channels** and **Read Message History**.
4. Copy the generated URL, open it, pick your server, **Authorize**.

Or build the URL directly — `66560` is `View Channels (1024) + Read Message
History (65536)`:

```
https://discord.com/oauth2/authorize?client_id=<APPLICATION_ID>&scope=bot&permissions=66560
```

`Administrator` is never required. Local policy can only narrow what the bot may
do, never widen Discord's own permissions.

The invite above requests only read permissions: **View Channels** and **Read
Message History**. Each mutation class needs more, granted to the bot's own role
under **Server Settings → Roles** (or through channel overwrites):

| Tools | Discord permissions |
|---|---|
| read tools | View Channels, Read Message History; member tools also need the **Server Members** intent |
| send / edit a message | Send Messages; Manage Messages to edit a message the bot did not write |
| delete, bulk delete, clear reactions | Manage Messages |
| add / remove reactions | Add Reactions |
| pin / unpin | Pin Messages (`PIN_MESSAGES`); Manage Messages on Discord API behaviour before February 2026 |
| channels, permission overwrites | Manage Channels, plus Manage Roles to change a role or member overwrite |
| threads | Create Public Threads or Create Private Threads, Manage Threads, Send Messages in Threads |
| invites | Create Instant Invite; Manage Channels on that channel (or Manage Guild) to delete |
| role tools | Manage Roles, and the role below the bot's highest role |

Role tools are enabled separately by `policy.administrative_enabled` and the
guild allowlist; structure tools by `policy.control_enabled`. See
[TOOLS.md §4](TOOLS.md#4-channel-thread-and-invite-control) and §5.

While on the Roles page, drag the bot's role above every role it must manage —
Discord only allows a bot to change roles below its own highest role, and a role
at or above it fails with `403` / code `50013` even when Manage Roles is granted.

Verify the guild permissions the bot actually holds:

```powershell
curl.exe -s https://discord.com/api/v10/users/@me/guilds `
  -H "Authorization: Bot $env:DISCORD_BOT_TOKEN"
```

Each entry has a `permissions` bitfield. The read-only invite sets `1024` (View
Channels) and `65536` (Read Message History). Add the bits for the classes you
enable: `2048` Send Messages, `8192` Manage Messages (delete, bulk delete, clear
reactions, editing another author's message), `64` Add Reactions, `1` Create
Instant Invite, `16` Manage Channels (channels, overwrites, deleting an invite),
`268435456` Manage Roles, `17179869184` Manage Threads, `34359738368` Create
Public Threads, `68719476736` Create Private Threads, `274877906944` Send
Messages in Threads, and `2251799813685248` Pin Messages on current Discord
(older API behaviour used Manage Messages). `Administrator` is never required;
the bot's Discord permissions remain the final authority.

**Intents and permissions are different things.** Permissions decide what the bot
may do in a server; intents decide what data Discord puts in its responses. Both
are required for message text.

---

## 3. Install

```powershell
git clone <repository> discord-mcp
cd discord-mcp
cargo build --release
```

Nothing is installed system-wide. `target\release\discord-mcp.exe` is the
artifact; MCP clients launch it by absolute path.

---

## 4. Secrets

The server reads **environment variables only**. It never reads `.env` itself —
that file exists so you can load the values into your shell or your MCP client's
`env` block.

Create `.env` in the project root (already git-ignored):

```dotenv
DISCORD_BOT_TOKEN=<bot token from §2.2>
DISCORD_MCP_HTTP_TOKEN=<only for the HTTP transport; 32+ random characters>
```

| Variable | Used by | Purpose |
|---|---|---|
| `DISCORD_BOT_TOKEN` | required by `serve` | Bot token. Name is configurable via `discord.token_env` |
| `DISCORD_MCP_HTTP_TOKEN` | HTTP transport with `http.require_auth = true` | Inbound bearer token. Name is configurable via `http.auth_token_env` |
| `DISCORD_MCP_CONFIG` | optional | Path to a TOML configuration file |
| `RUST_LOG` | optional | Log filter, e.g. `info` or `discord_mcp=debug` |

Generate an HTTP token:

```powershell
-join ((1..48) | ForEach-Object { '{0:x}' -f (Get-Random -Max 16) })
```

Load `.env` into the current PowerShell session:

```powershell
Get-Content .env | ForEach-Object {
  if ($_ -match '^([A-Z_]+)=(.*)$') { Set-Item "env:$($Matches[1])" $Matches[2] }
}
```

The two tokens are opposite directions of trust: the bot token is *outbound*
(this server → Discord), the HTTP token is *inbound* (MCP client → this server).
Never reuse one as the other, and never put the bot token in a client config.

### Where the bot token can live

The server reads the environment variable named by `discord.token_env`
(`DISCORD_BOT_TOKEN` by default) and nothing else. Every option below is a way of
putting the token into that variable for the server process, not a different code
path inside the server.

| Option | Use when | Trade-off |
|---|---|---|
| MCP client `env` block (§6) | A desktop client launches the server | The token sits in a client config file; keep that file out of version control and out of shared profiles |
| Shell variable, set per session | Interactive testing | Re-entered each session; nothing persists |
| `.env` plus the loader above | A repo-local workflow | Plaintext file, git-ignored but readable by anything running as your account |
| `setx DISCORD_BOT_TOKEN "..."` | Every new process should see it | Written to your user environment in the registry; visible to anything running as you, and the value lands in shell history |
| Windows Credential Manager plus a launcher | You keep secrets in the OS store | The server does not read it itself, so a launcher must load it into the variable |

Reading it back from Windows Credential Manager in a launcher:

```powershell
# One-time. cmdkey prompts for the secret, so it never appears on the command
# line or in PSReadLine history.
cmdkey /generic:discord-mcp-bot /user:discord-bot /pass

# Every launch: read it into this process and nothing else.
$cred = Get-StoredCredential -Target discord-mcp-bot   # CredentialManager module
$env:DISCORD_BOT_TOKEN = $cred.GetNetworkCredential().Password
.\target\release\discord-mcp.exe serve
```

`Get-StoredCredential` comes from the third-party `CredentialManager` module
(`Install-Module CredentialManager`), not from Windows itself. Without it, the
same value can be read with a small `CredentialRead` P/Invoke or served by a
wrapper that owns the credential.

**Native Credential Manager lookup is not implemented.** `SPEC.md` lists a
`keyring`-backed store as optional, and this build reads the environment only. If
you want the token out of environment blocks entirely, the launcher above is the
supported shape: the variable exists only in that launcher's session and the
server process it starts, and the stored credential is protected by your Windows
account. To check it is
wired up without starting the server, run `--validate-config` — it reports whether
the variable is present and never prints its value.

---

## 5. Configuration

Built-in defaults are compiled in from `config/default.toml`. To change any of
them, copy that file and point the server at the copy with `--config <PATH>` or
`DISCORD_MCP_CONFIG`. A file that fails to parse or validate is a startup error,
never a silent fallback.

```powershell
Copy-Item config\default.toml config\local.toml   # config/local.toml is git-ignored
```

Settings that matter most (see `config/default.toml` for every key):

| Key | Default | Meaning |
|---|---|---|
| `policy.write_enabled` | `false` | Send, edit, reactions and pins; requires a channel allowlist |
| `policy.control_enabled` | `false` | Message deletion, channels, overwrites, threads and invites; requires a channel allowlist |
| `policy.administrative_enabled` | `false` | Bulk delete and the role tools; requires a guild allowlist |
| `policy.autonomy.max_actions_per_minute` | `30` | Rolling per-minute budget shared by every mutation, sends included |
| `policy.autonomy.max_sends_per_channel_per_minute` | `5` | Messages one channel may receive per rolling minute |
| `policy.autonomy.budget_state_file` | unset | File carrying the budget across restarts; unset uses `%LOCALAPPDATA%\discord-mcp\action-budget.json`. One running instance owns it; a second instance with mutations enabled fails startup |
| `policy.autonomy.max_bulk_delete` | `50` | Largest accepted bulk-delete batch; Discord's own ceiling is 100 |
| `policy.autonomy.require_dry_run` | `[]` | Tools that refuse a real call unless called with `dry_run: true` |
| `server.transport` | `stdio` | `stdio`, `http`, or `both` |
| `policy.read_enabled` | `true` | Turn the read tools on |
| `policy.guild_allowlist` | `[]` | Empty means every guild the bot can see. List guild IDs to narrow it |
| `policy.protected_channels` | `[]` | Channel IDs no tool may edit, delete, overwrite or thread-edit, even when allowlisted |
| `policy.protected_roles` | `[]` | Role IDs no tool may edit or delete, even when allowlisted |
| `policy.max_messages_per_read` | `100` | Page-size ceiling; a larger `limit` argument is refused |
| `policy.max_concurrent_requests` | `8` | Caps concurrent Discord requests |
| `discord.request_timeout_seconds` | `20` | Per Discord call |
| `discord.max_retries` | `3` | Retries for retryable read failures |
| `http.host` / `http.port` | `127.0.0.1` / `8787` | HTTP bind; loopback by default |
| `http.require_auth` | `true` | Bearer gate for `/mcp` |
| `http.request_timeout_seconds` | `30` | Deadline per request, response body included |
| `http.max_request_bytes` | `1048576` | Request body cap |
| `http.session_expiry_seconds` | `3600` | Idle session timeout |
| `http.max_in_flight_requests` | `64` | Non-`GET` requests handled at once; beyond the cap: `503` |
| `http.allowed_origins` | `[]` | Empty means any request carrying `Origin` is refused |

Validate without starting anything:

```powershell
.\target\release\discord-mcp.exe --validate-config --config config\local.toml
```

Success prints `configuration valid (source: …)` plus the transport and whether a
bot token is present, on **stderr**. Exit code `0` means valid, `2` means a usage
or configuration error.

Example hardening config — one guild only, reads only:

```toml
# config/local.toml
[server]
transport = "stdio"

[policy]
read_enabled = true
guild_allowlist = ["1572365819281932408"]   # test_mcp_server
```

---

## 6. Use over STDIO

STDIO is the default: no listening socket, one client per process, stdin/stdout
carry MCP frames and every diagnostic goes to stderr.

```powershell
$env:DISCORD_BOT_TOKEN = "<token>"
.\target\release\discord-mcp.exe serve
```

Wire it into any MCP client that launches a stdio server. For Claude Desktop the
file is `%APPDATA%\Claude\claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "discord": {
      "command": "D:\\AI\\build\\discord\\working_discord_mcp\\target\\release\\discord-mcp.exe",
      "args": ["serve"],
      "env": { "DISCORD_BOT_TOKEN": "<set this in the client, not in git>" }
    }
  }
}
```

To use a custom configuration, add `"--config", "D:\\...\\config\\local.toml"` to
`args`.

### What the server exposes

Thirty-two tools and the `discord://guilds` resource: nine read tools (including
`discord_get_capabilities`) and 23 mutations split across the three class flags
above. Argument schemas reject unknown fields, and Discord snowflakes are decimal
strings, never numbers.

**[`docs/TOOLS.md`](TOOLS.md) is the reference**: every argument, result shape,
policy precondition and error code.

### Enabling mutations

Every mutation class is off until you say otherwise, and enabling one requires an
allowlist. Writes (send, edit, reactions, pins) and structure (delete, channels,
overwrites, threads, invites) use the channel allowlist; administration (bulk
delete, roles) uses the guild allowlist:

```toml
[policy]
write_enabled = true
control_enabled = true
administrative_enabled = true
channel_allowlist = ["1572365820133118035"]   # #general
guild_allowlist = ["1572365819281932408"]     # test_mcp_server
protected_channels = []                      # e.g. [#announcements ID]
protected_roles = []                         # e.g. [@admin role ID]

[policy.autonomy]
max_actions_per_minute = 30
max_sends_per_channel_per_minute = 5
max_bulk_delete = 50
require_dry_run = ["discord_delete_channel"]   # optional
```

`discord_send_message` can then post to those channels and no others; the other
mutations act on the same allowlisted scope. IDs in `protected_channels` or
`protected_roles` are refused by the editing and deleting tools even when
allowlisted, before the budget is spent. What applies to every real attempt:
mentions in content are neutralised (a zero-width space is inserted) and the
request carries an empty mention parse list; the body is limited by
`policy.max_message_characters` in characters; every mutation, including
`discord_send_message`, draws on the shared rolling budget
(`max_actions_per_minute`), sends also draw on their channel's
`max_sends_per_channel_per_minute`, and the window is persisted so a restart does
not reset it; and exactly one audit record is written to stderr (`allowed`,
`denied` or `failed` with a category). No mutation is retried after an ambiguous
outcome, because a retried `POST` may already have applied. A `403` means the bot
lacks the Discord permission (§2.4) or cannot reach the target, and it surfaces
as `PERMISSION_DENIED` rather than being retried.

Every mutation except `discord_send_message` accepts `dry_run`: it validates
arguments and local policy, returns `{"dry_run":true,"action":"…"}`, and sends
nothing — no request, no budget, no audit record. A dry run cannot reveal a
missing Discord permission or a role-hierarchy problem; the first real call is
where that `403` appears. `--read-only` (or `DISCORD_MCP_READ_ONLY=1`) turns all
three classes off for the session without editing the file.

A first call to try — `discord_list_guilds` with no arguments — returns:

```json
{"items":[{"id":"1572365819281932408","name":"test_mcp_server","icon_hash":null,"owner":false}],"next_cursor":null}
```

`next_cursor` is the value to pass back as `after` for the next page. Read tools
return Discord's data as JSON text inside the MCP result; failures come back as
`isError` results whose text begins with a stable code: `INVALID_ARGUMENT`,
`POLICY_BLOCKED`, `AUTHENTICATION_FAILED`, `PERMISSION_DENIED`,
`RESOURCE_NOT_FOUND`, `RATE_LIMITED`, `TIMEOUT`, `NETWORK_ERROR`,
`DISCORD_API_ERROR`, `CONFIGURATION_ERROR`, `INTERNAL_ERROR`.

---

## 7. Use over HTTP

For clients that speak MCP Streamable HTTP instead of spawning a process.

```toml
# config/local-http.toml
[server]
transport = "http"

[http]
host = "127.0.0.1"
port = 8787
require_auth = true
auth_token_env = "DISCORD_MCP_HTTP_TOKEN"
```

```powershell
$env:DISCORD_BOT_TOKEN = "<token>"
$env:DISCORD_MCP_HTTP_TOKEN = "<at least 16 characters>"
.\target\release\discord-mcp.exe serve --config config\local-http.toml
```

Endpoint: `POST http://127.0.0.1:8787/mcp` with
`Authorization: Bearer $env:DISCORD_MCP_HTTP_TOKEN`.

Full request sequence (Windows PowerShell 5.1; `curl.exe` ships with Windows):

```powershell
# Bodies go through files: PowerShell 5.1 strips embedded quotes when a literal
# JSON string is passed to a native command, so the request arrives malformed.
Set-Content target\init.json  -NoNewline -Value '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"curl","version":"0.1.0"}}}'
Set-Content target\ready.json -NoNewline -Value '{"jsonrpc":"2.0","method":"notifications/initialized"}'
Set-Content target\call.json  -NoNewline -Value '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"discord_get_guild","arguments":{"guild_id":"1572365819281932408"}}}'

$url   = "http://127.0.0.1:8787/mcp"
$token = $env:DISCORD_MCP_HTTP_TOKEN

# 1. initialize — the response headers carry Mcp-Session-Id
curl.exe -s --max-time 15 -D target\headers.txt -o NUL -X POST $url `
  -H "Authorization: Bearer $token" -H "Content-Type: application/json" `
  -H "Accept: application/json, text/event-stream" -d "@target\init.json"
$sid = (Get-Content target\headers.txt |
  Select-String -Pattern '^mcp-session-id:\s*(\S+)').Matches[0].Groups[1].Value
"session id: $sid"

# 2. acknowledge (a notification: accepted, no response body)
curl.exe -s --max-time 15 -o NUL -w "initialized: %{http_code}" -X POST $url `
  -H "Authorization: Bearer $token" -H "Content-Type: application/json" `
  -H "Accept: application/json, text/event-stream" -H "Mcp-Session-Id: $sid" `
  -d "@target\ready.json"
""

# 3. call a tool. The answer arrives as SSE events; `--max-time` only bounds this
# curl, not the session.
curl.exe -s --max-time 15 -X POST $url `
  -H "Authorization: Bearer $token" -H "Content-Type: application/json" `
  -H "Accept: application/json, text/event-stream" -H "Mcp-Session-Id: $sid" `
  -d "@target\call.json"
```

What the responses mean:

| Response | Meaning |
|---|---|
| `200` + `text/event-stream` | Normal; the JSON-RPC message is in the `data:` payload |
| `202` | Notification accepted |
| `400` | Missing/invalid `Mcp-Session-Id`, or a malformed body |
| `401` + `WWW-Authenticate: Bearer` | No token, or a wrong one |
| `403` | `Host` is not this server's bind, or `Origin` is not in `allowed_origins` |
| `503` + `Retry-After: 1` | `http.max_in_flight_requests` non-`GET` requests are already running |
| `504` | The request outlived `http.request_timeout_seconds` |

Notes for operators:

- The listener is loopback-only unless `http.allow_non_loopback = true`, and
  configuration validation refuses a non-loopback bind without
  `require_auth = true` and explicit `allowed_origins`.
- `Host` must be the bind address (or `localhost` when bound to loopback). This
  blocks DNS-rebinding attacks; a hostname that resolves to `127.0.0.1` is
  refused.
- With `allowed_origins = []` (the default) any request carrying `Origin` is
  refused, so a browser page cannot drive the server. CORS is not authentication.
- **TLS is not implemented.** See [Behind a reverse proxy](#behind-a-reverse-proxy-tls)
  below for the settings a proxy must use.
- `GET /health` answers a small JSON status behind the same bearer token and
  never contacts Discord, so it is safe to poll.
- At most `http.max_in_flight_requests` non-`GET` requests are handled at once;
  one beyond that gets `503` with `Retry-After` rather than waiting in a queue.
- Idle sessions are closed after `http.session_expiry_seconds`; `GET /mcp` is the
  long-lived event stream and is exempt from the request deadline.
- `transport = "both"` runs STDIO and HTTP in one process over the same policy
  and the same bot token.
- Ctrl+C stops the listener gracefully: it stops accepting, finishes the requests
  already in flight (each bounded by `http.request_timeout_seconds`) and logs the
  stop. STDIO stops when the client closes stdin.

### Behind a reverse proxy (TLS)

This server speaks plain HTTP and never terminates TLS. If a client has to reach
it over a network, keep the server on a private interface and let a proxy
terminate TLS in front of it. Nothing below is installed or managed by this
project; it is the shape that works, verified against this server's `Host`
validation.

Two rules the proxy must follow:

1. **Rewrite `Host` to the bind address.** `Host` is validated against the
   configured bind (plus the loopback spellings when bound to `127.0.0.1`), so a
   request arriving with the public name is refused `403`. The proxy has to send
   `Host: 127.0.0.1:8787`.
2. **Pass `Authorization` through unchanged**, and pass `Mcp-Session-Id` on every
   request after `initialize`. The token is still the authentication; a proxy's
   own auth is a second layer, not a replacement for `require_auth = true`.

nginx:

```nginx
server {
    listen 443 ssl;
    server_name mcp.example.com;
    ssl_certificate     /etc/ssl/certs/mcp.crt;
    ssl_certificate_key /etc/ssl/private/mcp.key;

    location / {
        proxy_pass http://127.0.0.1:8787;
        proxy_http_version 1.1;
        proxy_set_header Host 127.0.0.1:8787;   # must match http.host/http.port
        proxy_buffering off;                    # the answer is SSE; buffering stalls it
        proxy_read_timeout 3600s;               # at least http.session_expiry_seconds
    }
}
```

Caddy:

```
mcp.example.com {
    reverse_proxy 127.0.0.1:8787 {
        header_up Host 127.0.0.1:8787
        flush_interval -1        # stream SSE instead of buffering it
    }
}
```

Caddy obtains and renews the certificate itself; `header_up` performs the `Host`
rewrite and `flush_interval -1` disables response buffering.

If the proxy runs on a different host, the server has to bind that host's
address instead of loopback, which means `allow_non_loopback = true`,
`require_auth = true` and a non-empty `allowed_origins` in `[http]`. `Host` in
the proxy then has to be that address.

What the server deliberately does **not** trust:

- `X-Forwarded-For`, `X-Forwarded-Proto` and `X-Real-IP` are ignored. There is no
  trusted-proxy mode, client addresses are not logged, and the forwarded scheme
  is never used to relax a check.
- `Host` and `Origin` validation happen after the proxy, on what the proxy
  actually sends, so a misconfigured rewrite surfaces as `403` rather than as a
  silent bypass.
- The bot token never goes in the proxy configuration; it belongs to the server
  process.

A proxy health check can use `GET /health` with the bearer token: it answers
`200` with `{"status":"ok","server":"discord-mcp","version":"0.1.0","transport":"http"}`
and is exempt from `http.max_in_flight_requests`, so it still answers when every
request slot is busy.

---

## 8. Verify the installation

Run these in order; each one isolates a different layer.

1. **Configuration** — `discord-mcp.exe --validate-config` → exit `0`, and it
   reports `bot token: present`.
2. **Discord credentials** — ask for the guild list over STDIO. A `401` here
   means the bot token is wrong; `AUTHENTICATION_FAILED` is the tool result text.
3. **Guild visibility** — `discord_list_guilds` contains your server. If it does
   not, the bot was never invited (§2.4).
4. **Channel reads** — `discord_list_channels` then `discord_get_messages` on a
   channel. Text coming back proves permissions, intent and the model mapping.
   `discord_get_user_messages` exercises the fan-out path: it should list each
   visible text channel under `channels`, and a channel without Read Message
   History should carry an `error` without failing the others.
5. **Refusals still fail closed** — `guild_id: "abc"` must return
   `INVALID_ARGUMENT`, and a `limit` above `policy.max_messages_per_read` must be
   refused rather than clamped.
6. **Mutation gates** — with writes and control enabled, `dry_run` on a mutation
   must return `{"dry_run":true,…}` and leave no audit record; a real call in a
   channel outside `policy.channel_allowlist` must return `POLICY_BLOCKED`; with
   the class flag off, the same call must be `POLICY_BLOCKED` before any request.
7. **HTTP gate** — with the server running: no token → `401`, wrong token →
   `401`, `Host: rebind.example` → `403`, `Origin: https://evil.example` → `403`,
   correct token → `200`.
8. **Artifact integrity** — if you are checking a packaged build against
   [`../dist/discord-mcp-0.1.0-x86_64-pc-windows-msvc.sha256`](../dist/discord-mcp-0.1.0-x86_64-pc-windows-msvc.sha256):

   ```powershell
   .\scripts\build-release.ps1
   Get-Content dist\discord-mcp-0.1.0-x86_64-pc-windows-msvc.sha256
   ```

   The script prints the SHA-256 of the artifact it just built; it must equal the
   hash in that file. The release build is byte-reproducible: `build.rs` passes
   `/Brepro`, without which the MSVC linker stamps the build time and a random
   PDB GUID into the executable, and the script remaps the Cargo registry and
   workspace paths that dependency panic messages would otherwise embed. A plain
   `cargo build --release` in this checkout produces the same hash, as does a
   build from an empty `target/`. A build from a **different directory** does not:
   Cargo hashes the package's absolute path into crate metadata, which reorders
   symbols, so verify by rebuilding in the same checkout. The executable itself is
   not committed; the build made from this tag is attached to the GitHub release,
   with the same hash.

A quick scripted STDIO smoke test (PowerShell), which prints the JSON-RPC
responses and nothing else:

```powershell
$init = '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"smoke","version":"0.1.0"}}}'
$note = '{"jsonrpc":"2.0","method":"notifications/initialized"}'
$call = '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"discord_list_guilds","arguments":{"limit":10}}}'
@($init, $note, $call) | .\target\release\discord-mcp.exe serve
```

Not covered by this guide: launching the server from a Windows desktop MCP client
end to end. The stdio contract is exercised by process-level tests against the
real binary, but a specific client's launch behaviour is unverified here.

---

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Exit `2`, `DISCORD_BOT_TOKEN` missing | `serve` without a token | Load `.env` or set the variable (§4) |
| Exit `2`, `http.host … must be a loopback address` | Non-loopback bind without opt-in | Set `allow_non_loopback = true`, `require_auth = true`, non-empty `allowed_origins` |
| Exit `2`, `cannot bind …` | Port already in use | Change `http.port` |
| Tool result `AUTHENTICATION_FAILED` | Wrong/revoked bot token, or a user token | Reset the bot token and update `.env` |
| Tool result `content: null` for every message | Message Content intent is off | Enable it and confirm the Save dialog (§2.3) |
| Member tools return a permission error | Server Members intent is off | Enable it in the same section |
| Tool result `RESOURCE_NOT_FOUND` / `404` | Wrong ID, or the bot cannot see that channel | Re-read `discord_list_channels` |
| Tool result `POLICY_BLOCKED` | Guild/channel not in the allowlist, reads disabled, or the class flag off | Adjust `policy.*` |
| `POLICY_BLOCKED: … max_actions_per_minute … is exhausted` | More real mutations than the rolling budget allows | Wait out the minute, or raise the budget deliberately |
| `POLICY_BLOCKED: … max_sends_per_channel_per_minute … is exhausted` | Too many messages to one channel in a minute | Wait out the minute, or raise the per-channel cap |
| `POLICY_BLOCKED: … must be called with dry_run = true` | The tool is listed in `policy.autonomy.require_dry_run` | Call it with `dry_run: true`, or remove it from the list |
| `PERMISSION_DENIED` with `50013` on a role tool | Missing Manage Roles, or the role sits at/above the bot's highest role | See §2.4; move the bot's role higher |
| HTTP `401` | Missing/wrong bearer token | Check `DISCORD_MCP_HTTP_TOKEN` and the client header |
| HTTP `403` | Foreign `Host` or `Origin` | Call the bind address; add the origin to `allowed_origins` if a browser must reach it |
| HTTP `400` | Missing `Mcp-Session-Id` on a follow-up request | Reuse the id returned by `initialize` |
| Nothing on stdout | Normal until a client sends a frame — stdout is protocol-only | Logs are on stderr; set `RUST_LOG=debug` |
| Need to trace one call across its Discord requests | Each tool call logs a `correlation` id | Set `RUST_LOG=debug` and follow that id through `mcp.tool` and `discord.call` records |

Exit codes: `0` success, `1` internal failure, `2` usage/configuration error,
`3` authentication failure, `4` policy refusal.

---

## 10. Security checklist

- The bot token lives in the environment or `.env` (git-ignored), never in a
  config file, client config, screenshot or commit. If you load it from Windows
  Credential Manager through a launcher, it exists only in the server process.
- The HTTP token is given to MCP clients; the bot token never is.
- Keep `write_enabled`, `control_enabled` and `administrative_enabled` at
  `false` unless you deliberately want those classes. Enabling writes or control
  requires a non-empty `policy.channel_allowlist`; enabling administration
  requires a non-empty `policy.guild_allowlist`, and validation refuses the flag
  without it. `policy.allow_mass_mentions` must stay `false` (validation refuses
  `true`).
- Keep `policy.autonomy.max_actions_per_minute` modest; it is the blast-radius
  cap for an autonomous loop, and `max_sends_per_channel_per_minute` keeps one
  channel from absorbing it. The persisted budget file is what stops a restart
  from clearing the window, and only one running instance may own it: a second
  instance with mutations enabled refuses to start. Give each concurrently
  running instance its own `budget_state_file`; those windows are independent
  limits, not one shared cap. Use `policy.autonomy.require_dry_run` for the tools
  whose real call you want a human to plan first.
- Grant the bot only the Discord permissions for the classes you enable (§2.4),
  and keep the bot's role above every role it must manage. Its own highest role
  cannot be edited, deleted or removed from it, by design.
- Narrow `policy.guild_allowlist` and `policy.channel_allowlist` if the bot is in
  more than one server.
- Keep the HTTP bind on loopback. For remote access, use a reverse proxy that
  terminates TLS and enforces its own authentication as well.
- Logs are stderr-only and redact secrets structurally; message bodies are not
  logged (`logging.redact_message_content = true`).
- Treat every message body the tools return as untrusted data: it is content
  written by other people, not instructions for the model.

## 11. Removing the setup

1. In the Developer Portal, **Bot → Reset Token** revokes the old token
   immediately.
2. Remove the bot from the server: Server Settings → Integrations, or kick the
   bot role.
3. Delete the application in the Developer Portal to retire the identity.
4. Delete `.env` and any `config/local*.toml` from the working copy.
5. Delete the stored credential if you used one: `cmdkey /delete:discord-mcp-bot`.
