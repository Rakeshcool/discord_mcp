# Tool reference

Every tool this build exposes, with its arguments, its result shape, the policy
that applies to it and the errors it can return.

This build has **eight read tools and one write tool**, plus one resource. The
tool list is fixed: tools are registered unconditionally, and a call that policy
refuses comes back as a `POLICY_BLOCKED` result rather than a tool that vanishes
from `tools/list`. That way a client can discover what a deployment would allow
and see the refusal reason in the same place.

[`SETUP.md`](SETUP.md) covers installing the server; [`SECURITY.md`](SECURITY.md)
covers the threat model.

---

## 1. Calling convention

- Arguments are a JSON object. Unknown fields are rejected (`INVALID_ARGUMENT`),
  so a typo cannot silently fall back to a default.
- Every ID is a **decimal string**, never a JSON number:
  `"1572365819281932408"`, not `1572365819281932408`. Discord snowflakes exceed
  the range JavaScript numbers represent exactly.
- A successful result is one `text` content block containing JSON. There is no
  `structuredContent`; parse the text.
- A failed result is `isError: true`, and its text is `CODE: message`, where
  `CODE` is one of the [stable error codes](#5-error-codes). The message is
  written by this server, never copied from Discord, and never contains a token
  or a message body.
- Annotations say what a tool does to the world. The eight read tools carry
  `readOnlyHint: true`. `discord_send_message` carries `readOnlyHint: false`,
  `destructiveHint: false`, `idempotentHint: false` and `openWorldHint: true` —
  it creates a new message each time, which is why it is not idempotent.

### Policy in front of every tool

| Setting | Effect on tools |
|---|---|
| `policy.read_enabled = false` | Every read tool refuses with `POLICY_BLOCKED` |
| `policy.write_enabled = false` (default) | `discord_send_message` refuses with `POLICY_BLOCKED` |
| `policy.guild_allowlist` | Guild tools refuse unlisted guilds; channel tools resolve the channel's guild first (one extra request) and refuse it when unlisted; a direct-message channel is refused because its guild cannot be shown to be allowed |
| `policy.channel_allowlist` | Channel tools refuse unlisted channels; required to enable writes |
| `policy.max_messages_per_read` | Ceiling for `discord_get_messages.limit` |
| `policy.max_message_characters` | Ceiling for `discord_send_message.content`, counted in characters |
| `policy.max_concurrent_requests` | Caps in-flight Discord requests; calls queue rather than fail |

`discord_list_guilds` filters its results to allowlisted guilds instead of
refusing, so a guild allowlist cannot be inferred from the listing.

---

## 2. Read tools

### `discord_list_guilds`

Guilds the bot is a member of, ascending by ID.

| Argument | Type | Required | Notes |
|---|---|---|---|
| `limit` | integer | no | 1–200, default 200 (Discord's maximum) |
| `after` | string | no | Guild ID cursor from a previous page's `next_cursor` |

```json
{"items":[{"id":"1572365819281932408","name":"test_mcp_server","icon_hash":null,"owner":false}],"next_cursor":null}
```

`next_cursor` is `null` when the page was not full, otherwise the last item's ID.
Pass it back as `after`. Allowlisted-only filtering happens after paging, so a
page can come back shorter than `limit`.

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

Channels in a guild that the bot can see, in Discord's order.

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

- `content` is `null` when the bot may not read it: with the **Message Content**
  intent off, Discord blanks every message the bot did not send itself. A page of
  `content: null` is the intent, not a broken tool.
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

Returns one object of the shape shown above. A message the bot cannot see is
`RESOURCE_NOT_FOUND`, the same answer as a nonexistent one.

### `discord_list_members`

Guild members, ascending by user ID.

| Argument | Type | Required | Notes |
|---|---|---|---|
| `guild_id` | string | yes | Guild ID |
| `limit` | integer | no | 1–1000, default 100 |
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

## 3. Write tool

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

- **Mentions are neutralised.** `@everyone`, `@here`, `<@id>`, `<@!id>` and
  `<@&id>` have a zero-width space inserted after the `@`, so Discord neither
  resolves nor notifies them. The request also carries `allowed_mentions:
  {parse: []}`, so there are two independent layers.
- **Length is an authority limit**, not a suggestion: too many characters is
  `INVALID_ARGUMENT` before any request is built.
- **One audit record per attempt**, to stderr: `allowed`, `denied` (policy) or
  `failed` (Discord rejected it, with the category). It carries the action,
  channel ID, outcome and the calling tool call's correlation id — never the
  content.
- **Never retried.** A retried `POST` could create the message twice, so a
  timeout or a `500` surfaces as an error with one attempt made. A `429` is the
  exception: Discord rejected that attempt, so the wait is honoured and the
  request resent once the bucket allows it.
- **`403` is `PERMISSION_DENIED`**: the bot lacks Send Messages, or cannot see
  the channel. Granting it is a Discord-side change (Developer Portal install
  URL or the channel's permission overwrites), not a configuration one.

---

## 4. Resource

| URI | MIME type | Contents |
|---|---|---|
| `discord://guilds` | `application/json` | The same page `discord_list_guilds` returns with default arguments |

Resources have no per-result error flag, so a refusal on this resource is a
JSON-RPC error rather than an `isError` result. Clients that can call tools should
prefer the tool.

---

## 5. Error codes

The text of a failed result is `CODE: message`. The code is stable; the message
is not, and a client should branch on the code.

| Code | Meaning | Retryable |
|---|---|---|
| `INVALID_ARGUMENT` | An ID, limit or cursor failed validation; a `before`+`after` pair; empty or oversized content | No — fix the call |
| `POLICY_BLOCKED` | Local policy refused: feature flag, guild or channel allowlist, unclassified tool | No |
| `AUTHENTICATION_FAILED` | Discord rejected the bot token | No — fix the token |
| `PERMISSION_DENIED` | The bot lacks the Discord permission, intent or channel access | No |
| `RESOURCE_NOT_FOUND` | Unknown or invisible guild, channel, message or member | No |
| `RATE_LIMITED` | Discord kept rate limiting after the retry budget | Yes, later |
| `TIMEOUT` | The request or operation exceeded its deadline | Yes, for reads |
| `NETWORK_ERROR` | The request never reached Discord, or the response was unreadable | Yes, for reads |
| `DISCORD_API_ERROR` | A Discord failure that maps to no narrower category | No |
| `CONFIGURATION_ERROR` | Configuration is missing, unparsable or inconsistent | No — fix the config |
| `INTERNAL_ERROR` | A local bug or unexpected failure; details are in the redacted logs | No |

## 6. Retries, rate limits and timeouts

- **Reads** retry only transient failures — network errors, timeouts and `5xx` —
  with jittered backoff, at most `discord.max_retries` times (default 3). `429`
  waits for `Retry-After` and then resends. Validation and permission failures are
  never retried.
- **Writes** are never retried after an ambiguous outcome (see above).
- A single Discord call is bounded by `discord.request_timeout_seconds` (default
  20). Over HTTP, the whole request is additionally bounded by
  `http.request_timeout_seconds` (default 30), and a request beyond
  `http.max_in_flight_requests` gets `503` with `Retry-After` instead of queueing.
- Every tool call has a correlation id in the logs (`correlation`), shared by the
  audit records of the Discord requests it makes. Set `RUST_LOG=debug` to see the
  `mcp.tool` and `discord.call` spans.

## 7. What this surface does not do

- No edit, delete, react, pin, thread or bulk-delete tool. Nothing destructive is
  registered, so there is no tool that needs an approval flow.
- No gateway connection, no event stream of new messages, no scheduled jobs. All
  reads are request/response.
- No search endpoint, no reactions or embeds in results, no user (DM) token, no
  self-bot path.
- No `discord_send_message` variant for embeds, files or replies; the write tool
  sends plain text only.
