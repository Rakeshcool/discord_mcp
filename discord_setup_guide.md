# Discord bot setup guide

This guide is for the account owner adding `discord-mcp` to a Discord server. It
covers the Discord Developer Portal, the bot invite, least-privilege
permissions, role hierarchy, channel overrides, and the outcomes to expect.

The server uses an official **bot** token. It does not support user tokens or
self-bots. Discord-side setup is manual; the local setup runbook is
[`instruction.md`](instruction.md).

## Before you start

You need:

- a Discord account that can manage the target server;
- access to the [Discord Developer Portal](https://discord.com/developers/applications);
- the repository checked out and a built `discord-mcp.exe`;
- the test server ID and channel IDs you intend to allow.

Start with read-only access. Add mutation permissions only when you deliberately
enable the matching local class.

## 1. Create the Discord application

1. Open <https://discord.com/developers/applications>.
2. Select **New Application**.
3. Name it, for example `discord-mcp`.
4. Accept the Developer Terms and select **Create**.

**Expected outcome:** the application opens on its **General Information** page.
The application ID is shown there; you will use it in the invite URL.

## 2. Add the bot and get its token

1. In the left sidebar, open **Bot**.
2. Select **Add Bot** if the application does not have a bot user yet.
3. Select **Reset Token**, then **Copy**.
4. Put the token in the local `.env` as `DISCORD_BOT_TOKEN`, or give it to the MCP
   host through its `env` block.
5. Never paste the token into chat, a screenshot, a commit, or a support issue.

The token is a password. Resetting it revokes the previous token immediately.

**Expected outcome:** `DISCORD_BOT_TOKEN` is present in the server process
environment. `--validate-config` reports `bot token: present` but never prints
the value.

**Exception:** if the token is missing, `serve` exits with a configuration
error. If the token is wrong or revoked, startup may succeed but tool calls
return `AUTHENTICATION_FAILED`. Reset the token and update the environment.

## 3. Enable the privileged intents

On the same **Bot** page, scroll to **Privileged Gateway Intents**.

| Intent | Enable when | If it is off |
|---|---|---|
| **Message Content** | Any tool must read message text, embeds or attachments | Discord returns `content: null` and empty rich fields, even in REST responses |
| **Server Members** | You use `discord_list_members` or `discord_get_member` | Member tools return `PERMISSION_DENIED` / Discord's permission error |

1. Toggle the intents you need.
2. Select **Save Changes**.
3. Confirm the dialog. Without confirmation the toggle is not applied.

Do not enable **Presence** for this server; it uses REST only and does not open a
gateway connection.

**Expected outcome:** message reads return real `content` and rich fields, and
member tools return member records.

**Exceptions:**

- **Message Content off:** reads do not fail. They return `content: null`, often
  with empty `embeds` and `attachments`. This is Discord withholding data, not a
  broken tool. Enable the intent and confirm the save dialog.
- **Server Members off:** member tools fail with a permission error; message
  reads can still work.
- Intents and permissions are different: permissions decide what the bot may do,
  intents decide what data Discord includes in responses. Both are required for
  message text.
- The bot may appear **offline** in the member list. That is expected for a
  REST-only server; it does not need a gateway connection for the tools to work.

## 4. Invite the bot with least privilege

1. Open **OAuth2 → URL Generator**.
2. Under **Scopes**, select **`bot` only**. Do not select
   `applications.commands`; this server registers no slash commands.
3. Under **Bot Permissions**, start with:
   - **View Channels**
   - **Read Message History**
4. Copy the generated URL, open it in a browser, select the target server, and
   choose **Authorize**.
5. Return to the server and confirm the bot appears in the member list.

The equivalent read-only invite URL, after replacing the application ID, is:

```text
https://discord.com/oauth2/authorize?client_id=<APPLICATION_ID>&scope=bot&permissions=66560
```

`66560` is `View Channels (1024) + Read Message History (65536)`.

**Expected outcome:** the bot joins the server and can see channels where
**View Channels** and **Read Message History** are allowed. The invite itself
grants no send, delete, moderation, or management permissions; any extra access
would come from `@everyone` or a channel overwrite.

**Exceptions:**

- If the bot does not appear, the invite was cancelled or the account lacks
  **Manage Server**. Re-run the invite with an account that can manage the
  server.
- If the bot joins but sees no channels, check channel-level overwrites and
  **View Channels**.
- `Administrator` is never required. Do not grant it to make an error disappear;
  local policy can only narrow Discord permissions, never widen them.

## 5. Add permissions for the features you enable

Grant these through the bot's role under **Server Settings → Roles**, or through
channel overwrites. Keep the bot's permissions limited to the classes you
actually use.

| Feature | Discord permission | Notes |
|---|---|---|
| List guilds / get guild metadata | Bot membership | No channel permission required |
| List/get channels | View Channels | Channel visibility and metadata |
| Read message history | View Channels, Read Message History | `discord_get_messages`, `discord_get_message` |
| Read or scan message text | View Channels, Read Message History; Message Content intent | `discord_get_user_messages`, `discord_find_abusive_messages` |
| Capability report | None | Still gated locally by `read_enabled` |
| Member lookup/list | Server Members intent | No Manage Members permission required |
| Send a message | Send Messages | Send, edit and reply to messages |
| Edit another author's message | Manage Messages | The bot can edit its own messages with Send Messages |
| Delete, bulk delete, clear reactions | Manage Messages | Includes `discord_delete_message`, `discord_bulk_delete_messages`, `discord_clear_reactions` |
| Add or remove reactions | Add Reactions | Reaction tools |
| Pin or unpin | Pin Messages (`PIN_MESSAGES`) | Current Discord permission; older API behaviour used Manage Messages |
| Create/edit/delete channels and overwrites | Manage Channels | Add Manage Roles when changing a role or member overwrite |
| Create and edit threads | Create Public Threads, Create Private Threads, Manage Threads, Send Messages in Threads | The tool's thread type determines which create permission is needed |
| Create an invite | Create Instant Invite | `discord_create_invite` |
| Delete an invite | Manage Channels on that channel, or Manage Guild | `discord_delete_invite` |
| Change nicknames | Manage Nicknames | `discord_set_member_nickname` |
| Kick members | Kick Members | `discord_kick_member` |
| Ban or unban members | Ban Members | `discord_ban_member`, `discord_unban_member` |
| Timeout or clear timeout | Moderate Members | `discord_timeout_member`, `discord_clear_member_timeout` |
| Create a role | Manage Roles | The new role is created at the bottom; no hierarchy precondition |
| Edit/delete/assign/remove roles | Manage Roles | Target role must be below the bot's highest role |

**Expected outcome:** a tool with the needed permission succeeds; a tool whose
permission is missing returns `PERMISSION_DENIED` with Discord's `403` and usually
code `50013`. The server does not retry a refused mutation.

**Exceptions:**

- **Local policy can still refuse the call.** A Discord permission does not
  bypass `policy.write_enabled`, the allowlist, the budget, protected IDs, or
  `dry_run`. A `POLICY_BLOCKED` result means local configuration, not Discord.
- **Pin requires `PIN_MESSAGES`.** If only Manage Messages is granted, pin and
  unpin can still return `403 / 50013`.
- Channel overwrites and role hierarchy are checked in steps 6 and 7; a role
  grant alone does not override either.

## 6. Set the bot's role position

Discord checks role hierarchy for role tools and member moderation:

1. Open **Server Settings → Roles**.
2. Drag the bot's role **above every role it must edit, assign, remove, or
   moderate**.
3. Keep the bot's own highest role above the roles it manages.

**Expected outcome:** the bot can manage roles and members below its highest
role.

**Exceptions:**

- `discord_create_role` has no hierarchy precondition; Discord creates the new
  role at the bottom.
- Role creation and editing can only grant permissions the bot itself holds.
  `ADMINISTRATOR` cannot be granted through a role or a channel overwrite.
- An existing role at or above the bot's highest role cannot be edited, deleted,
  assigned, or removed, even with Manage Roles. Discord returns `403 / 50013`.
- `@everyone` can be edited but cannot be deleted, assigned, or removed.
- A managed role (for example, a bot or integration role) cannot be deleted or
  have its permission set changed.
- The bot's own highest role is protected unconditionally; it cannot be edited,
  deleted, or removed from the bot.
- The bot cannot moderate itself. Kick, ban, and timeout actions are subject to
  hierarchy, so the server owner and members at or above the bot cannot be
  actioned.

## 7. Check channel visibility and overwrites

For each channel the bot should use:

1. Confirm the bot's role has **View Channels** and **Read Message History**.
2. Open the channel's **Edit Channel → Permissions**.
3. Check the bot or its role for an explicit deny. An explicit deny overrides the
   role grant.
4. For private channels, add the bot or its role explicitly.

**Expected outcome:** the channel appears in `discord_list_channels`, messages
can be read, and mutations target it only when its ID is in the local
`channel_allowlist`.

**Exceptions:**

- A channel missing from `discord_list_channels` cannot be read or scanned.
- A channel with View Channel but without Read Message History may return an
  empty message page instead of an error.
- A channel outside `policy.channel_allowlist` is refused with `POLICY_BLOCKED`
  even when Discord allows the bot to use it.

## 8. Finish the local configuration

1. Create `.env` from the example if it does not exist:

   ```powershell
   Copy-Item .env.example .env
   ```

2. Put the bot token in `DISCORD_BOT_TOKEN`. Do not print the file.
3. Create a local config if you need mutations:

   ```powershell
   Copy-Item config\default.toml config\local.toml
   ```

4. Set the allowlists:
   - `policy.channel_allowlist` for write and control tools;
   - `policy.guild_allowlist` for moderation and administrative tools.
5. Leave `write_enabled`, `control_enabled`, `moderation_enabled`, and
   `administrative_enabled` at `false` until the matching allowlist is set.
6. Reuse the `$exe` path set in `instruction.md` (or set it again to that same
   artifact), then validate without starting:

   ```powershell
   & $exe --validate-config --config config\local.toml
   ```

**Expected outcome:** exit code `0`, a valid configuration message, and the
effective transport and token-presence status on stderr.

**Exceptions:**

- Enabling write or control without `channel_allowlist` is a configuration error.
- Enabling moderation or administration without `guild_allowlist` is a
  configuration error.
- `--access` only enables classes for one process; it does not remove the
  allowlist requirements.
- `--read-only` and `DISCORD_MCP_READ_ONLY=1` override every access grant.

## 9. Verify the setup

With the bot invited and the token loaded:

1. Start read-only with the `$exe` path from step 8:

   ```powershell
   & $exe serve
   ```

2. Call `discord_get_capabilities` and confirm the bot identity and enabled
   classes.
3. Call `discord_list_guilds`; the target server must be present.
4. Call `discord_list_channels`; expected channels must be visible.
5. Call `discord_get_messages` on one allowed channel.
6. For a mutation, call it with `dry_run: true` first and inspect Discord after
   the real call.

**Expected outcomes:**

- `discord_get_capabilities` returns the bot, limits, and only the classes you
  enabled.
- `discord_list_guilds` lists the server; an empty result means the bot was not
  invited or the token is wrong.
- `discord_list_channels` lists only channels the bot can see.
- Message reads return content when the Message Content intent is on.
- A dry run reports a plan and changes nothing.
- A real mutation succeeds only inside the allowlist and budget.

## Quick exception table

| Result | Likely cause | What to do |
|---|---|---|
| `AUTHENTICATION_FAILED` | Wrong or revoked bot token | Reset the token and update the environment |
| `content: null` everywhere | Message Content intent off | Enable it and confirm Save Changes |
| Member tools denied | Server Members intent off | Enable the privileged intent |
| `PERMISSION_DENIED` / `50013` | Missing Discord permission or role hierarchy | Grant the permission and move the bot's role higher |
| Channel absent from `list_channels` | Missing View Channels or explicit deny | Fix the role or channel overwrite |
| Empty message page | Missing Read Message History | Check the channel permission separately |
| `POLICY_BLOCKED` | Local class, allowlist, protected ID, budget, or dry-run rule | Adjust local configuration; do not bypass it |
| Pin/unpin `403` | Missing `PIN_MESSAGES` | Grant Pin Messages |
| Role edit/delete `403` | Missing Manage Roles, or existing target role at/above the bot | Grant Manage Roles or lower the target role |
| `RESOURCE_NOT_FOUND` / `404` | Wrong ID or invisible channel | Re-read the channel and guild IDs |
| `429` / rate limited | Discord rate limit | Let the server wait for `Retry-After`; do not replay mutations blindly |
| Bot shows offline | REST-only server, no gateway | Expected; tools can still work |
