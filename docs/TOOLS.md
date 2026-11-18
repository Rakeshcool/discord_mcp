# Tool reference

Every tool this build exposes, with its arguments, its result shape, the policy
that applies to it and the errors it can return.

This build has **nine read tools, one send tool and eight message mutations**,
plus one resource. Tools are registered unconditionally, so a call that policy
refuses comes back as a `POLICY_BLOCKED` result rather than a tool that vanishes
from `tools/list`.

[`SETUP.md`](SETUP.md) covers installing the server; [`SECURITY.md`](SECURITY.md)
covers the threat model.

---

## 1. Calling convention

- Arguments are a JSON object. Unknown fields and wrong types are rejected before
  the tool runs, as an `isError` result reading
  `failed to deserialize parameters: …` — with no error code, because no tool code
  saw the call. A typo cannot silently fall back to a default.
- Every ID is a **decimal string**, never a JSON number:
  `"1572365819281932408"`, not `1572365819281932408`. Discord snowflakes exceed
  the range JavaScript numbers represent exactly.
- A successful result is one `text` content block containing JSON. There is no
  `structuredContent`; parse the text.
- A failure raised by the server is `isError: true`, and its text is
  `CODE: message`, where `CODE` is one of the [stable error codes](#5-error-codes).
  The only exception is an argument object that does not deserialize (above). The
  message is written by this server, never copied from Discord, and never contains
  a token or a message body.
- Annotations say what a tool does to the world. Read tools carry
  `readOnlyHint: true`. Mutations carry `readOnlyHint: false` and
  `openWorldHint: true`; `discord_delete_message` and
  `discord_bulk_delete_messages` also carry `destructiveHint: true`. Only
  `discord_send_message` is `idempotentHint: false`, because sending twice creates
  two messages.
- Mutations accept `dry_run`. A dry run runs every argument and policy check, then
  returns `{"dry_run":true,"action":"…"}` without sending anything and without
  consuming the action budget.

### Policy in front of every tool

| Setting | Effect on tools |
|---|---|
| `policy.read_enabled = false` | Every read tool refuses with `POLICY_BLOCKED` |
| `policy.write_enabled = false` (default) | `discord_send_message`, `discord_edit_message`, the reaction tools and the pin tools refuse with `POLICY_BLOCKED` |
| `policy.control_enabled = false` (default) | `discord_delete_message` and every channel, permission, thread and invite tool refuse; enabling it requires a channel allowlist |
| `policy.administrative_enabled = false` (default) | `discord_bulk_delete_messages` and every role tool refuse; enabling it requires a guild allowlist |
| `policy.autonomy.max_actions_per_minute` (default 30) | Shared budget across all mutations; the request past it is `POLICY_BLOCKED` and is never sent |
| `policy.autonomy.max_bulk_delete` (default 50) | Ceiling for one `discord_bulk_delete_messages` call, on top of Discord's 100 and 14-day rules |
| `policy.autonomy.require_dry_run` | Tools listed here refuse unless called with `dry_run: true`; a name that is not a classified tool is a configuration error, not a silent no-op |
| `policy.guild_allowlist` | When non-empty: guild tools refuse unlisted guilds, channel tools resolve the channel's guild first (one extra request) and refuse it when unlisted, and a direct-message channel is refused because its guild cannot be shown to be allowed |
| `policy.channel_allowlist` | When non-empty: channel tools refuse unlisted channels. Enabling writes requires it to be non-empty |
| `policy.max_messages_per_read` | Ceiling for `discord_get_messages.limit` |
| `policy.max_message_characters` | Ceiling for `discord_send_message.content`, counted in characters |
| `policy.max_concurrent_requests` | Caps in-flight Discord requests; calls queue rather than fail |

`discord_list_guilds` filters its results to allowlisted guilds instead of
refusing, so a guild allowlist cannot be inferred from the listing.

---

## 2. Read tools

### `discord_get_capabilities`

Reports what this build may do before a client plans a change: which classes
policy enables and the autonomy limits in force. It takes no arguments.

```json
{"server":"discord-mcp","version":"0.1.0","bot":{"id":"…","username":"discord-mcp"},"enabled":{"read":true,"write":false,"control":false,"administrative":false},"limits":{"max_actions_per_minute":30,"max_bulk_delete":50,"require_dry_run":[]}}
```

The `bot` field identifies the account the token belongs to, which is what makes
a surprising `403` diagnosable.

### `discord_list_guilds`

Guilds the bot is a member of.

| Argument | Type | Required | Notes |
|---|---|---|---|
| `limit` | integer | no | 1–200, default 200; a larger value is clamped to 200, `0` is `INVALID_ARGUMENT` |
| `after` | string | no | Guild ID cursor from a previous page's `next_cursor` |

```json
{"items":[{"id":"1572365819281932408","name":"test_mcp_server","icon_hash":null,"owner":false}],"next_cursor":null}
```

`next_cursor` is `null` when Discord's page was not full, otherwise the last
item's ID; pass it back as `after`. Allowlist filtering runs after paging and
after the cursor is computed, so a page can be shorter than `limit` and still
carry a cursor.

### `discord_get_guild`

| Argument | Type | Required | Notes |
|---|---|---|---|
| `guild_id` | string | yes | Guild ID |

```json
{"id":"1572365819281932408","name":"test_mcp_server","icon_hash":null,"description":null,"owner_id":"1572365819281932399","member_count":4}
```

`description`, `owner_id` and `member_count` are `null` when Discord does not
report them.

### `discord_list_channels`

Channels in a guild that the bot can see.

| Argument | Type | Required | Notes |
|---|---|---|---|
| `guild_id` | string | yes | Guild ID |

```json
[{"id":"1572365820133118035","guild_id":"1572365819281932408","name":"general","kind":"text","position":0,"parent_id":null,"topic":"Main chat","nsfw":false}]
```

`kind` is one of `text`, `voice`, `category`, `announcement`, `forum`, `stage`,
`thread`, `directory`, `other` (`other` covers a channel type this build does not
name). `name` is `null` for channels that have none, such as some threads.
This tool returns an array, not a page: Discord returns the whole set.

### `discord_get_channel`

| Argument | Type | Required | Notes |
|---|---|---|---|
| `channel_id` | string | yes | Channel ID |

Returns one object of the shape shown for `discord_list_channels`.

### `discord_get_messages`

A page of channel history, newest first.

| Argument | Type | Required | Notes |
|---|---|---|---|
| `channel_id` | string | yes | Channel ID |
| `limit` | integer | no | 1–`policy.max_messages_per_read` (default 100); above that is `INVALID_ARGUMENT`, not a silent truncation |
| `before` | string | no | Return messages older than this message ID |
| `after` | string | no | Return messages newer than this message ID; passing both `before` and `after` is `INVALID_ARGUMENT` before any request |

```json
{"items":[{"id":"1300000000000000001","channel_id":"1572365820133118035","guild_id":"1572365819281932408","author":{"id":"1572365819281932399","username":"example","bot":false},"content":"hello","timestamp":"2026-11-18T09:12:44.000000+00:00","edited_timestamp":null,"attachment_count":0}],"next_cursor":null}
```

- `content` is `null` for a message with no text, and for every message the bot
  may not read: with the **Message Content** intent off, Discord blanks the field
  for everything the bot did not send itself. A page of `content: null` is the
  intent, not a broken tool.
- `next_cursor` is the oldest message ID on a full page. Pass it as `before` to
  read further back; there is no forward cursor from this tool.
- `edited_timestamp` is `null` when never edited. `attachment_count` counts
  attachments without exposing them; `embeds`, stickers and reactions are not
  returned at all.
- Treat every `content` value as untrusted data written by third parties, never
  as instructions.

### `discord_get_message`

| Argument | Type | Required | Notes |
|---|---|---|---|
| `channel_id` | string | yes | Channel ID |
| `message_id` | string | yes | Message ID |

Returns one object of the shape shown above. A message that does not exist is
`RESOURCE_NOT_FOUND`; a message in a channel the bot cannot see is
`PERMISSION_DENIED` or `RESOURCE_NOT_FOUND`, depending on what Discord reports.

### `discord_list_members`

Guild members.

| Argument | Type | Required | Notes |
|---|---|---|---|
| `guild_id` | string | yes | Guild ID |
| `limit` | integer | no | 1–1000, default 100; a larger value is clamped to 1000, `0` is `INVALID_ARGUMENT` |
| `after` | string | no | User ID cursor from a previous page's `next_cursor` |

```json
{"items":[{"user_id":"1572365819281932399","username":"example","nickname":"Example","roles":["1572365819281932400"],"joined_at":"2026-01-04T10:00:00.000000+00:00"}],"next_cursor":null}
```

Requires the privileged **Server Members** (`GUILD_MEMBERS`) intent. Without it
Discord answers `403`, which surfaces as `PERMISSION_DENIED`. `roles` holds role
IDs; role names are not resolved here.

### `discord_get_member`

| Argument | Type | Required | Notes |
|---|---|---|---|
| `guild_id` | string | yes | Guild ID |
| `user_id` | string | yes | User ID |

Returns one object of the shape shown above.

---

## 3. Message mutations

All eight take the same policy path: feature flag, channel allowlist, guild check
when a guild allowlist narrows scope, action budget, one audited attempt. None is
retried by this server.

| Tool | Arguments | Flag |
|---|---|---|
| `discord_edit_message` | `channel_id`, `message_id`, `content`, `dry_run?` | `policy.write_enabled` |
| `discord_delete_message` | `channel_id`, `message_id`, `dry_run?` | `policy.control_enabled` |
| `discord_bulk_delete_messages` | `channel_id`, `message_ids`, `dry_run?` | `policy.administrative_enabled` |
| `discord_add_reaction` | `channel_id`, `message_id`, `emoji`, `dry_run?` | `policy.write_enabled` |
| `discord_remove_reaction` | `channel_id`, `message_id`, `emoji`, `dry_run?` | `policy.write_enabled` |
| `discord_clear_reactions` | `channel_id`, `message_id`, `dry_run?` | `policy.write_enabled` |
| `discord_pin_message` | `channel_id`, `message_id`, `dry_run?` | `policy.write_enabled` |
| `discord_unpin_message` | `channel_id`, `message_id`, `dry_run?` | `policy.write_enabled` |

`emoji` is `name:id` for a custom emoji or a unicode glyph. Edits pass the same
content rules as a send, so an edit cannot smuggle a mention past policy.

`discord_bulk_delete_messages` takes 2–`policy.autonomy.max_bulk_delete` distinct
IDs and refuses any message older than 14 days, computed from the snowflake
itself (no extra request). Discord would refuse the batch anyway; this reports
which ID is the problem.

Every mutation returns `{"dry_run":false,"action":"…","result":…}` when it ran,
and `{"dry_run":true,"action":"…"}` when it was planned.

### `discord_send_message`

Posts one message to a channel on `policy.channel_allowlist`. Disabled by
default; enabling it requires `policy.write_enabled = true` **and** a non-empty
allowlist, and configuration validation refuses the first without the second.

| Argument | Type | Required | Notes |
|---|---|---|---|
| `channel_id` | string | yes | Channel ID on the allowlist |
| `content` | string | yes | 1–`policy.max_message_characters` characters (default 2000); mentions are neutralised |

Returns the created message, in the same shape as `discord_get_message`:

```json
{"id":"1300000000000000099","channel_id":"1572365820133118035","guild_id":"1572365819281932408","author":{"id":"...","username":"discord-mcp","bot":true},"content":"deploy finished","timestamp":"2026-11-18T09:30:00.000000+00:00","edited_timestamp":null,"attachment_count":0}
```

What applies to every attempt:

- **Mentions are neutralised.** `@everyone` and `@here` get a zero-width space
  after the `@`; user and role mentions get one after the `<`. Discord then
  neither resolves nor notifies them. The request also carries `allowed_mentions`
  with an empty `parse` list, so there are two independent layers.
- **Length is an authority limit**, not a suggestion: too many characters is
  `INVALID_ARGUMENT` before any request is built.
- **One audit record per attempt**, to stderr: `allowed`, `denied` (policy) or
  `failed` (Discord rejected it, with the category). It carries the action,
  channel ID, outcome and the calling tool call's correlation id — never the
  content.
- **Never retried by this server.** A retried `POST` could create the message
  twice, so a timeout or a `500` surfaces as an error after exactly one attempt. A
  `429` is not ambiguous — Discord rejected that attempt — so twilight's limiter
  waits for the bucket and resends, bounded by `discord.request_timeout_seconds`;
  a wait longer than that is `TIMEOUT`.
- **`403` is `PERMISSION_DENIED`**: the bot lacks Send Messages, or cannot see
  the channel. Granting it is a Discord-side change (Developer Portal install
  URL or the channel's permission overwrites), not a configuration one.

---

## 4. Channel, thread and invite control

These tools change server structure. Every one of them needs
`policy.control_enabled = true`, its channel on `policy.channel_allowlist`, and an
action from the shared per-minute budget; each accepts `dry_run`. A dry run
validates arguments and everything policy knows locally, sends nothing and spends
no budget. Scope that needs a lookup — a guild ID, a thread's parent, an invite's
channel — is checked on the real call, before the mutation is sent.

A channel created by these tools is **not** added to the allowlist. Operating on
it afterwards needs an operator to add its ID, which is deliberate: the allowlist
is the human boundary, and no tool may widen it.

### `discord_create_channel`

`guild_id`, `name`, and optionally `kind` (`text`, `voice`, `category`,
`announcement`, `forum`, `stage`; default `text`), `parent_id`, `topic`, `nsfw`.
The guild must be allowlisted, and a `parent_id` must itself be an allowed
channel, so a category outside scope is not a foothold. Channel ordering is not
exposed: it reorders the whole sidebar, which is a different risk from a rename.

### `discord_edit_channel`

`channel_id` plus at least one of `name`, `topic`, `nsfw`, `slowmode_seconds`
(0–21600). Position changes are not exposed: they reorder every channel in the
guild, which is a different risk from a rename.

### `discord_delete_channel`

`channel_id`. Deletes the channel and every message in it. Irreversible.

### `discord_set_permission_overwrite`

`channel_id`, `target_id`, `target_type` (`role` or `member`), and `allow` /
`deny` lists of Discord permission names such as `VIEW_CHANNEL` or
`SEND_MESSAGES`. `ADMINISTRATOR` cannot appear in `allow`: an overwrite must not
escalate privileges. An overwrite replaces the target's whole rule for that
channel, so an omitted list is sent as empty rather than "keep what is there".

### `discord_delete_permission_overwrite`

`channel_id`, `target_id`, `target_type`. Removes the target's overwrite, so the
channel falls back to its category and guild rules.

### `discord_create_thread`

`channel_id`, `name`, and optionally `from_message_id` (start from an existing
message), `private` (standalone threads only; a message thread follows its
message), `auto_archive_minutes` (60, 1440, 4320 or 10080). The parent channel
must be allowed.

### `discord_edit_thread`

`thread_id` plus `archived` and/or `locked`. The thread itself is rarely on the
allowlist — it did not exist when the list was written — so policy resolves its
parent channel and checks that instead.

### `discord_create_invite`

`channel_id`, and optionally `max_age_seconds` (0–604800; default 86400, 0 means
never), `max_uses` (0–100; 0 means unlimited), `temporary`, `unique`. The returned
invite code is a credential for the guild: treat it as one.

### `discord_delete_invite`

`code`. Policy resolves the invite's channel before spending an action, so a
code that points outside the allowlist is refused without a Discord mutation and
without consuming budget. The audit record for a revocation names that resolved
channel; only a refusal before resolution is recorded without a target.

## 5. Role control

Roles are guild-wide configuration, so every tool here needs
`policy.administrative_enabled = true`, its guild on `policy.guild_allowlist`, and
an action from the shared budget; each accepts `dry_run`. Permission names resolve
to Discord bits, and `ADMINISTRATOR` cannot be granted. Discord still enforces
role hierarchy and refuses managed roles; a refusal surfaces as
`PERMISSION_DENIED`.

### `discord_create_role`

`guild_id`, `name`, and optionally `permissions` (names such as `VIEW_CHANNEL`;
defaults to none), `color` (`0xRRGGBB`; `0` means none), `hoist`, `mentionable`.

### `discord_edit_role`

`guild_id`, `role_id`, plus at least one of `name`, `permissions`, `color`,
`hoist`, `mentionable`. `permissions` is a **replacement set**, not a diff:
sending it drops every permission not listed. It is how a role is made
administrator, so combine it with `dry_run` when the intent is not trivial.

### `discord_delete_role`

`guild_id`, `role_id`. Irreversible; members lose the role. `@everyone` is refused
locally because Discord does not allow deleting it.

### `discord_assign_role`

`guild_id`, `user_id`, `role_id`. Idempotent: assigning a role the member already
has is a no-op on Discord's side. `@everyone` cannot be assigned.

### `discord_remove_role`

`guild_id`, `user_id`, `role_id`. Idempotent. `@everyone` cannot be removed.

## 6. Resource

| URI | MIME type | Contents |
|---|---|---|
| `discord://guilds` | `application/json` | The same page `discord_list_guilds` returns with default arguments |

Resources have no per-result error flag, so a refusal on this resource is a
JSON-RPC error rather than an `isError` result. Clients that can call tools should
prefer the tool.

---

## 7. Error codes

The text of a failed result is `CODE: message`. The code is stable; the message
is not, and a client should branch on the code.

| Code | Meaning | Retryable |
|---|---|---|
| `INVALID_ARGUMENT` | An ID, limit or cursor failed validation; a `before`+`after` pair; empty or oversized content | No — fix the call |
| `POLICY_BLOCKED` | Local policy refused: feature flag, guild or channel allowlist, unclassified tool | No |
| `AUTHENTICATION_FAILED` | Discord rejected the bot token | No — fix the token |
| `PERMISSION_DENIED` | The bot lacks the Discord permission, intent or channel access | No |
| `RESOURCE_NOT_FOUND` | Unknown or invisible guild, channel, message or member | No |
| `RATE_LIMITED` | A `429` arrived with no usable rate-limit headers, so there was nothing to wait out | Yes, later |
| `TIMEOUT` | A deadline passed: the server's per-call deadline, or the transport timing out | The server's own deadline ends the call; a transport timeout is retried for reads |
| `NETWORK_ERROR` | The request never reached Discord, or the body could not be read | Retried for reads, except an unreadable body |
| `DISCORD_API_ERROR` | A Discord failure that maps to no narrower category, including any `5xx` | Retried for reads when it is a `5xx` |
| `CONFIGURATION_ERROR` | Configuration is missing, unparsable or inconsistent | No — fix the config |
| `INTERNAL_ERROR` | A local bug or unexpected failure; details are in the redacted logs | No |

## 8. Retries, rate limits and timeouts

- **Reads** retry only transport-level failures — a request that never completed,
  a transport timeout, a `5xx` — with jittered backoff, at most
  `discord.max_retries` times (default 3, so four attempts at most). An unreadable
  body, a `4xx`, a validation error and a permission error are never retried.
- **Rate limits sit below that budget.** twilight reads `Retry-After`, waits for
  the bucket and resends; the wait, like the request, is bounded by
  `discord.request_timeout_seconds`, so a long limit surfaces as `TIMEOUT`.
- **Writes** are never retried after an ambiguous outcome (see above).
- **Mutations share one budget.** `policy.autonomy.max_actions_per_minute`
  (default 30) counts applied mutations in a rolling minute across every tool; the
  action past the budget is `POLICY_BLOCKED` and no request is made. Dry runs do
  not consume it.
- Each attempt, including a rate-limit wait inside it, is bounded by
  `discord.request_timeout_seconds` (default 20). Over HTTP the whole request is
  additionally bounded by `http.request_timeout_seconds` (default 30), and a
  request beyond `http.max_in_flight_requests` gets `503` with `Retry-After`
  instead of queueing.
- Every tool call has a correlation id in the logs (`correlation`), shared by the
  audit records of the Discord requests it makes. Set `RUST_LOG=debug` to see the
  `mcp.tool` and `discord.call` spans.

## 9. What this surface does not do

- No member, webhook or emoji management yet; those are the next packs.
- No gateway connection, event stream or scheduled job: every read is
  request/response.
- No search endpoint; results carry no embeds, reactions or attachments.
- No variant of `discord_send_message` for embeds, files or replies; it sends plain
  text only.
