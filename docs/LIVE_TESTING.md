# Live testing record

This is a historical record. Current tool behavior is documented in
`docs/TOOLS.md`; current project status and open items are in `TASKS.md`.

## Runs

- **2026-11-18:** MCP bridge over stdio and Streamable HTTP, using the
  server-control build available at the time.
- **2026-11-20:** Message-analysis scans using the 40-tool build.

Test guild: `test_mcp_server` (`1572365819281932408`).

Bot: `mcp-test-bot` (`1572386219596316742`).

## Results

| Area | Result |
|---|---|
| Read tools | All read tools available at the time passed through the bridge. Message text was withheld on that run because the Message Content intent was off. |
| Message writes | Send, edit, bot-owned delete and reactions passed. Clear-reactions, pin/unpin and bulk delete initially returned `403` because the bot lacked Manage Messages and `PIN_MESSAGES`; the pin descriptions were corrected. |
| Structure | Permission overwrite, thread and invite operations passed. Channel create/edit initially returned `403` without Manage Channels; those paths passed after the permission was granted. |
| Roles | Create, edit, assign, remove and delete passed for a temporary role. Above-bot and `@everyone` operations were refused as expected. |
| Policy and budget | Channel allowlists, dry runs, budget exhaustion, `require_dry_run`, audit records and `--read-only` behaved as documented. |
| HTTP | Missing/wrong token returned `401`; foreign `Host`/`Origin` returned `403`; authenticated health and bridge calls passed. |
| Local bridge | Handshake, `tools/list`, direct calls and reconnect after idle stop passed. Tool names use `discord-mcp__<tool>`. |

At the time of the first run the bridge reported 32 tools. The later
message-analysis run used the current 40-tool build.

## Message-analysis scans

Probes were posted in `#support` (`1572731346550526005`): two attachment
messages, two text-only messages, two `idiot` messages and two `shut up`
messages.

| Check | Result |
|---|---|
| `has_attachments: true` | 2 attachment messages; `support.matched: 2`; 6 requests |
| `has_attachments: false` | 20 non-attachment messages, including all six text probes |
| Default abuse scan | 5 hits: four bot probes plus an existing second-user `shut up dude` message in `#testing_reaading`; grouped by author |
| Denied Read Message History | `#support` returned an empty page, not a per-channel error; the scan continued |
| Direct read on denied `#support` | `items: []` |

The denied-history result is a Discord behavior, not a scan failure. It is
documented as an empty page in `docs/TOOLS.md` and `docs/SETUP.md`.

## Findings fixed

- Pin/unpin descriptions now name `PIN_MESSAGES`.
- The `ADMINISTRATOR` refusal now names the caller's field.
- `@everyone` refusals now say “deleted, assigned or removed”.
- Message results omit `guild_id`; clients resolve it from the channel.
- A locked thread cannot be unlocked by its creator without Manage Threads.
- Invite revocation can return `404/10006` while the invite still resolves.
- The bridge can idle-stop during a long batch; reconnecting resumes it.

## Cleanup

Verified against the test guild on 2026-11-20 by querying channels, roles,
active and archived threads, invites, and `#support` history.

- All probe messages were deleted.
- The temporary role and permission overwrite were removed; neither remains.
- One archived private test thread remains: `mcp-live-thread-1795036916` in
  `#general`, archived 2026-11-18T21:23:34Z and unlocked. It is not active.
- One active invite remains in `#general`, created 2026-11-18T22:36:29Z and
  expiring 2026-12-18T22:36:29Z, with one recorded use. The earlier claim that
  it had expired was wrong.
- `#support` has its four fixture messages.
- The bridge was restored to built-in read-only `serve`.

## Current gaps

- Live rich send: embed, file, reply and reply-ping check.
- Positive role CRUD below the bot's highest role.
- Cross-user Manage Messages paths.
- Managed-role delete and permission-edit refusal with a second bot role.
- The bridge's `llama-server` chat loop.
