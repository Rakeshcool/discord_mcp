# Tool reference

Every tool this build exposes, with its arguments, its result shape, the policy
that applies to it and the errors it can return.

This build has **eleven read tools and twenty-nine mutations** — seven message
writes, ten structure/control tools, six member-moderation tools and six
administrative tools — plus one resource. Tools are registered unconditionally,
so a call that policy refuses comes back as a `POLICY_BLOCKED` result rather
than a tool that vanishes from `tools/list`.

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
  `CODE: message`, where `CODE` is one of the [stable error codes](#8-error-codes).
  The only exception is an argument object that does not deserialize (above). The
  message is written by this server, never copied from Discord, and never contains
  a token or a message body.
- Annotations say what a tool does to the world. Read tools carry
  `readOnlyHint: true`. Every mutation carries `readOnlyHint: false` and
  `openWorldHint: true`; 20 of them also carry `destructiveHint: true` — the two
  message deletions, `discord_clear_reactions`, every structure tool, every role
  tool, and member kick, ban and timeout. `idempotentHint` is `false` where a
  repeat is not a no-op: `discord_send_message`, `discord_create_channel`,
  `discord_delete_channel`, `discord_create_thread`, `discord_create_invite`,
  `discord_create_role`, `discord_delete_role`, `discord_kick_member`,
  `discord_ban_member` and `discord_timeout_member`.
- Every mutation except `discord_send_message` accepts `dry_run`. A dry run runs
  every argument and local policy check, then returns
  `{"dry_run":true,"action":"…"}` without sending anything, without consuming
  the action budget and without writing an audit record.

### Policy in front of every tool

| Setting | Effect on tools |
|---|---|
| `policy.read_enabled = false` | Every read tool refuses with `POLICY_BLOCKED` |
| `policy.write_enabled = false` (default) | `discord_send_message`, `discord_edit_message`, the reaction tools and the pin tools refuse with `POLICY_BLOCKED` |
| `policy.control_enabled = false` (default) | `discord_delete_message` and every channel, permission, thread and invite tool refuse; enabling it requires a channel allowlist |
| `policy.moderation_enabled = false` (default) | `discord_set_member_nickname`, kick, ban, unban and timeout tools refuse; enabling it requires a guild allowlist |
| `policy.administrative_enabled = false` (default) | `discord_bulk_delete_messages` and every role tool refuse; enabling it requires a guild allowlist |
| `policy.autonomy.max_actions_per_minute` (default 30) | Shared budget across all mutations, including sends; the request past it is `POLICY_BLOCKED` and is never sent |
| `policy.autonomy.max_sends_per_channel_per_minute` (default 5) | Messages one channel may receive per rolling minute; the next send is `POLICY_BLOCKED` |
| `policy.autonomy.budget_state_file` | File carrying the rolling windows across restarts; omitted means `%LOCALAPPDATA%\discord-mcp\action-budget.json`. Exactly one running process may own it; a second instance with mutations enabled fails startup, and read-only instances do not open it |
| `policy.autonomy.max_bulk_delete` (default 50) | Ceiling for one `discord_bulk_delete_messages` call, on top of Discord's 100 and 14-day rules |
| `policy.autonomy.require_dry_run` | Tools listed here refuse unless called with `dry_run: true`; a name that is not a classified tool is a configuration error, not a silent no-op |
| `policy.guild_allowlist` | When non-empty: guild tools refuse unlisted guilds, channel tools resolve the channel's guild first (one extra request) and refuse it when unlisted, and a direct-message channel is refused because its guild cannot be shown to be allowed |
| `policy.channel_allowlist` | When non-empty: channel tools refuse unlisted channels. Enabling writes requires it to be non-empty |
| `policy.protected_channels` | Channels no tool may edit or delete, even when allowlisted. Refused before the budget; a refusal spends no action |
| `policy.protected_roles` | Roles no tool may edit or delete, even when allowlisted. Refused before the budget |
| `policy.max_messages_per_read` | Ceiling for `discord_get_messages.limit` and for each page of `discord_get_user_messages` and `discord_find_abusive_messages`; the scan's per-channel default |
| `policy.max_message_characters` | Ceiling for `discord_send_message.content`, counted in characters |
| `policy.max_embeds_per_message` | Embeds allowed on one send (default 10) |
| `policy.max_embed_characters` | Total characters across every embed on one send (default 6000) |
| `policy.max_attachments_per_message` | Files allowed on one send (default 10) |
| `policy.max_attachment_bytes` | Ceiling for one uploaded file (default 8 MiB) |
| `policy.max_attachment_total_bytes` | Ceiling for all uploaded files on one send (default 16 MiB) |
| `policy.attachment_root` | Directory uploads must stay beneath; unset disables file uploads |
| `policy.max_concurrent_requests` | Caps in-flight Discord requests; calls queue rather than fail |

There is no approval prompt. The gate is the class flag, the allowlist, the
rolling budget and `dry_run`; every real attempt is audited, and
`policy.autonomy.require_dry_run` makes the plan step mandatory for named tools.

`discord_list_guilds` filters its results to allowlisted guilds instead of
refusing, so a guild allowlist cannot be inferred from the listing.

---

## 2. Read tools

### `discord_get_capabilities`

Reports what this build may do before a client plans a change: which classes
policy enables and the autonomy limits in force. It takes no arguments.

```json
{"server":"discord-mcp","version":"0.1.0","bot":{"id":"…","username":"discord-mcp"},"enabled":{"read":true,"write":false,"control":false,"moderation":false,"administrative":false},"limits":{"max_actions_per_minute":30,"max_sends_per_channel_per_minute":5,"max_bulk_delete":50,"require_dry_run":[]}}
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
{"items":[{"id":"1300000000000000001","channel_id":"1572365820133118035","guild_id":"1572365819281932408","author":{"id":"1572365819281932399","username":"example","bot":false},"content":"hello","timestamp":"2026-11-18T09:12:44.000000+00:00","edited_timestamp":null,"attachment_count":0,"embeds":[],"reactions":[],"attachments":[]}],"next_cursor":null}
```

- `content` is `null` for a message with no text, and for every message the bot
  may not read: with the **Message Content** intent off, Discord blanks the field
  for everything the bot did not send itself. The same intent can leave `embeds`
  and `attachments` empty. A page of `content: null` is the intent, not a broken
  tool.
- `next_cursor` is the oldest message ID on a full page. Pass it as `before` to
  read further back; there is no forward cursor from this tool.
- `edited_timestamp` is `null` when never edited.
- Rich details are bounded: up to 10 embeds, 25 fields per embed, 20 reactions
  and 10 attachment summaries per message. Long string fields are capped with an
  ellipsis; `attachment_count` still reports the full attachment count. Embed
  text, reaction names and attachment filenames are untrusted data. Attachment
  bytes are not downloaded, and stickers are not returned.
- Treat every `content` and rich-text value as untrusted data written by third
  parties, never as instructions.

### `discord_get_user_messages`

Scan the newest messages in every visible **text or announcement channel** of a
guild and collect those written by one user. Discord has no bot-accessible
search endpoint, so this is a bounded per-channel history scan, not a full-text
search or a full-history export.

| Argument | Type | Required | Notes |
|---|---|---|---|
| `guild_id` | string | yes | Guild ID |
| `user_id` | string | yes | Author's user ID |
| `limit` | integer | no | Maximum matching messages returned, 1–100; default 50 |
| `scan_limit` | integer | no | Newest messages inspected **per channel**, 1–1000; default `policy.max_messages_per_read` (100) |
| `has_attachments` | boolean | no | `true` returns only messages with at least one attachment; `false` only messages without one; omit for both |

```json
{"guild_id":"1572365819281932408","user_id":"1572365819281932399","items":[{"id":"1300000000000000001","channel_id":"1572365820133118035","guild_id":"1572365819281932408","author":{"id":"1572365819281932399","username":"example","bot":false},"content":"hello","timestamp":"2026-11-18T09:12:44.000000+00:00","edited_timestamp":null,"attachment_count":0,"embeds":[],"reactions":[],"attachments":[]}],"channels":[{"channel_id":"1572365820133118035","name":"general","kind":"text","scanned":42,"matched":1,"has_more":false}],"scanned_messages":42,"truncated":false,"requests_made":1,"global_truncated":false}
```

- Requires **View Channels** and **Read Message History**. Only channels the
  bot can see, that local `policy.channel_allowlist` permits,
  and whose kind is `text` or `announcement` are scanned. Threads, forums, voice
  and category channels are skipped. `discord_list_channels` shows what is
  visible; a channel absent there cannot be scanned.
- `scan_limit` is per channel, and pages are bounded by
  `policy.max_messages_per_read`. A scan issues at most **50** history requests;
  `discord_get_user_messages` does not use the write action budget.
- Each matching message has the same shape as a `discord_get_messages` item,
  including the same bounded rich fields, and is subject to the same
  `content: null` caveat. `has_attachments` filters on `attachment_count`, so it
  still works for messages whose attachment metadata was truncated.
- `channels` reports what each attempted channel contributed. `matched` counts
  matching messages seen in that channel's scanned window, even if the global
  result cap later dropped one. `has_more` means the per-channel scan stopped at
  `scan_limit`, or the global request cap stopped it, while older messages may
  remain. A channel that returns `PERMISSION_DENIED`, `RESOURCE_NOT_FOUND` or
  another error appears with its `error` string and does not abort the other
  channels. Discord can instead answer a channel without Read Message History
  with a successful empty page; that channel appears with `scanned: 0` and no
  `error`, so inspect its permissions separately.
- `truncated` is `true` when more matching messages were found than the global
  `limit` could return. The scan keeps visiting eligible channels past that
  result cap, until every channel is visited or the request cap is reached.
- `requests_made` reports how many channel-history requests the call issued,
  from 0 to 50. `global_truncated` is `true` when the 50-request cap was reached
  before every eligible channel was visited or before a channel's `scan_limit`
  was exhausted. Channels not listed in `channels` were not examined.
- This is not an exhaustive history search. Matching messages are ordered
  newest-first across channels, and `limit` bounds the returned set, not how many
  channels are scanned.

### `discord_find_abusive_messages`

Scan the newest messages in every visible **text or announcement channel** and
return messages containing configured abuse or harassment terms, plus an
author-level summary. This is keyword detection for review, not a finding that a
user violated a rule and not an automatic moderation action.

| Argument | Type | Required | Notes |
|---|---|---|---|
| `guild_id` | string | yes | Guild ID |
| `keywords` | array of strings | no | 1–25 terms, each at most 64 characters; replaces the built-in conservative list |
| `limit` | integer | no | Maximum matching messages returned, 1–100; default 50 |
| `scan_limit` | integer | no | Newest messages inspected **per channel**, 1–1000; default `policy.max_messages_per_read` (100) |

```json
{"guild_id":"1572365819281932408","keywords":["idiot","shut up"],"items":[{"id":"1300000000000000002","channel_id":"1572365820133118035","guild_id":"1572365819281932408","author":{"id":"1572365819281932399","username":"example","bot":false},"content":"shut up, idiot","timestamp":"2026-11-18T09:12:44.000000+00:00","edited_timestamp":null,"attachment_count":0,"embeds":[],"reactions":[],"attachments":[],"matched_keywords":["idiot","shut up"]}],"users":[{"user_id":"1572365819281932399","username":"example","matched_messages":1,"matched_keywords":["idiot","shut up"],"channel_ids":["1572365820133118035"]}],"channels":[{"channel_id":"1572365820133118035","name":"general","kind":"text","scanned":42,"matched":1,"has_more":false}],"scanned_messages":42,"truncated":false,"requests_made":1,"global_truncated":false}
```

- The default list is deliberately conservative: `idiot`, `stupid`, `moron`,
  `dumbass`, `loser`, `worthless`, `pathetic`, `useless`, `shut up`,
  `nobody likes you`, `hate you`, `kill yourself`, `kys`, `go die`,
  `you're dead`, `youre dead`, `i will hurt you`, `i'll hurt you` and
  `watch your back`. Supplying `keywords` replaces it; use the caller's list when
  the community's language or policy requires different terms.
- Matching is case-insensitive and word-bounded: `idiot` matches `IDIOT!` but not
  `idiotic`, and whitespace is collapsed so `shut   up` matches `shut up`.
  Keyword matching does not understand context, sarcasm, quotation, or who the
  target was. Treat every hit as a review signal and read the surrounding
  messages before drawing a conclusion.
- The scan uses the same eligible-channel, allowlist, per-channel `scan_limit`
  and **50-request** cap as `discord_get_user_messages`. It skips threads,
  forums, voice and category channels, and a per-channel failure appears in
  `channels` without aborting the rest of the scan. Discord may return an empty
  page, rather than an error, when it withholds Read Message History; that case
  appears as `scanned: 0` and should be checked against channel permissions.
- `items` is ordered newest-first and bounded by `limit`; `matched_keywords`
  preserves the configured term order. `users` aggregates the returned items by
  author, highest match count first, with usernames, matched terms and channel
  IDs. `channels[].matched` counts all matching messages seen in that channel's
  scanned window, even when the global result cap drops one; `truncated` and
  `global_truncated` report those limits.
- Requires **View Channels**, **Read Message History**, and the **Message Content
  intent** for message bodies. Without that intent Discord returns no content, so
  the scan can produce no matches even though messages exist.

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
when a guild allowlist narrows scope, action budget, one audited attempt. Every
one consumes the shared budget; `discord_send_message` additionally consumes its
channel's send budget. None is retried by this server.

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
| `content` | string | no | 1–`policy.max_message_characters` characters (default 2000); mentions are neutralised. Required unless `embeds` or `files` carries the message |
| `embeds` | array | no | Up to `policy.max_embeds_per_message` rich embeds, bounded by `policy.max_embed_characters` |
| `files` | array of strings | no | Relative paths beneath `policy.attachment_root`; disabled when that setting is unset |
| `reply_to` | string | no | Message ID in the same channel to reply to |

Each embed object accepts `title`, `description`, `url`, `color`, `fields`,
`footer`, `image_url` and `thumbnail_url`. A field is `{name, value, inline?}`.
Titles are capped at 256 characters, descriptions at 4096, fields at 25 with
256-character names and 1024-character values, footers at 2048, and URLs at
2048. `url` must be http(s); image and thumbnail URLs may also be
`attachment://filename`. Discord's 6000-character total is enforced through
`policy.max_embed_characters`.

File uploads are off by default. Set `policy.attachment_root` to a directory and
pass relative paths beneath it; absolute paths, `..`, directories and symlinks
that resolve outside the root are refused. Up to
`policy.max_attachments_per_message` files are accepted, each at most
`policy.max_attachment_bytes`, with `policy.max_attachment_total_bytes` across
the send. File contents are read only when the request is made and are never
logged or returned.

Returns the created message, in the same shape as `discord_get_message`:

```json
{"id":"1300000000000000099","channel_id":"1572365820133118035","guild_id":"1572365819281932408","author":{"id":"...","username":"discord-mcp","bot":true},"content":"deploy finished","timestamp":"2026-11-18T09:30:00.000000+00:00","edited_timestamp":null,"attachment_count":0,"embeds":[],"reactions":[],"attachments":[]}
```

What applies to every attempt:

- **The send is budgeted.** It consumes one action from the shared
  `max_actions_per_minute` window and one from the channel's
  `max_sends_per_channel_per_minute` window. Either limit refuses the send before
  a request is built, and both windows are persisted across restarts.
- **Mentions are neutralised.** `@everyone` and `@here` get a zero-width space
  after the `@`; user and role mentions get one after the `<`. Discord then
  neither resolves nor notifies them. The request also carries `allowed_mentions`
  with an empty `parse` list, so there are two independent layers.
- **Content and rich fields are authority limits**, not suggestions: too many
  characters, embeds, fields or attachment bytes is `INVALID_ARGUMENT` or
  `POLICY_BLOCKED` before any request is built.
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
action from the shared per-minute budget; each accepts `dry_run`. A channel on
`policy.protected_channels` refuses `discord_edit_channel`,
`discord_delete_channel` and both permission-overwrite tools, and a protected
thread refuses `discord_edit_thread`, regardless of the allowlist and before the
budget. A dry run
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
to Discord bits, and `ADMINISTRATOR` cannot be granted. A role on
`policy.protected_roles` refuses `discord_edit_role` and `discord_delete_role`
before the budget, even when its guild is allowlisted.

### Discord permissions and role hierarchy

Local policy is only half the gate: the bot must also hold **Manage Roles** in the
guild. The policy flag lets a tool run; Discord still refuses the request when
the permission is missing or the role is out of reach. That refusal surfaces as
`PERMISSION_DENIED` with Discord's `403` and error code `50013`
(*Missing Permissions*).

| Tool | Discord permission | Hierarchy requirement |
|---|---|---|
| `discord_create_role` | Manage Roles | None for creation; the role is created at the bottom |
| `discord_edit_role` | Manage Roles | Role must sit below the bot's highest role |
| `discord_delete_role` | Manage Roles | Role must sit below the bot's highest role |
| `discord_assign_role` | Manage Roles | Role must sit below the bot's highest role |
| `discord_remove_role` | Manage Roles | Role must sit below the bot's highest role |

Position decides as much as permission: a role at or above the bot's highest
role cannot be edited, deleted, assigned or removed, even with Manage Roles and
even with `Administrator`. For `discord_assign_role` and `discord_remove_role`
the rule is about the role, not the target member: a role below the bot's highest
role can be given to or taken from a member, including the server owner. When
creating or editing a role, the bot can only grant permissions the bot itself
holds.

`@everyone` is position 0: it can be edited, but cannot be deleted, assigned or
removed. Discord also refuses deleting a managed role or changing its permission
set.

A `dry_run` does not contact Discord, so it cannot detect a missing permission or
a hierarchy problem; the first real call is where the `403` appears.

The bot's **own highest role** is protected unconditionally: editing or deleting
it would strip the bot's access, and removing it from the bot's own membership
would do the same, so `discord_edit_role`, `discord_delete_role` and
`discord_remove_role` refuse it. There is no setting that turns the guard off.
Finding that role needs the bot's membership and the guild's role list, so the
guard runs on a real call and is skipped on a dry run, like the other
remote-scope checks. `discord_remove_role` only fetches the role list when the
target member is the bot itself.

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

`guild_id`, `user_id`, `role_id`. Idempotent. `@everyone` cannot be removed, and
the bot's own highest role cannot be removed from the bot.

## 6. Member moderation

These six tools need `policy.moderation_enabled = true`, their guild on
`policy.guild_allowlist`, and one action from the shared budget. Every one
accepts `dry_run`. The bot refuses to target itself on a real call; Discord still
decides the remaining hierarchy and permission checks, returning
`PERMISSION_DENIED` when the bot lacks the required permission or target rank.

| Tool | Arguments | Discord permission |
|---|---|---|
| `discord_set_member_nickname` | `guild_id`, `user_id`, `nickname`, `reason?`, `dry_run?` | `MANAGE_NICKNAMES` |
| `discord_kick_member` | `guild_id`, `user_id`, `reason?`, `dry_run?` | `KICK_MEMBERS` |
| `discord_ban_member` | `guild_id`, `user_id`, `delete_message_seconds?`, `reason?`, `dry_run?` | `BAN_MEMBERS` |
| `discord_unban_member` | `guild_id`, `user_id`, `reason?`, `dry_run?` | `BAN_MEMBERS` |
| `discord_timeout_member` | `guild_id`, `user_id`, `duration_seconds`, `reason?`, `dry_run?` | `MODERATE_MEMBERS` |
| `discord_clear_member_timeout` | `guild_id`, `user_id`, `reason?`, `dry_run?` | `MODERATE_MEMBERS` |

`nickname` is required. A non-empty nickname is trimmed to 1–32 characters; an
exactly empty string clears the nickname. `reason` is optional, capped at 512
characters, and rejects newlines and NUL because it becomes Discord's
`X-Audit-Log-Reason` header. `delete_message_seconds` is 0–604800 (7 days) and
defaults to 0. `duration_seconds` is 1–2419200 (28 days).

`discord_kick_member` removes the member but does not prevent a rejoin.
`discord_ban_member` can target a user ID that is not currently in the guild;
that is how a pre-emptive ban is expressed. `discord_unban_member` removes the
ban. `discord_timeout_member` sets the timeout relative to the server's current
time; `discord_clear_member_timeout` removes it. Timeouts and bans are subject to
Discord's hierarchy rules, so the guild owner and members at or above the bot
cannot be actioned.

The local policy checks run before the budget is spent and before any request is
built. A real call records exactly one content-free audit event with the guild,
user ID, action and outcome; the optional reason is sent to Discord but is not
put in the local audit record. A dry run reports the plan, sends nothing, spends
no budget, writes no audit event, and cannot discover a missing Discord
permission or hierarchy problem.

## 7. Resource

| URI | MIME type | Contents |
|---|---|---|
| `discord://guilds` | `application/json` | The same page `discord_list_guilds` returns with default arguments |

Resources have no per-result error flag, so a refusal on this resource is a
JSON-RPC error rather than an `isError` result. Clients that can call tools should
prefer the tool.

---

## 8. Error codes

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

## 9. Retries, rate limits and timeouts

- **Reads** retry only transport-level failures — a request that never completed,
  a transport timeout, a `5xx` — with jittered backoff, at most
  `discord.max_retries` times (default 3, so four attempts at most). An unreadable
  body, a `4xx`, a validation error and a permission error are never retried.
- **Rate limits sit below that budget.** twilight reads `Retry-After`, waits for
  the bucket and resends; the wait, like the request, is bounded by
  `discord.request_timeout_seconds`, so a long limit surfaces as `TIMEOUT`.
- **Writes** are never retried after an ambiguous outcome (see above).
- **Mutations share one budget.** `policy.autonomy.max_actions_per_minute`
  (default 30) is a rolling 60-second window shared by every mutation, sends
  included; the action past the budget is `POLICY_BLOCKED` and no request is
  made. Dry runs do not consume it. Sends additionally draw on
  `policy.autonomy.max_sends_per_channel_per_minute` (default 5), a separate
  window per channel. The windows are persisted to
  `policy.autonomy.budget_state_file` after each consumed action, so restarting
  the process cannot reset them; a state file that cannot be read is a startup
  error. The file has one live owner: a process with mutations enabled holds an
  exclusive lock, and a second process on the same path fails startup. Read-only
  processes do not open it. Each concurrent instance needs its own
  `budget_state_file`, and those windows are independent limits.
- Each attempt, including a rate-limit wait inside it, is bounded by
  `discord.request_timeout_seconds` (default 20). Over HTTP the whole request is
  additionally bounded by `http.request_timeout_seconds` (default 30), and a
  request beyond `http.max_in_flight_requests` gets `503` with `Retry-After`
  instead of queueing.
- Every tool call has a correlation id in the logs (`correlation`), shared by the
  audit records of the Discord requests it makes. Set `RUST_LOG=debug` to see the
  `mcp.tool` and `discord.call` spans.

## 10. What this surface does not do

- No webhook or emoji management yet; those are the next packs.
- No gateway connection, event stream or scheduled job: every read is
  request/response.
- No Discord search endpoint. `discord_get_user_messages` and
  `discord_find_abusive_messages` scan the newest messages in visible text and
  announcement channels, but cannot reach history older than their bounded
  per-channel scan. The abuse scan is keyword-based only; it does not classify
  context or prove that abuse occurred.
- Attachment bytes are not downloaded; reads return metadata and Discord URLs
  only. Stickers, components and poll details are not returned.
- Rich sends stop at embeds, files and replies: no stickers, polls, components,
  threads started from the send, or forwarding.
