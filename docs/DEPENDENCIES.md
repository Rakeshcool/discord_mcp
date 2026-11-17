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

### Mock-server strategy for integration tests (resolved)

`twilight-http` has no base-URL override, but `ClientBuilder::proxy(host,
use_http)` — verified in `twilight-http-0.18.0/src/client/mod.rs` — rewrites every
request to `{scheme}://{host}/api/v{API_VERSION}/{path}`. That is the hook
twilight documents for running through an HTTP proxy, and it is what the adapter
uses, so no route needs a fallback transport:

- All application code depends on the application-owned `DiscordApi` trait in
  `src/discord/client.rs`; twilight types never leave `src/discord/adapter.rs`.
- The adapter derives the proxy authority from `discord.api_base`.
  `https://discord.com/api` is used directly; any other base (a loopback mock, or
  an operator's explicit proxy) goes through `proxy(authority, use_http)`.
- `discord.api_base` must end in `/api`, and plaintext `http` is refused for any
  non-loopback authority, so a misconfigured base cannot silently strip TLS to a
  remote host.
- `tests/integration/mock_discord.rs` is a hand-rolled tokio `TcpListener` that
  answers with canned JSON, records the requests it saw and can delay or fail on
  demand. No extra test-server dependency, no `reqwest` fallback: the tests drive
  the shipped client over a real socket.
- 429 handling (including `Retry-After`) belongs to twilight's ratelimiter and is
  covered through the same mock. The adapter's own retry loop handles only
  transient failures — network errors, timeouts and 5xx — bounded by
  `discord.max_retries` with jittered backoff, and every attempt is wrapped in the
  configured `request_timeout_seconds`, so a rate-limit wait cannot exceed it.
- Page sizes are clamped to Discord's documented maxima before any request is
  built (guilds 200, messages 100, members 1000); an out-of-range limit is an
  `INVALID_ARGUMENT`, not a `400` from Discord. A full page yields `next_cursor`
  from the last id, so callers paginate by passing it back as the cursor.

### TLS crypto provider — `rustls 0.23.46` with `ring`

`twilight-http` depends on `rustls` with `default-features = false`, so rustls
0.23 cannot pick a process-wide `CryptoProvider` and the first TLS handshake
would panic. The adapter installs `ring`'s provider exactly once
(`install_crypto_provider`, via `std::sync::Once`) and the crate pins the `rustls`
version already in the lock file with that provider enabled. `ring` is chosen over
`aws-lc-rs` because it builds on Windows MSVC without CMake or NASM.

### HTTP serving (Phase 5)

`rmcp`'s `server-side-http` feature brings its own HTTP stack for the Tower
service. We will not pin `axum`/`tower` independently in advance; the pin will
match the version rmcp exposes, and it will be recorded here when Phase 5 lands.

## Pinned crate set

| Crate | Version | Role |
|---|---|---|
| `rmcp` | 3.11.0 | MCP protocol, STDIO and Streamable HTTP server transports |
| `tokio` | 1.54.1 | Async runtime (`rt-multi-thread`, `io-std`, `io-util`, `time`, `net`, `signal`, `macros`, `sync` for the policy concurrency cap) |
| `serde` | 1.0.229 | Configuration and protocol deserialization |
| `toml` | 1.2.1 | Configuration parsing |
| `thiserror` | 2.0.21 | Typed error enums (`ConfigError`) |
| `tracing` | 0.1.44 | Structured diagnostics |
| `tracing-subscriber` | 0.3.23 | `fmt` subscriber, `env-filter` for `RUST_LOG` |
| `secrecy` | 0.10.3 | `SecretBox`/`SecretString` with zeroize-on-drop |
| `twilight-http` | 0.18.0 | Discord REST client (rate limiting, 429 handling, proxy hook) |
| `twilight-model` | 0.18.0 | Discord API models, confined to the adapter layer |
| `async-trait` | 0.1.92 | `dyn`-compatible `DiscordApi` trait |
| `rustls` | 0.23.46 | TLS, `ring` provider selected at startup; no default features |
| `serde_json` | 1.0.154 | Tool result serialization and protocol-frame parsing in tests |
| `schemars` | 1.2.3 | JSON Schema derivation for tool argument types (same version rmcp re-exports) |

`secrecy` 0.10 exposes `pub type SecretString = SecretBox<str>` and the
`ExposeSecret` trait, which this crate uses so a token cannot be `Debug`-printed
by accident.

## MCP tool schemas (Phase 4)

Tool arguments are derived with `schemars 1.2.3`, the version `rmcp` itself
re-exports, so the schema the SDK validates against and the schema the macros
derive are the same implementation. Every argument struct carries
`#[serde(deny_unknown_fields)]`, which schemars renders as
`additionalProperties: false`: an unknown argument is refused instead of ignored.

Discord snowflakes are strings on the wire. The ID newtypes implement `Serialize`
by hand to emit decimal strings, because a 19-digit snowflake does not survive an
`f64` round trip. Schema-level `pattern` refinement is not used: validation is
performed by the ID parsers, which return `INVALID_ARGUMENT` before any request.

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
