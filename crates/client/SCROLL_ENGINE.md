# Smooth scroll engine

`src/scroll` is an input adapter around the native iced `Scrollable`, not a
replacement renderer. The shared `ui::scroller` factory covers messages, channels,
members, settings and forms; the rail and horizontal code blocks use it too.
New picker views should use that factory rather than raw `scrollable(...)`.

- Wheel lines: additive, independently completing cubic ease-out impulses, 140 ms.
  New notches extend the tail without resetting earlier motion. At most 64 impulses
  per axis. Clamp the target to native content bounds.
- Pixel wheel/touchpad: native 1:1 fractional deltas; cancel any wheel tail so OS
  momentum is not filtered or animated a second time. Native thumb/touch dragging.
- PgUp/PgDn: animated 90% viewport. Home/End: direct. Child controls get first
  refusal; otherwise the scroll area under the pointer handles keyboard scrolling.
- Query the window monitor on startup/moves; use its refresh-specific redraw
  deadlines only during motion. Coalesce viewport notifications at redraw, never
  input distance, so fast pixel events do not rebuild the app at input rate.
  No global ticker, animation thread or idle polling. On Windows DX12 prefer
  Mailbox (advertised by wgpu-hal 27) with the existing maximum frame latency 1;
  respect explicit backend/present-mode overrides. FIFO was observed to pace the
  secondary 75 Hz display at the primary 280 Hz cadence. Other backends retain
  their default present mode. Content-only subpixel correction removes iced's
  logical-pixel quantization without translating the clip or thumb.
- Existing production message virtualization remains: binary-search height index,
  visible rows + four rows overscan each side, 256 retained shaped-body trees.
  Loaded messages now always render; skeletons are only for missing history.
  Existing off-thread thumbnail decode and metadata-sized placeholders are retained.

## Reproduce surface benchmark

Build with the default wgpu feature. Use **one** task target directory outside
OneDrive; delete it after validation. No server or real account is used.

```powershell
$env:CARGO_TARGET_DIR = '$env:USERPROFILE/.cargo-target/sticktalk-scroll'
$env:STICKTALK_AGENT_WINDOW = 'secondary'
$env:CARGO_PROFILE_RELEASE_LTO = 'false'
$env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = '16'
cargo build -p sticktalk --release
$exe = "$env:CARGO_TARGET_DIR/release/sticktalk.exe"
foreach ($mode in @('before', 'after')) {
    $benchArgs = @('--mock', '--scroll-bench', "crates/client/scroll-surface-$mode.json",
        '--agent-window-report', "crates/client/scroll-surface-$mode-window.json")
    if ($mode -eq 'before') { $benchArgs += '--baseline' }
    Start-Process -FilePath $exe -ArgumentList $benchArgs -WindowStyle Hidden -Wait
}
python crates/client/tools/scroll_timeline.py crates/client/scroll-surface-before.json crates/client/scroll-surface-after.json crates/client/scroll-surface-timeline.png
```

This is a wheel-input A/B in the same renderer, not a performance claim about
an old binary. `--baseline` disables wheel-line easing, retaining other fixes.
The baseline uses native wheel jumps, FIFO, and unpaced redraw requests; after uses
the shared engine and monitor-paced Mailbox when supported. A fresh 10k-message fixture contains wrapped text, code blocks, images, static
GIF previews and embed-like markdown. Actual full embeds and animated GIFs are
not implemented in this client. The harness feeds iced input events to the real
view/widget/update path, with no SendInput, pointer movement, browser, IO tasks or
network subscriptions. One second warmup, ten seconds measured, one second cooldown, two seconds idle.
The HWND starts hidden and is shown only through `agent_window::show_secondary`
(no activation, never primary); without a secondary, it remains hidden and uses
an offscreen wgpu texture, explicitly labeled in JSON. The window closes itself.

JSON logs **CPU timestamps after `SurfaceTexture::present` returns**, not physical
scanout. Rounded interval multiples estimate missed submission slots; they cannot
prove zero dropped/doubled displayed frames. Displayed-frame acceptance requires
external PresentMon/ETW instrumentation and an owner feel test. Windows PDH provides
mean per-process 3D-engine utilization when available; process CPU is percent of
one logical core (can exceed 100). Idle CPU is for the isolated harness after
rendering stops, not a full connected-client idle certification. Memory includes
the synthetic 10k retained bodies (production body retention is separately bounded).



## Recorded result (75 Hz secondary monitor)

Latest files: `scroll-surface-before.json`, `scroll-surface-after.json`,
`scroll-surface-timeline.png`. Both windows stayed off primary, foreground unchanged;
both benchmarks exited 0. The baseline is the same UI with native wheel/FIFO,
not an untouched old executable.

| Metric | Native/FIFO | Smooth/Mailbox |
|---|---:|---:|
| Present-return interval p50 / p99 / max (ms) | 3.57 / 4.10 / 17.36 | 13.57 / 14.91 / 27.44 |
| Intervals shorter than 0.8x the 13.33 ms monitor interval | 99.96% | 0.41% |
| Estimated missed submission slots | 0%* | 0.14% |
| CPU, % of one logical core | 31.96% | 13.77% |
| Process 3D GPU utilization | 1.59% | 0.35% |
| Post-cooldown idle CPU | 2.34% | 0.00% |

*The baseline submits about 280 frames/sec to a 75 Hz monitor, so its "0% missed"
number emphatically does not mean 0 dropped displayed frames. After matches the
monitor rate rather than maximizing a misleading frame counter. The p99 target
(<16 ms) passed, but **zero misses did not**: one late submission remains. A prior
repeat (`scroll-surface-after-run2.json`) had a 388 ms wake/presentation gap with
3.95% estimated misses; its measured render/present p99 was 4.00 ms. This variability
is retained, not excluded from the evidence. FIFO paced at the monitor rate
(`scroll-surface-after-fifo.json`) had p99 17.26 ms, hence the Mailbox selection.

After retained at most 15 visible/overscan widgets, with 256 shaped-body cache
entries maximum. The synthetic 10k-body process used 142.4 MiB working set and
213.8 MiB private bytes: **the <100 MiB target is not met by this stress fixture**.
Production's separate 1k-body/8 MiB retention bounds were not increased.

Still needed: physical displayed-frame ETW/PresentMon validation, owner's actual
feel comparison, real 144/240 Hz surface runs (controller distance is tested at
60/144/240 Hz), full connected-client idle measurement, and full GIF/embed playback
coverage once those views exist. New picker implementations must use the shared
factory. No claim of "100x" or zero dropped displayed frames is made.

Validation: `cargo test -p sticktalk` (33 tests),
`cargo clippy -p sticktalk --all-targets -- -D warnings`,
`cargo fmt --all -- --check`, `git diff --check`; secondary-window before/after
benchmarks; headless screenshots at 1280x800 and 1920x1080. Review contact sheets
and selected full-resolution renders are in `screenshots/scroll-review/`.
Existing screenshot baselines were restored after QA to avoid timestamp-only PNG
churn and conflicts with the concurrent UI work. No visual restyling was done.
