# 1440p60 qualification â€” 2026-11-14

Implementation worktree: `t3/fix-1440p-startup-stall-1`, starting at master `e69cf8d`.
The older `t3/fix-1440p-startup-stall` branch was reviewed, not merged wholesale.
Its server upload/NetEq changes are already on master and were not modified.

## Method / gates

- One optimized `server` profile, `C:/Users/dwhee/.cargo-target/sticktalk-q1440`,
  `CARGO_INCREMENTAL=0`. No dev/release dual tree or profile overrides.
- Real loopback SFU, hardware H.264 encoder and two independent hardware-decoder
  processes. FPS is decoder output, not renderer presentation. Drift is change
  in receive/RTP offset, not glass-to-glass latency.
- The existing **eight-second warmup is unchanged**. Every measured one-second
  bin, including bin zero, is retained and gated. No first-bin trimming.
- Ordered final schedule: synthetic 1080p60/180 s (avg >=58, min >=55,
  abs(drift)<5 ms); five synthetic 1440p60/60 s startups; synthetic 1440p60/600 s;
  owned moving-WGC 1440p60/180 s and Source/180 s; compiler-loaded startups.
  Other cases gate avg >=58, min >=45, abs(drift)<20 ms, zero reconfigurations.
- Source is the Source quality preset; the owned fixture's dimensions are
  recorded in the sharer log. No personal desktop pixels, audio, browser,
  firewall/UAC prompt or non-loopback listener.
- `qualify.py` and `run-acceptance.ps1` preserve all failed cases, reject reused
  case names, and gate independently of the Rust process exit. Each case stores
  commit, exact source diff and executable SHA-256. A failed final stage stops
  the ordered sequence; diagnostics/baselines are labelled separately.

## Review of the prior attempt

- Its trace instrumentation, bounded fair-drain queue, faster initial
  allocation and pre-dequeue probe admission are selectively ported here. The global 32 Mbps wire cap, strict token buckets, 100 ms
  burst, 250 ms age limit and <=10 ms whole-RTC admission deferral must remain.
- Its encoder TWCC feedback path and downstream ceiling cache are **not imported**
  here. The prior native runs showed drift and compiler-loaded regressions;
  adding another rate-control loop is not justified without measurement.
- Its recovery-PLI timer/subscription refactor is not imported: healthy
  delivery should avoid resets rather than depend on additional recovery work.
- Two encrypted-RTP regression fixtures are ported from the old branch: a 295 KB
  fragmented IDR and sixty simulated seconds of two-viewer media plus probes.
  These are transport tests, never substitutes for native hardware acceptance.

- Publisher original-only backlog accounting was considered and then excluded.
  Fresh baseline traces show genuine publisher oversupply (TWCC ~8 Mbps versus
  14.38 Mbps initial encoder output); padding-only inflation was not established.
  Its extra lifecycle/sequence bookkeeping would therefore be unjustified here.

## Baseline observations (not final acceptance)

The trace-only master baseline retains the original data path. Its 1080p/180 s
run failed the strict minimum: both sharer and B had 54 fps at measured bin 36
(C had 55), with seven capture drops. Its 1440p/60 s measured run passed, but
publisher guards fired three times during warmup (617/607/591 KB, 251–265 ms).
The two viewers ended with 382/631 discarded probes and zero discarded originals.
No fresh SFU guard reset occurred in that one baseline startup. This is not a
claim to have reproduced the historical 11-fps measured first bin in this run.
The prior archived traces separately establish premature SFU guards at 262–355 KB
and only 94–141 ms old; the ported bounded-IDR test covers this deterministic seam.

## Results

See `acceptance-table.md`, `results.json`, per-case complete metric arrays and
`startup-trace-summary.json`. Queue maxima are sampled at 100 ms, not claimed
packet-exact peaks. Raw traces preserve BWE kind (TWCC/REMB), PLI/FIR attribution,
keyframe enqueue/send size, guard resets, allocation, token balance and admission
wait for both viewers during the first 15 seconds.

## Final measured result

**All 14 requested measured cases PASS**, in the required (a)–(e) order on one
unchanged native executable. This includes all five compiler-loaded startups,
not just one loaded sample. `final-acceptance.md` is the compact full table;
`acceptance-table.md` includes baseline rows and `results.json` stores exact numbers.
The ten-minute 1440p run delivered B/C 59.999929/59.999922 fps, minima 59/59,
drift -0.685034/-0.640984 ms and zero reconfigurations. 1080p/180 s delivered
B/C 59.999645/59.999306 fps, minima 59/59 and drift -2.122099/-2.110093 ms.
All five unloaded startups had minima 59/59 and first bins >=59; all five
loaded startups had minima >=59 and first bins >=60. Worst loaded drift was
4.767049 ms. Moving WGC and Source are both actual owned-window captures at
2560x1440; captured-frame counts reject static-repeat substitutes.

All final SFU traces show **zero original-queue resets and zero encrypted cap
drops**, versus baseline 1440p's 382/631 dropped probes. The 32 Mbps cap and
100 ms wire burst remain unchanged; byte guards now use sustainable fair drain
capacity with the independent 250 ms age, 2048-packet and 2 MiB ceilings intact.
Only this SFU policy subset was ported; no adaptive encoder loop, extra recovery
retry timer or publisher backlog rewrite was merged.

### Important remaining limitation

This is **not a stall-free-from-time-zero claim**. Synthetic publisher guards
still fire during the original warmup, at RTP times 0.58–2.917 seconds: local
TWCC initially falls to ~8 Mbps while the encoder starts at 14.38 Mbps before
settling to the downstream ceiling. The old branch's extra encoder feedback
loop had known drift regressions and was not brought back to conceal this.
No final guard event reached the measured interval. The eight-second warmup
was not lengthened, and every first measured bin is present. WGC cases had no
publisher guard events. Full GUI presentation/glass-to-glass/network/device
qualification is outside this native decoder-output matrix.

### Validation / reproduction

All commands use `CARGO_TARGET_DIR=C:/Users/dwhee/.cargo-target/sticktalk-q1440`,
`CARGO_INCREMENTAL=0`, `CMAKE_GENERATOR=Visual Studio 17 2022` and the existing
`server` profile. No profile overrides or second target tree.

```powershell
cargo test --profile server -p sticktalk -p sticktalk-media -p sticktalk-sfu --lib --bins --tests --no-fail-fast -j3 -- --test-threads=1
cargo test --profile server -p sticktalk -p sticktalk-media -p sticktalk-sfu --doc -j3
cargo clippy --profile server -p sticktalk -p sticktalk-media -p sticktalk-sfu --all-targets -j3 -- -D warnings
cargo fmt --all -- --check
python crates/client/bench/q1440-2026-11-14/test_qualify.py
cargo build --profile server -p sticktalk -p sticktalk-media -p sticktalk-sfu --example motion_fixture -j3
python crates/client/bench/q1440-2026-11-14/qualify.py --final
# In a separate hidden process, start load-build.py, then run cases
# final-e-load-1 through final-e-load-5 with --quality 1440 --seconds 60.
# Create the evidence directory's stop-load file afterward and await the build.
```

Rust tests: **257 passed, 0 failed, 14 ignored** (client 173, media 40, SFU 44).
Strict all-target clippy and formatting passed. Six fail-closed Python gate tests
cover first-bin retention, strict 1080 limits, drift, completeness, reconfigs and
actual compiler presence. The load helper touches only a source timestamp
(bytes stay unchanged) and runs a real optimized Cargo build; all loaded cases
record active rustc processes at both start and end, rather than a cached no-op.

Raw traces/build logs, including failed/interrupted attempts, are verified and
archived in `trace-logs.zip`; exact hashes/member paths are in `log-archive.json`.
Per-case metrics, provenance and complete FPS/offset arrays remain directly
inspectable. `trace-summary.py` can also read from the archive after log cleanup.
Target cleanup and final build/doc-test exit status are recorded separately.

Harness-only preflight failure: an empty `git diff | Set-Content` did not create
the clean-tree snapshot file. The native test never launched. The failed runner
log and exit are retained as `harness-empty-diff-failure`; explicit file writing
now handles empty snapshots. This is not a measured media case.


### Final cleanup / scope

The optimized compiler-load build finished successfully after **879.0 s**;
all three doc-test targets passed (zero doctests present). The final format and
six Python gate checks also passed. The maximum observed target size was
**8,420,504,303 bytes (8.42 GB decimal)**, below the 10 GB budget, with only the
`server` profile tree. `cargo clean` exited 0, removed 23,259 files and left the
entire task target absent; see `cleanup.json`. Owned fixture/decoder processes
were verified stopped before cleanup. No push, browser, sound, notification,
UAC or non-loopback listener.

Across all final viewer runs, the span of the recorded **one-second mean**
receive/RTP offset was at most **11.025 ms** (`offset-stability.json`). This is
not a per-frame maximum jitter or glass-to-glass measurement. Runtime trees
match the fully tested/qualified code; subsequent changes are evidence/gate
scripts only. Fresh measured failure, harness preflight failure and interrupted
compiler attempt remain preserved, rather than overwritten or recast as passes.

For a fresh reproduction, copy the runner scripts into a new sibling bench
directory and adjust the example paths above; existing committed case directories
and archives intentionally refuse overwrite. This preserves prior failures.
