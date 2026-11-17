# Dependencies

Decisions in this document were verified against the actual vendored crate
sources and crate metadata on 2026-11-17, not from memory. Every dependency is
pinned to an exact version in `Cargo.toml` and `Cargo.lock` is committed.

## Toolchain

| Item | Value |
|---|---|
| `rustc` / `cargo` | 1.97.1 (stable), Windows MSVC host |
| Host triple | `x86_64-pc-windows-msvc` |
| Installed targets | `x86_64-pc-windows-msvc` |
| Effective `rust-version` | 1.89 (highest MSRV among direct dependencies) |

No WSL, Docker or Linux subsystem is required, and none is used.

## MCP SDK — `rmcp =3.11.0`

Verified from `~/.cargo/registry/src/*/rmcp-3.11.0`.

- MSRV 1.88.
- Default features: `base64`, `macros`, `server`. The `server` feature itself
  enables `transport-async-rw`, `schemars`, `pastey` and `uuid`, so tool schemas
  and JSON Schema derivation come with it.
- Enabled on top of the defaults:
  - `transport-io` — `rmcp::transport::io::stdio()` returns the
    `(tokio::io::Stdin, tokio::io::Stdout)` pair accepted by
    `ServiceExt::serve`.
  - `transport-streamable-http-server` — pulls in
    `transport-streamable-http-server-session`, `server-side-http` and
    `transport-worker`. Exposes `transport::streamable_http_server::
    {StreamableHttpService, StreamableHttpServerConfig, SessionManager}`.
- `StreamableHttpService::new(service_factory, Arc<M: SessionManager>, config)`
  returns a Tower service that can be mounted on a router. `StreamableHttpServerConfig`
  carries `allowed_hosts` (loopback-only by default), `allowed_origins` (empty by
  default means **validation disabled**, so it must be set explicitly),
  `max_request_bytes`, `sse_keep_alive`, `sse_retry`, `legacy_session_mode`,
  `json_response`, a cancellation token and an optional external session store.
- Session semantics: per SEP-2567 the `2026-07-28` protocol version removes
  sessions, so clients negotiating it are served statelessly regardless of
  `legacy_session_mode`. Older clients can still get session-id behaviour. This
  is an rmcp/SDK property we document rather than invent.
- The handshake result is a `#[non_exhaustive]` `InitializeResult`, built with
  `InitializeResult::new(capabilities)`, `.with_server_info(..)`,
  `.with_instructions(..)` and `.with_protocol_version(..)`. rmcp 3.11 exposes it
  under two aliases, `ServerInfo` and `ServerConfig`; `ServerInfo` is deprecated
  as of this version, so the code uses `ServerConfig`. Neither alias may be
  constructed with a struct literal, and `Implementation` (also
  `#[non_exhaustive]`) is built with `Implementation::new(name, version)`.
  Both rules were confirmed by the compiler during Phase 1.

## Discord client — `twilight-http 0.18` (selected, added in Phase 2)

- REST-only client for Discord's HTTP API, hyper-based, from the maintained
  twilight ecosystem. Its own documentation states it "meets Discord's
  ratelimiting requirements and supports proxying".
- Chosen over `serenity`: serenity's `http`-only configuration still drags in a
  model/framework graph sized for gateway bots, and this server subscribes to no
  events in v1. One REST stack, one set of models.
- Chosen over a hand-rolled `reqwest` client: bucket-level rate limiting and
  `Retry-After` handling are exactly what a hand-rolled client gets wrong.
- MSRV 1.89, which is why the crate MSRV is 1.89.

### Mock-server strategy for integration tests (Phase 2 risk, resolved in design)

`twilight-http` has no first-class base-URL override. The plan is:

1. All application code depends on an application-owned `DiscordApi` trait, not
   on twilight types. Domain-level tests use a in-memory fake implementing that
   trait — this is where pagination and policy interaction are tested.
2. HTTP-level behaviour (status mapping, 429 handling, malformed bodies) is
   tested through twilight's proxy support aimed at a local mock server, plus a
   smoke test that the real client is constructed with the configured base URL.
3. If the proxy path turns out to be unusable for a route, the fallback is a
   narrow `reqwest` transport behind the same trait for that route only, and
   this document is updated before that code is written.

### HTTP serving (Phase 5)

`rmcp`'s `server-side-http` feature brings its own HTTP stack for the Tower
service. We will not pin `axum`/`tower` independently in advance; the pin will
match the version rmcp exposes, and it will be recorded here when Phase 5 lands.

## Pinned crate set

| Crate | Version | Role |
|---|---|---|
| `rmcp` | 3.11.0 | MCP protocol, STDIO and Streamable HTTP server transports |
| `tokio` | 1.54.1 | Async runtime (`rt-multi-thread`, `io-std`, `io-util`, `time`, `net`, `signal`, `macros`) |
| `serde` | 1.0.229 | Configuration and protocol deserialization |
| `toml` | 1.2.1 | Configuration parsing |
| `thiserror` | 2.0.21 | Typed error enums (`ConfigError`) |
| `tracing` | 0.1.44 | Structured diagnostics |
| `tracing-subscriber` | 0.3.23 | `fmt` subscriber, `env-filter` for `RUST_LOG` |
| `secrecy` | 0.10.3 | `SecretBox`/`SecretString` with zeroize-on-drop |
| `serde_json` (dev) | 1.0.154 | Parsing protocol frames in the protocol tests |

`secrecy` 0.10 exposes `pub type SecretString = SecretBox<str>` and the
`ExposeSecret` trait, which this crate uses so a token cannot be `Debug`-printed
by accident.

## v1 bot scopes and minimum permissions (Phase 0 decision)

OAuth2 install URL scope: `bot` only. `applications.commands` is deliberately
not requested — this server registers no slash commands.

Minimum permissions the operator grants per guild:

| Capability | Discord permission | Needed for |
|---|---|---|
| See channels | View Channels | every read tool |
| Read history | Read Message History | `discord_get_messages`, `discord_get_message` |
| Send messages | Send Messages | `discord_send_message`, only when writes are enabled |
| Member data | `GUILD_MEMBERS` **privileged intent** (Developer Portal toggle) | `discord_list_members`, `discord_get_member` |

`Administrator` is never required. Discord permissions remain the final
authority: local policy can only narrow what the bot may do, never widen it.

## Discord API v10 surface for v1 read tools

Base: `https://discord.com/api/v10`, bot authentication via
`Authorization: Bot <token>`.

| Tool | Endpoint | Notes |
|---|---|---|
| `discord_list_guilds` | `GET /users/@me/guilds` | Only guilds the bot is a member of |
| `discord_get_guild` | `GET /guilds/{guild.id}` | 404 or 403 when not visible |
| `discord_list_channels` | `GET /guilds/{guild.id}/channels` | Includes the permission-filtered set the bot can see |
| `discord_get_channel` | `GET /channels/{channel.id}` | 403 when the bot cannot view it |
| `discord_get_messages` | `GET /channels/{channel.id}/messages` | `limit` 1–100, `before`/`after`/`around` cursors |
| `discord_get_message` | `GET /channels/{channel.id}/messages/{message.id}` | 404 when absent |
| `discord_list_members` | `GET /guilds/{guild.id}/members` | `limit` 1–1000, requires the **privileged `GUILD_MEMBERS` intent** |
| `discord_get_member` | `GET /guilds/{guild.id}/members/{user.id}` | Usually also needs `GUILD_MEMBERS` |

Write endpoint for the single v1 write tool is `POST /channels/{channel.id}/messages`
(`content` 1–2000 characters, explicit `allowed_mentions`).

The API version is pinned to `v10` in configuration and validation rejects any
other value, so a version change is a deliberate, tested change.
