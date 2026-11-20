# TASKS

Updated: 2026-11-20

Branch: `feat/user-message-search`

Current base: `main`; tag `v0.1.0-moderation.1` remains at `28e0b3e`.

## Current state

- [x] 40 tools and one resource, with read-only defaults.
- [x] Read tools, including attachment-filtered user scans and the abuse-keyword scan.
- [x] Guarded message, channel, thread, invite, role and member-moderation tools.
- [x] Class flags plus allowlists, rolling budgets, dry runs and audit records.
- [x] `--access` session overrides for `read`, `write`, `control`, `moderation`,
  `administrative` and `all`; allowlists are still required.
- [x] 271 tests pass: 28 lib, 57 integration, 43 protocol, 143 unit.
- [x] Formatting, Clippy and release build pass.
- [x] Documentation reconciled with the current tool set.

## Open

- [ ] Live rich send in the test guild: embed, file, reply and reply-ping check.
- [ ] Positive role create/edit/assign/remove/delete below the bot's highest role.
  The test guild's `smoke-above-bot` role is currently above `mcp-test-bot`.
- [ ] Cross-user Manage Messages paths, including clear-reactions and bulk delete.
- [ ] Managed-role delete and permission-edit refusal against a second bot or
  integration role.
- [ ] Commit, push, PR/CI review and release for the current branch.

## Release

- `main` carries the rewritten history; tag `v0.1.0-moderation.1` remains at `28e0b3e`.
- Download release assets and any published SHA-256 from the GitHub release page.
  Verify the hash against that same release; do not compare it with a different
  build.
- The current branch is not released.
