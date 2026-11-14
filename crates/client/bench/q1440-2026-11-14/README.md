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

Final implementation, validation and cleanup status will be appended at handoff.
