# Live testing through MCP Bridge

Date: 2026-11-18
Build: `discord-mcp 0.1.0`, `feat/server-control`, `target/release/discord-mcp.exe`
Client: `llama-mcp-bridge` at `D:\AI\build\mcp` (stdio and Streamable HTTP)
Guild: `test_mcp_server` (`1572365819281932408`), channel `#general`
(`1572365820133118035`)
Bot: `mcp-test-bot` (`1572386219596316742`)

This run replaced the earlier hand-written JSON-RPC harness with the bridge as a
real MCP client. The bridge performed `initialize` and `tools/list`, namespaced
the tools as `discord-mcp__<tool>`, and every call below went through
`node scripts/mcp.mjs call`.

---

## 1. Method

The stdio server was registered once:

```bash
cd D:/AI/build/mcp
node scripts/mcp.mjs up --no-connect
```

```json
{
  "id": "discord-mcp",
  "transport": "stdio",
  "connection": {
    "command": "D:/AI/build/discord/working_discord_mcp/target/release/discord-mcp.exe",
    "args": ["serve", "--config", "<config>"],
    "env": {},
    "secretEnv": ["DISCORD_BOT_TOKEN"]
  }
}
```

The token was supplied once through the API, moved by the bridge into
`data/secrets.json`, and never appears in `data/servers.json` or this document.

Configurations used, all under `target/` (build output, not committed):

| Config | Purpose |
|---|---|
| built-in defaults | read-only baseline |
| `live-test.toml` | all three mutation classes enabled, guild + `#general` allowlisted, budget 300 |
| `live-budget.toml` | same, `max_actions_per_minute = 2` |
| `live-dryrun.toml` | same, `require_dry_run = ["discord_pin_message"]` |
| `live-http.toml` | `transport = "http"`, `127.0.0.1:8788`, bearer auth |

After changing the configuration, the bridge entry was updated with `PUT
/api/servers/discord-mcp`, which restarts the child. A restart resets the action
budget and the correlation counter.

Raw per-call results are kept outside version control in
`target/live-bridge/*.json` for the life of the build directory.

---

## 2. Read tools — 9/9 passed

| Tool | Result |
|---|---|
| `discord_get_capabilities` | bot `mcp-test-bot`, enabled classes, limits 30/50/[] |
| `discord_list_guilds` | `test_mcp_server`, cursor returned on a full 1-item page |
| `discord_get_guild` | id, name, `owner_id`, `member_count: null` |
| `discord_list_channels` | categories and text channels |
| `discord_get_channel` | `#general`, parent category, position |
| `discord_get_messages` | page; `content: null` because Message Content intent is off |
| `discord_get_message` | the bot's existing message |
| `discord_list_members` | owner first, bounded page |
| `discord_get_member` | owner record |

Mapping note: messages from `discord_get_messages` / `discord_get_message` report
`guild_id: null` even though the channel belongs to the guild. The guild is only
in the channel object, not the message result.

---

## 3. Message mutations

| Operation | Expected | Observed |
|---|---|---|
| `send_message` | ok | ok, id returned |
| `edit_message` | ok | ok, content replaced |
| `add_reaction` (✅) | ok | ok |
| `remove_reaction` (✅) | ok | ok |
| `clear_reactions` | ok | **403 / 50013** (needs Manage Messages) |
| `pin_message` | ok | **403 / 50013** (needs `PIN_MESSAGES`) |
| `unpin_message` | ok | **403 / 50013** |
| mention neutralisation | zero-width space | `"hi @\u200beveryone <\u200b@4673…>"`, confirmed in the returned content |
| `bulk_delete_messages` | ok | **403 / 50013** (needs Manage Messages) |
| `delete_message` (own) | ok | ok |

The failing four are consistent with Discord permissions, not policy: the bot's
role has View Channels, Read Message History and Manage Roles, `@everyone`
supplies Send Messages and Add Reactions, and nothing supplies Manage Messages or
`PIN_MESSAGES`. The pin tools were the important case: their descriptions said
"Needs MANAGE_MESSAGES", but current Discord requires `PIN_MESSAGES`
(February 2026 change), which this guild does not grant. Both descriptions now
name `PIN_MESSAGES`, and a protocol test asserts it so the wording cannot
regress.

All created messages were deleted afterwards; bulk-delete leftovers were removed
one by one with `delete_message`.

---

## 4. Structure tools

| Operation | Expected | Observed |
|---|---|---|
| `create_channel` | ok | **403 / 50013** (no Manage Channels) |
| `edit_channel` on `#general` | ok | **403 / 50013** |
| `create_channel` dry run | planned | planned, even though the real call is 403 |
| `set_permission_overwrite` on `#general` (deny Add Reactions for `@everyone`) | ok | ok, then reverted |
| `delete_permission_overwrite` | ok | ok; channel overwrites verified `[]` |
| `create_invite` | ok | ok, code `YJPnAKBr` (default 24 h expiry) |
| `create_invite` dry run | planned | planned |
| `delete_invite` | ok | **404 / 10006 Unknown Invite** (`GET /invites/<code>` still resolves it; the bot lacks Manage Channels/Manage Guild) |
| `create_thread` | ok | ok, thread `1572632771044581406` |
| `edit_thread` archive | ok | ok |
| `edit_thread` unarchive + lock | ok | ok |
| `edit_thread` archive/unlock after locking | ok | **403 / 50001 Missing Access** |

The channel-allowlist boundary could not be exercised as planned, because
`create_channel` itself is refused by Discord. The "new channel is not on the
allowlist until an operator adds it" behaviour therefore remains covered by unit
tests only.

The dry-run result is the clearest demonstration of its limits: the planned
`create_channel` succeeded locally and the same real call returned 403, exactly
as `docs/TOOLS.md` warns.

The invited code is still valid. The bot cannot revoke it without Manage Channels
or Manage Guild, so it will expire on its own 24 hours after creation.

The thread is left archived=false, locked=true. Once locked, the creator without
Manage Threads cannot archive or unlock it (`50001`), so it needs a moderator.

---

## 5. Role tools

| Operation | Expected | Observed |
|---|---|---|
| `edit_role` on the bot's own highest role | policy refusal | `POLICY_BLOCKED` |
| `delete_role` on the bot's own highest role | policy refusal | `POLICY_BLOCKED` |
| `delete_role` on `@everyone` | local refusal | `INVALID_ARGUMENT` |
| `assign_role` / `remove_role` `@everyone` | local refusal | `INVALID_ARGUMENT` |
| `create_role` with `ADMINISTRATOR` | local refusal | `INVALID_ARGUMENT` |
| `create_role` with a 101-character name | local refusal | `INVALID_ARGUMENT` |
| `create_role` dry run | planned | planned |
| `create_role` | ok | ok, id `1572633321580662914`, position 1 |
| `edit_role` new role | ok | ok: name, colour 255, permissions replaced with `VIEW_CHANNEL` |
| `assign_role` owner | ok | ok |
| `remove_role` owner | ok | ok |
| `delete_role` | ok | ok |

A freshly created role at position 1 is manageable even though it ties the bot's
own role; the earlier assumption that a new role always lands out of reach was
wrong for this guild. The role was deleted and the guild is back to
`@everyone`, `mcp-test-bot`, `smoke-above-bot`.

---

## 6. Policy, budget and audit

| Test | Result |
|---|---|
| send to a channel outside `channel_allowlist` | `POLICY_BLOCKED: channel … is not on policy.channel_allowlist` |
| `discord_list_guilds` `limit: 0` | `INVALID_ARGUMENT: limit must be at least 1` |
| `guild_id: "abc"` | `INVALID_ARGUMENT: guild_id must be a Discord snowflake ID` |
| unknown argument | `failed to deserialize parameters: unknown field 'bogus'` |
| budget 2, three reactions | two applied, third `POLICY_BLOCKED: max_actions_per_minute (2) is exhausted` |
| same window, delete | also `POLICY_BLOCKED` until the process restarted |
| `require_dry_run = ["discord_pin_message"]` | real call `POLICY_BLOCKED`, dry run planned |
| `--read-only` with all classes enabled | capabilities all false; reads work; send and delete `POLICY_BLOCKED` |
| audit records | 21 matched records; denials at `INFO`, failed allowed attempts at `WARN`, each with action, target IDs, outcome, code and correlation |

Audit is visible through the bridge only as captured stderr:
`GET /api/servers/discord-mcp/logs` returns objects `{ts, level: "stderr",
message}`, so a reader must match on the message text, not `level`.

---

## 7. HTTP transport

The server ran on `127.0.0.1:8788` with `require_auth = true`, started directly
so the bridge could attach over Streamable HTTP.

| Check | Result |
|---|---|
| `/health` without token | `401` |
| `/health` with wrong token | `401` |
| `/mcp` with foreign `Origin` | `403` |
| `/health` with correct token | `200` `{"server":"discord-mcp","status":"ok","transport":"http","version":"0.1.0"}` |
| bridge connect | `status=connected`, `tools=32` |
| `discord_get_capabilities` | ok |
| `discord_list_guilds` | ok |
| `discord_send_message` under read-only policy | `POLICY_BLOCKED` |

The bridge entry `discord-mcp-http` was removed afterwards and the temporary
bearer token file deleted; the HTTP process was stopped.

---

## 8. Findings

1. **`pin_message`/`unpin_message` descriptions were stale.** They said Manage
   Messages; live Discord requires `PIN_MESSAGES`. Fixed: both descriptions now
   name `PIN_MESSAGES`.
2. **`ADMINISTRATOR` refusal reused overwrite wording.** `create_role` with
   `ADMINISTRATOR` returned ``` `allow` must not grant ADMINISTRATOR; an overwrite
   cannot escalate privileges ``` even though the input key is `permissions`.
   Fixed: the refusal now names the caller's field (`allow` or `permissions`) and
   says the grant would escalate privileges.
3. **`@everyone` removal message was wrong.** `remove_role` on `@everyone` said
   "cannot be deleted or assigned", omitting "removed". Fixed: it now says
   "cannot be deleted, assigned or removed".
4. **Message results omit the guild.** `guild_id` is `null` for channel messages;
   clients must resolve it from the channel.
5. **Locked threads are a one-way door for the creator.** After `locked: true`,
   a bot without Manage Threads cannot archive or unlock the thread (`50001`).
6. **Invite revocation returned 404/10006** while the invite still resolved. This
   is consistent with Discord hiding an operation the bot is not permitted to
   perform, but it was not confirmed against Discord's documentation.
7. **Bridge idle stop can stall a long batch.** The role batch exceeded the 295 s
   command budget because the child had idle-stopped; the next call reconnected
   and completed in ~2 s. Long live suites should use one call per command or a
   shorter idle timeout.

## 9. State left behind

- Guild roles: `@everyone`, `mcp-test-bot`, `smoke-above-bot` — unchanged.
- `#general` permission overwrites: `[]` — the test overwrite was removed.
- Test messages: none remain; all created messages were deleted.
- `live-thread` (`1572632771044581406`): deleted after the run; verified `404`.
  A thread creator can delete its own thread even without `MANAGE_THREADS`, which
  is why the delete succeeded while `edit_thread` had returned `50001`.
- Invite `YJPnAKBr`: still live until `2026-11-19T15:21:09Z`. Revocation needs
  Manage Channels or Manage Guild, which the bot has and cannot grant itself;
  the API returns `404/10006`. Revoke it in Server Settings → Invites, or grant
  Manage Channels and rerun `DELETE /invites/YJPnAKBr`.
- Bridge: `discord-mcp` restored to built-in read-only `serve`; the temporary
  `discord-mcp-http` entry and its secret were deleted; HTTP process stopped.

## 10. Not covered

- A channel create/edit/delete cycle and the created-channel allowlist boundary
  (Discord Manage Channels not granted).
- `PIN_MESSAGES`, Manage Messages, `MANAGE_THREADS` positive paths.
- Positive overwrite change on a dedicated test channel.
- A managed-role delete/permission-edit refusal.
- The bridge's llama-server chat loop (no local model).
