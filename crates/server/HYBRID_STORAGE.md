# Hybrid blob storage implementation notes

## Boundaries

- Public upload/download DTOs, authorization, resumable offsets, quota admission,
  disk guard, cancellation guards, MIME checks and Range/ETag handling remain in
  the existing chat server. No proto changes.
- `BlobStore` is an object-safe streaming interface; `LocalDisk` and `RemoteStore`
  implement it. File I/O uses 64 KiB buffers; remote chunks are at most 16 MiB.
- The same binary's `storage --listen ADDRESS --data DIRECTORY` mode constructs
  only the private blob service, not the chat database, SFU or browser app.
  It accepts explicit loopback or Tailscale IPv4 addresses, never wildcard binds.
- HMAC-SHA256 covers method, raw path, time, nonce, offset, length, SHA-256 and
  Range. Compare is constant-time. A ±60-second timestamp window and persisted
  nonce table reject replay across restart. The nonce table is capped at 32,768;
  four request/stream permits bound node concurrency. Secrets are debug-redacted.
- Node PUT persists rollback before truncating a retry, streams/verifies the
  chunk, fsyncs it, then persists the new durable offset with SQLite FULL sync.
  Finalize verifies the full hash before marking an immutable blob complete.
- The main DB tracks acknowledged replicas. Only verified replicas can be
  evicted. Defaults are 100 GiB total hot cache and 25 MiB per ordinary file;
  avatars/icons and thumbnails are eligible regardless of that per-file limit.
  The total LRU cap still applies to all hot files. Outage spools are not cache.
- A mutation-driven worker drains bounded batches, retries pending failures
  after 60 seconds, and has no idle timer/polling. It also drives bounded expired
  tombstone reclamation so a full node can recover while uploads are pending.
- Cancelled incomplete objects are deleted, not retained as backups. Deleted
  completed objects stay 15 days for database snapshot restoration. Chat access
  is revoked immediately by the existing permissions/metadata checks. An
  authenticated stat revives a completed replica referenced after DB restoration.

## Deliberate remaining requirement gap

Ingestion is **not spool-free direct-to-Pi**. Each durable resumable prefix is
mirrored after its local chunk commit; Oracle keeps the complete local spool
until Pi verifies publication. Multipart uploads mirror after local validation.
This retains the existing full-file hash/media preparation, disk guard and
mid-upload outage/cancellation guarantees, but large concurrent uploads still
require their full temporary size on Oracle. Published large files are evicted
after Pi acknowledgment. A truly remote-only ingestion/preparation path remains
future work, not something this implementation claims to provide.

## Backups and restore

`admin backup` uses SQLite `VACUUM INTO`, checks the snapshot, ensures every
referenced blob is on Pi, then publishes the snapshot through the same streaming
protocol. Incomplete snapshots cannot be selected by failover. Only successful
publication prunes snapshots older than 14 days. Incomplete backup transfers
older than a day are eligible for bounded cleanup. Snapshot files live in a
separate `backups/` directory, outside the full server's orphan-file cleanup.

The shipped nightly systemd timer and fenced, one-command Pi failover are
documented in `deploy/README.md`, `deploy/HYBRID.md` and `deploy/pi/README.md`.
Failover remains private and requires Oracle to be stopped/fenced first.
There are no CPU-burning keepalives or automatic split-brain failover.

## Validation: Windows x86-64, unoptimized test profile

- Two real in-process HTTP servers, all listeners on `127.0.0.1`.
- Interrupted node PUT and GET, node restart, durable-offset retry, main-server
  offline fallback, cache eviction, proxied Range retry, snapshot integrity,
  retention, bad MAC/payload, expired signature, persisted replay rejection and
  traversal rejection. Existing server tests remain covered.
- Opt-in stream test uploaded **2,147,549,184 bytes** (2 GiB + 64 KiB), finalized
  and downloaded the whole Pi-only object, verifying SHA-256 end to end.
- OS peak working set/RSS for the **combined main + node + test client process**:
  **36,438,016 bytes = 34.75 MiB**. Baseline and all four 512-MiB progress
  checkpoints had the same high-water mark; no size-dependent RSS growth.
- Measured transfer/verification section: **1,228.53 seconds**; entire debug test
  including expected-digest preparation: **1,383.93 seconds**. These are not
  release throughput numbers or physical Pi measurements.

Commands (one target directory for all builds):

```text
cargo test -p sticktalk-server
cargo test -p sticktalk-server hybrid_multigb_stream_flat_rss --lib -- --ignored --nocapture --test-threads=1
cargo clippy -p sticktalk-server --all-targets -- -D warnings
cargo fmt --all -- --check
bash -n deploy/pi/failover.sh
```

The Linux systemd units, actual Oracle/Pi/Tailscale deployment, HDD durability,
physical ARM RSS/throughput and real failover/failback require owner-equipment
rehearsal. No visible windows, browser, notifications, public test listeners,
administrative installers or security prompts were used for validation.
