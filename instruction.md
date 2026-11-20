# Automated setup for an LLM

Read this file before changing or running the project. It is the setup runbook
for `discord-mcp` on Windows 11 x64. Work from the repository root. The server
uses an official Discord bot token, never a user token.

Discord portal steps — creating the application, inviting the bot, and granting
permissions — require the account owner. Do not pretend they are complete. Do
not print the bot token or the contents of `.env`.

## 1. Check prerequisites

- Windows 11 x64.
- Rust with the MSVC target and linker.
- Git, PowerShell, and a shell such as Git Bash.
- No WSL, Docker, or Node.js is required for the server.

```powershell
cargo --version
rustc --version
```

If the target is missing:

```powershell
rustup target add x86_64-pc-windows-msvc
```

## 2. Build and verify

For a normal release build:

```powershell
cargo build --release --locked
```

That produces `target\release\discord-mcp.exe`.

For the reproducible release build used for published checksums:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build-release.ps1
```

That produces
`target\x86_64-pc-windows-msvc\release\discord-mcp.exe` and prints its SHA-256.

After either build, set the executable path once for the rest of the session.
Use the assignment that matches the build you ran:

```powershell
# After cargo build --release:
$exe = ".\target\release\discord-mcp.exe"

# After scripts\build-release.ps1:
# $exe = ".\target\x86_64-pc-windows-msvc\release\discord-mcp.exe"

if (-not (Test-Path $exe)) { throw "Build artifact not found: $exe" }
```

Verify the executable and the built-in configuration:

```powershell
& $exe --version
& $exe --validate-config
```

The default configuration is read-only. A missing-token warning is expected
until step 3.

## 3. Provide the Discord token

The server reads `DISCORD_BOT_TOKEN` from the environment. It does **not** read
`.env` itself.

1. Check whether `.env` exists. If it does not, create it from the example:

   ```powershell
   Copy-Item .env.example .env
   ```

2. Ask the user to put the bot token in `.env`, or to provide it through the MCP
   host's `env` block. Never print, echo, log, or commit the token.

3. If `DISCORD_BOT_TOKEN` is missing, stop and ask the user. Do not use a user
   token, a placeholder, or a guessed value.

4. Load the token into the current shell or the MCP host environment. For a
   local PowerShell session, this reads only the token variable without printing
   the file:

   ```powershell
   Get-Content .env |
     Where-Object { $_ -match '^DISCORD_BOT_TOKEN=' } |
     ForEach-Object { $env:DISCORD_BOT_TOKEN = ($_ -split '=', 2)[1] }
   ```

Do not run `Get-Content .env` or any command that displays the token.

## 4. Complete the Discord account setup

The user must do these steps in the Discord Developer Portal and Discord client:

1. Create or select the bot application.
2. Invite the bot to the test server with least-privilege permissions.
3. Enable **Message Content** if message text is needed.
4. Enable **Server Members** if member tools are needed.
5. Grant only the permissions for the tools being tested. See
   [`docs/SETUP.md`](docs/SETUP.md) §2.4.

If the bot is not invited, the intents are off, or a required permission is
missing, the server may start but the affected tool will fail. Report the
missing Discord-side step rather than changing local policy to work around it.

## 5. Configure allowlists

The default configuration is safe for read-only use. For mutations, copy the
default to a local config and edit the allowlists:

```powershell
Copy-Item config\default.toml config\local.toml
```

Set these in `config\local.toml`:

- `policy.channel_allowlist` for write and control tools;
- `policy.guild_allowlist` for moderation and administrative tools;
- optionally `policy.protected_channels` and `policy.protected_roles`.

A class cannot be enabled without its matching allowlist. Do not add `0`,
placeholder IDs, or IDs from a different server.

Validate the edited file without starting the server:

```powershell
& $exe --validate-config --config config\local.toml
```

## 6. Start the server

Read-only, using built-in defaults:

```powershell
& $exe serve
```

With a local config:

```powershell
& $exe serve --config config\local.toml
```

To enable classes for this session, use `--access`. The allowlists in the config
must already be present:

```powershell
& $exe serve --config config\local.toml --access write
& $exe serve --config config\local.toml --access write,control
```

`--access` accepts `read`, `write`, `control`, `moderation`, `administrative`,
or `all`; it can be repeated or comma-separated. `--read-only` and
`DISCORD_MCP_READ_ONLY=1` override every access grant.

For an MCP host, use the executable path and the same arguments:

```json
{
  "mcpServers": {
    "discord": {
      "command": "C:\\\\path\\\\to\\\\discord-mcp.exe",
      "args": ["serve", "--config", "C:\\\\path\\\\to\\\\config\\\\local.toml"],
      "env": { "DISCORD_BOT_TOKEN": "<set this outside version control>" }
    }
  }
}
```

Add `"--access", "write"` to `args` only when the config already has the
required allowlists and the user has approved mutations.

## 7. Verify the running server

Call `discord_get_capabilities` before anything else. Confirm:

- the bot identity is the expected account;
- `read` is enabled;
- only the intended mutation classes are enabled;
- the action limits and required dry-run tools are what the operator expects.

For a source checkout, run the tests with:

```powershell
cargo test --quiet
```

Do not perform a mutation during setup. After setup, read
[`llm_instruction.md`](llm_instruction.md) for the tool workflow, dry-run rules,
and error handling.

## 8. Report blockers

Stop and report one of these instead of inventing a workaround:

- missing `DISCORD_BOT_TOKEN`;
- bot not invited;
- Message Content or Server Members intent disabled;
- missing Discord permission or role hierarchy;
- missing or invalid allowlist;
- configuration validation failure;
- build failure.

Never print secrets, never commit `.env`, and never enable a mutation class
without the user's explicit request.
