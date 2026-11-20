# Manual QA checklist

Use this only in a dedicated test guild. The executed live results are in
`docs/LIVE_TESTING.md`; current project status is in `TASKS.md`.

## Setup

1. Create a test guild and a bot with the permissions for the tools being
   tested. Do not grant Administrator.
2. Put `DISCORD_BOT_TOKEN` in the server environment. Enable **Message Content**
   for message text and **Server Members** for member tools.
3. Put the test guild and channel IDs in the config allowlists.
4. Start read-only first:

   ```powershell
   .\discord-mcp.exe serve --config config\local.toml
   ```

5. For controlled write tests, use the matching access class:

   ```powershell
   .\discord-mcp.exe serve --config config\local.toml --access write,control
   ```

6. For HTTP, set `transport = "http"` and repeat the read checks. Confirm a
   missing token returns `401` and a foreign `Origin` returns `403`.

## Test data

- Use one marker for the run, for example `QA-2026-11-20`.
- Include messages from the bot and from a consenting human account.
- Include one attachment and one message containing a link.
- Do not create fake members or fake join dates.
- Clean up only messages and objects created for the run.

## Checks

### Read

- `discord_get_capabilities` shows the expected classes and limits.
- `discord_list_guilds` returns the test guild.
- `discord_list_channels` lists the expected text channels.
- `discord_get_messages` returns newest-first pages and a usable `next_cursor`.
- `discord_get_user_messages` filters by user and by `has_attachments`.
- `discord_find_abusive_messages` returns keyword matches grouped by author.
- A channel without Read Message History may return an empty page; verify its
  permissions separately.

### Write

- With mutations off, send/edit/delete return `POLICY_BLOCKED` and change nothing.
- With `--access write`, send, edit, react and pin one test message.
- With `--access control`, delete a bot-owned message.
- With `--access moderation`, set and clear a consenting member's nickname and
  timeout.
- With `--access administrative`, run a dry-run bulk delete and one role change
  below the bot's highest role.
- Confirm every mutation respects its allowlist, budget and dry-run rules.

### Safety

- A channel outside `channel_allowlist` is refused.
- A protected channel or role is refused.
- Invalid IDs and oversized arguments return `INVALID_ARGUMENT`.
- `@everyone`, `@here`, role and user mentions are neutralised.
- A missing Discord permission returns `PERMISSION_DENIED`, not a false success.
- Logs contain no token, authorization header or message body.

## Finish

1. Delete only the test messages and objects created for the run.
2. Restore any permission changes.
3. Stop the server and restart it read-only.
4. Record the build, tool names, results and any remaining gap.
