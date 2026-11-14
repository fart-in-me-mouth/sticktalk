# Calls QA evidence

See `../../../../docs/QA_CALLS_REPORT.md` for the feature/status table and **known
1440p acceptance failure**. Do not infer success from an outer process returning
zero: `summarize.py` independently checks all three endpoints' metric gates.

`final-synthetic-*` are the two pre-merge 600-second cases. The 1440p native test
failed (viewer C minimum 11 fps), despite an interrupted-run collector recording
zero. Original files and this discrepancy are retained. Short `diagnostic-*` and
`smoke-*` runs are **not** substitutes for full acceptance.

`run-acceptance.ps1` uses only the one already-built optimized target and checks
metrics independently. `run-matrix.ps1` describes the requested 19-case matrix;
only its first two cases completed before the lead's shortened deadline. No new
ten-minute run was started after merging master. `test_summary.py` exercises the
fail-closed report gates (including a zero exit status plus an 11-fps result).

`trace-logs.zip` retains raw media diagnostics, hardware/control/voice probes,
failed suite attempts and final validation logs. Device endpoint names are
excluded. `web-evidence/` records a separate fake-device browser-browser run
against the native server, using another agent's prebuilt web artifact. Its exact
local test script is archived under `web-probe/calls.mjs`; adapt the Playwright
import path to rerun it. It does not qualify a mixed desktop/browser GUI call.

`render-calls.ps1` renders iced views offscreen with tiny-skia, never opening an
OS window. `review-sheets.py` generates contact sheets for reviewing every PNG.
Final renders and binary/hash provenance live in `../../screenshots/qa-calls/`.

All network listeners were loopback; moving WGC and real input tests used owned
secondary-monitor fixtures. No private desktop pixels or microphone recordings
were saved. Build artifacts use `C:/Users/dwhee/.cargo-target/sticktalk-qa-calls`
only and are removed after final validation.
