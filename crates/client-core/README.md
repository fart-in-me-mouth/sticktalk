# sticktalk-client-core

Shared behavior for iced desktop and the forthcoming Leptos DOM/PWA client. The
crate has no GUI, Tokio, native sockets/HTTP, keyring, filesystem, threads, OS
clock or `Instant`. Default features are empty. `web` enables browser adapters
only on `wasm32`; all other modules build on either target without an adapter.

## Public surface

- `model::Model`: bounded protocol projection/reducers, snapshot reconciliation,
  message history merging/eviction, unread counts, typing, presence, contacts,
  servers/channels/members, server-projected permissions and mention candidates.
- `state::ClientState`: navigation/session, pending commands/drafts/attachments,
  read state, history windows/cursors and metadata pagination reducers. View
  geometry, measured row slots, editor widgets and widget caches are UI-owned.
- `backend::{Config, Session, Request<R>, Data<H>, HistoryAnchor, AccountAction}`:
  shared DTOs. `R` is a host resource reference (desktop `PathBuf`, browser Blob
  registry key); `H` is the UI's bounded command-queue handle. No OS path type
  leaks into core. Existing `sticktalk::{model, markdown, backend, mock}` APIs
  remain desktop facades for the live harness.
- `protocol::run`: one socket, Identify/resume, applied-cursor tracking, pending
  nonce correlation/retry expiry, heartbeat/ack deadline and jittered recovery.
  REST concurrency remains 2 active/16 queued; pending receipts remain 32.
  Its input items are `(Request<R>, Lease)`; a host byte-budget permit stays
  alive through processing, rather than being released when dequeued.
- `http::Http<A>`: validated origins, REST/auth/account/history routes, JSON/error
  policy and bounded streaming collection. `upload::UploadSource` plus
  `Http::{upload_resumable,resume_upload}` hash, create, query durable offsets,
  stream chunks and finish. UI-specific file pick/save/legacy multipart stay
  in desktop glue.
- `markdown`: parser, mention/link targets, AST and bounded parsed-tree cache.
  `datetime`: formatting of host-localized calendar parts (no timezone database).
- `oauth::{Proof, TokenExchange, exchange}`: injected-entropy PKCE/state, URL and
  callback validation and bounded token exchange. Browser launch, loopback
  listener, build credentials and credential persistence remain desktop-only.
- `mock::{Mock, MockAuth, IMAGE}`: protocol peer, scenarios, REST fixture replies,
  startup events, auth fixtures and shared image. Use `with_clock(Arc<dyn Clock>)`
  interactively; default/new fixtures intentionally use a deterministic clock.

## Host boundaries

| Boundary | Desktop (`sticktalk::platform`) | Browser (`sticktalk_client_core::web`) |
|---|---|---|
| `protocol::Transport` | `Native`: tungstenite, sticktalk.v1, frame bounds | `Browser`: web-sys WebSocket, bounded event queue, teardown |
| `http::HttpTransport` | `NativeHttp`: reqwest streaming | `Fetch`: fetch/readable streams, same-origin cookies, optional CSRF |
| `io::Clock` / `io::Entropy` | `Native`: monotonic/wall time, OS CSPRNG | `Browser`: performance/Date, Web Crypto, cancellable one-shot timers |
| `io::Spawner` | abort-on-drop Tokio tasks | abort-on-drop spawn_local tasks |
| `upload::UploadSource` | `FileSource`: bounded file streams | `BlobSource`: 64 KiB Blob slices |
| `protocol::RestService` | desktop wrapper handles file save/upload | `Http<A>` handles REST; UI wraps it for Blob-registry upload/save |
| `protocol::EventSink` | existing 128-item / 2 MiB delivery queue | supplied by UI; must enforce the same count/byte budget |

Futures/streams require `Send` on native and are local on wasm. Dropping the
protocol future closes its transport and cancels outstanding REST tasks. No
idle animation, polling or additional runtime is introduced. HTTP adapters
must refuse redirects, enforce response limits while streaming and bound time.
Desktop constructs reqwest clients with redirect policy `none`; browser fetch
uses redirect `error`. Fetch buffers at most one 16 MiB upload chunk for Safari/
Chrome compatibility, never an entire file. Browser session/server endpoints
and web-redirect OAuth integration are subsequent B8 work, not claimed here.

## Checks

```powershell
rustup target add wasm32-unknown-unknown
./tools/check-client-core.ps1
cargo check -p sticktalk-client-core --target x86_64-pc-windows-msvc
cargo test -p sticktalk-client-core -p sticktalk
cargo clippy -p sticktalk-client-core -p sticktalk --all-targets -- -D warnings
cargo fmt --all -- --check
```

The wasm check must not select Tokio, reqwest, tungstenite, iced or keyring in
`cargo tree -p sticktalk-client-core --target wasm32-unknown-unknown --features web`.
See `crates/client/CORE_EXTRACTION_REPORT.md` for parity and resource evidence.
