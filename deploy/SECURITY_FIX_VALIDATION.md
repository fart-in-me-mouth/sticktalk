# Security fixes — validation (2026-11-13)

Implementation branch: `t3/security-hardening-account-deletion`.
All listeners/tests used loopback. No browser, visible app window, audio playback,
notifications, administrator operations or real Google account were used.

## Findings and regression coverage

| Finding | Fixed behavior | Main regression |
|---|---|---|
| S1 | Whole ACL delta checked against pre-mutation rights; unauthorized removal rejects atomically. | `s1_empty_partial_and_bit_removal_preserve_protected_acl` |
| S2 | Fresh password/Google proof, one operation/session, single use, five-minute expiry. Native/web prompts included; last method cannot be removed. | `s2_stolen_session_cannot_link_unlink_or_create_first_password`; `s2_google_reauth_proofs_expire_and_cannot_cross_sessions_or_operations` |
| S3 | Separate transfer admission; 3 downloads/user, 8/IP; body and actual socket-write deadlines. Caddy mirrors deadlines without imposing the upstream HTTP read timeout on WS upgrades. | `s3_unpolled_slow_body_releases_admission_and_separate_pool_preserves_rest`; `s3_stalled_socket_writer_times_out_without_body_polling` |
| S4 | Both upload paths share accounting; unlimited file default, 3 unfinished/user, 1 h idle expiry, configurable 100 GiB/day actual ingress. | `s4_shared_daily_ingress_no_declared_charge_or_abort_refund`; `s4_three_shared_unfinished_slots_idle_expiry_and_quota`; `s4_zero_means_unlimited_for_multipart_too` |
| S5 | Affected call only; reserved cleanup lane; signaling cannot consume reserved chat queue space or cancel the app. | `s5_signaling_flood_does_not_shutdown_chat` (2,000 signals); `s5_voice_frames_cannot_consume_reserved_chat_capacity`; `s5_full_internal_output_reserves_cleanup_and_preserves_other_sessions` |
| S6 | Atomic password update + all-old-token revocation + current token rotation; shared WS/voice cleanup. | `s6_password_change_rotates_and_revokes_sockets_and_voice` |
| S7 | LUKS/key recovery instructions; owner-recipient encrypted scripts and online snapshots; 14-day retention. | `s7_snapshot_is_ciphertext_and_only_owner_identity_decrypts`; `s7_backups_require_recipient_encrypt_before_sync_and_expire_in_14_days` |
| S8 | Server nonce, short-lived/single-use, verified signed claim. Web callbacks bind operation/session even when Strict cookies are absent on return from Google. | `s8_google_nonce_is_mandatory_single_use_short_lived_and_session_bound`; `web_redirect_pkce_state_cookie_signup_and_existing_session`; core `s8_authorization_requests_the_exact_server_nonce` |
| S9 | Guild-only bytes; host capacity fields undisclosed; host aggregates in local admin CLI only. | `s9_guild_storage_hides_other_guilds_dms_and_host_capacity` |
| S10 | SQLx 0.9.0 / libsqlite3-sys 0.39.1, lru 0.18.5, ringbuf 0.5.3; compatible Tract overflow backport. | `s10_bundled_sqlite_is_security_patched`; `s10_nnef_shape_and_byte_overflows_are_errors_not_panics_or_allocations` |
| Account deletion | Fresh proof, anonymous tombstones, file removal, credential revocation, 30-day username hold; offline admin erasure. Other users retain valid tokens and resync cached data. | `account_deletion_requires_proof_erases_files_and_holds_name_30_days`; `admin_cli_erasure_uses_the_same_tombstone_and_username_hold`; `google_only_account_deletion_accepts_only_fresh_existing_identity_proof` |

## Commands/results

All Cargo commands used the ONE target directory
`C:/Users/dwhee/.cargo-target/sticktalk-secfix`, `CARGO_INCREMENTAL=0`, and the
workspace's unmodified dev profile. No release tree was built.

Passed:

```text
PROPTEST_CASES=64 cargo test -p sticktalk-server -p sticktalk-sfu -p sticktalk-client-core -p sticktalk-proto
cargo clippy -p sticktalk-server -p sticktalk-sfu -p sticktalk-client-core -p sticktalk-proto --all-targets -- -D warnings
cargo fmt --all -- --check
cargo check -p sticktalk -p sticktalk-web -p sticktalk-server -p sticktalk-sfu --all-targets
cargo clippy -p sticktalk-server -p sticktalk-sfu -p sticktalk-client-core -p sticktalk-proto -p sticktalk -p sticktalk-web --all-targets -- -D warnings
cargo check -p sticktalk-web --target wasm32-unknown-unknown --all-targets
cargo test -p sticktalk-server s7_
bash -n deploy/backup.sh deploy/pi/backup.sh deploy/pi/failover.sh
caddy adapt --adapter caddyfile --config deploy/Caddyfile
caddy adapt --adapter caddyfile --config deploy/pi/Caddyfile
git diff --check
```

Main suite: **227 passed, 0 failed, 1 ignored**. The ignored existing test writes
more than 2 GiB per storage tier and is not appropriate for this disk budget.
Property suites used 64 cases, not the much larger default 3,072-case soak.
Targeted vendored Tract test: **1 passed** (temporarily selected as a workspace
member to retain the same profile/target, then restored to excluded dependency).
The native/web all-target checks and clippy additionally cover the required UI
and nonce/proof glue. Actual Google/browser/desktop integration and deployment
commands remain acceptance work; they were not run on the owner's active PC.

Caddy 2.10.2 was adapted only, never started/listened. Pi adapter placeholders
used a test hostname/private IP; no certificate provisioning was attempted.

## Raw audit (not a clean-audit claim)

Portable **cargo-audit 0.23.0**, database revision
`af7b3ec33460b8e2de8d422e16d628518891a519` (2026-11-12):

```text
cargo-audit audit --file Cargo.lock --db C:/Users/dwhee/.cache/sticktalk-secreview-audit/advisory-db --no-fetch --json
exit 1; 941 dependencies; 2 advisories; 6 unmaintained notices
```

Advisories: portaudio 0.8.0 (inactive optional path), rsa 0.9.10 (dev-only).
Normal production dependency inspection excludes both. Notices: anymap2, paste,
rustybuzz, ttf-parser, basic-toml and proc-macro-error2 (the latter two are age
build-time localization helpers). Exact boundaries and the Tract source backport
are documented in `AUDIT_EXCEPTIONS.md`; local path code is not attested by the
registry audit. No global advisory ignores were added.

## Deployment/behavior notes

- Set `STICKTALK_BACKUP_RECIPIENT` on Oracle before enabling its backup timer;
  keep the private age identity on the owner's separate trusted computer.
- Pi off-device scripts require `BACKUP_RECIPIENT` and the LUKS mapper mount.
  Both private-node and legacy Pi-only layouts are supported. Encrypted online
  snapshots now require owner-assisted decryption/review for cold recovery;
  standby no longer silently picks an old plaintext backup.
- Account erasure currently forces other connected clients to resync cached
  content; their credentials are not revoked. Calls tied to those WS connections
  may need rejoining. This favors prompt erasure over a new bulk-erasure event.
- Linux encryption/restore steps are documented, not executed here. Old plaintext
  backups must be removed only after verifying a new encrypted restore.
- SFU changes are limited to S5 queue/cleanup containment; no forwarding/bitrate
  algorithm was changed. Outside the core scope, changes are only requested UI
  glue, additive protocol DTOs/docs and the two NetEQ dependency manifest forms.
- Final handoff removes the task target directory; no build output is committed.
