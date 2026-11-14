# Final acceptance (two viewers)

All requested measured gates passed. Eight-second warmup unchanged; all first bins retained.

| Stage | Seconds | B avg/min/first fps | C avg/min/first fps | B/C drift ms | Reconfigs |
|---|---:|---:|---:|---:|---:|
| final-a-1080 | 180 | 60.000/59/61 | 59.999/59/61 | -2.122/-2.110 | 0 |
| final-b-startup-1 | 60 | 59.997/59/61 | 59.997/59/60 | -3.752/-3.691 | 0 |
| final-b-startup-2 | 60 | 59.995/59/60 | 59.995/59/60 | -4.305/-4.400 | 0 |
| final-b-startup-3 | 60 | 59.999/59/59 | 60.000/59/60 | -3.913/-3.915 | 0 |
| final-b-startup-4 | 60 | 59.999/59/60 | 59.999/59/60 | -3.133/-3.071 | 0 |
| final-b-startup-5 | 60 | 59.998/59/60 | 59.999/59/60 | -4.133/-4.177 | 0 |
| final-c-1440-long | 600 | 60.000/59/61 | 60.000/59/61 | -0.685/-0.641 | 0 |
| final-d-wgc-1440 | 180 | 60.004/59/60 | 60.004/59/60 | -0.730/-0.767 | 0 |
| final-d-wgc-source | 180 | 59.999/59/60 | 59.999/59/60 | -2.016/-2.053 | 0 |
| final-e-load-1 | 60 | 59.999/59/61 | 59.999/59/61 | -1.486/-1.350 | 0 |
| final-e-load-2 | 60 | 59.997/59/60 | 59.996/60/60 | -4.754/-4.767 | 0 |
| final-e-load-3 | 60 | 60.017/59/60 | 60.000/59/60 | -2.721/-3.052 | 0 |
| final-e-load-4 | 60 | 59.997/59/60 | 59.995/59/60 | -4.040/-4.412 | 0 |
| final-e-load-5 | 60 | 59.998/59/61 | 59.995/59/60 | -4.224/-4.303 | 0 |

## Baselines / failures (retained)

| Case | B avg/min/first fps | C avg/min/first fps | B/C drift ms | Result |
|---|---:|---:|---:|---|
| baseline-a-1080 | 59.960/54/60 | 59.959/55/60 | -3.045/-2.923 | FAIL |
| baseline-b-1440 | 60.016/58/59 | 60.016/58/59 | -2.740/-2.730 | PASS |

The 1080 baseline dip also occurred at capture (bin 36: sharer 54, seven capture drops).
A clean-tree harness preflight failed before launching native workers; its log/exit are retained.

**Limitation:** synthetic publisher guards still fired during the existing warmup,
latest at RTP time 2.917 s; no stall-free-from-time-zero claim. Every final case had
zero SFU original-queue resets and zero encrypted cap drops. No first measured bin was removed.

All final cases use executable SHA-256 `4155C67A0C2E6F13F453BBFB48A38BDE8894AAAF26348DF9CFBF16E59341F3AB`
and an empty runtime source diff. Load cases have active compilers recorded at start and end.
